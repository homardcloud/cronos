use anyhow::{anyhow, Context, Result};
use rand::Rng;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const ISSUER: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPES: &str = "openid profile email offline_access";

const CALLBACK_PORT: u16 = 1455;

/// Generate a PKCE verifier (64 random bytes, base64url-encoded) and its
/// SHA256 base64url challenge. Matches the Codex CLI implementation.
fn generate_pkce() -> (String, String) {
    let mut rng = rand::rng();
    let bytes: [u8; 64] = rng.random();
    let verifier = base64_url_encode(&bytes);

    let hash = Sha256::digest(verifier.as_bytes());
    let challenge = base64_url_encode(&hash);

    (verifier, challenge)
}

fn base64_url_encode(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_state() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
    hex::encode(bytes)
}

/// Build the authorize URL, matching the exact parameter set from the Codex CLI.
fn build_auth_url(state: &str, challenge: &str) -> String {
    let params: &[(&str, &str)] = &[
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", REDIRECT_URI),
        ("scope", SCOPES),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("state", state),
        ("originator", "codex_cli_rs"),
    ];
    let qs: String = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}/oauth/authorize?{}", ISSUER, qs)
}

/// Bind a local server and wait for the OAuth callback.
/// Returns the authorization code from the callback query string.
async fn run_callback_server(expected_state: &str) -> Result<String> {
    let listener = TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .with_context(|| format!("failed to bind 127.0.0.1:{}", CALLBACK_PORT))?;

    let (mut stream, _) = listener.accept().await.context("accepting connection")?;

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.context("reading request")?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse the first line: GET /auth/callback?code=...&state=... HTTP/1.1
    let first_line = request.lines().next().unwrap_or_default();
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow!("malformed HTTP request"))?;

    let query = path
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or_default();

    let params: Vec<(String, String)> = query
        .split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            let v = urlencoding::decode(v).unwrap_or(std::borrow::Cow::Borrowed(v));
            Some((k.to_string(), v.into_owned()))
        })
        .collect();

    let state = params.iter().find(|(k, _)| k == "state").map(|(_, v)| v.as_str());
    let code = params.iter().find(|(k, _)| k == "code").map(|(_, v)| v.clone());

    if state != Some(expected_state) {
        let response = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\n\r\n<html><body><h1>State mismatch</h1><p>OAuth state parameter mismatch. Please try again.</p></body></html>";
        let _ = stream.write_all(response.as_bytes()).await;
        return Err(anyhow!("OAuth state mismatch"));
    }

    let code = code.ok_or_else(|| anyhow!("no authorization code in callback"))?;

    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h1>Success!</h1><p>You can close this tab and return to the terminal.</p></body></html>";
    let _ = stream.write_all(response.as_bytes()).await;

    Ok(code)
}

/// Tokens returned from the authorization code exchange.
pub struct ExchangedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
}

#[derive(serde::Deserialize)]
struct TokenResponseRaw {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    id_token: String,
}

/// Exchange the authorization code for tokens.
async fn exchange_code(code: &str, verifier: &str) -> Result<ExchangedTokens> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/oauth/token", ISSUER))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
            urlencoding::encode(code),
            urlencoding::encode(REDIRECT_URI),
            urlencoding::encode(CLIENT_ID),
            urlencoding::encode(verifier),
        ))
        .send()
        .await
        .context("sending token exchange request")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("token exchange failed ({}): {}", status, body));
    }

    let raw: TokenResponseRaw = resp.json().await.context("parsing token response")?;
    Ok(ExchangedTokens {
        access_token: raw.access_token,
        refresh_token: raw.refresh_token,
        id_token: raw.id_token,
    })
}

/// Decode JWT payload and extract claims from `https://api.openai.com/auth`.
fn jwt_auth_claims(jwt: &str) -> serde_json::Map<String, serde_json::Value> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    let empty = serde_json::Map::new();
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return empty;
    }

    // JWT base64url payload may need padding
    let padded = match parts[1].len() % 4 {
        2 => format!("{}==", parts[1]),
        3 => format!("{}=", parts[1]),
        _ => parts[1].to_string(),
    };

    let bytes = match URL_SAFE_NO_PAD.decode(&padded) {
        Ok(b) => b,
        Err(_) => return empty,
    };

    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return empty,
    };

    value
        .get("https://api.openai.com/auth")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default()
}

#[derive(serde::Deserialize)]
struct ApiKeyResponse {
    access_token: String,
}

/// Exchange the id_token for an OpenAI API key via token exchange.
async fn obtain_api_key(id_token: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/oauth/token", ISSUER))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type={}&client_id={}&requested_token={}&subject_token={}&subject_token_type={}",
            urlencoding::encode("urn:ietf:params:oauth:grant-type:token-exchange"),
            urlencoding::encode(CLIENT_ID),
            urlencoding::encode("openai-api-key"),
            urlencoding::encode(id_token),
            urlencoding::encode("urn:ietf:params:oauth:token-type:id_token"),
        ))
        .send()
        .await
        .context("sending API key exchange request")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("API key exchange failed ({}): {}", status, body));
    }

    let parsed: ApiKeyResponse = resp
        .json()
        .await
        .context("parsing API key response")?;

    Ok(parsed.access_token)
}

/// Result of the login flow — either an API key or OAuth tokens for direct use.
pub enum LoginResult {
    /// Full API key obtained (user has a platform org).
    ApiKey(String),
    /// No platform org — use access_token directly (ChatGPT Plus/Pro).
    OAuthTokens {
        access_token: String,
        refresh_token: String,
    },
}

/// Run the full OAuth PKCE login flow. Opens the browser, waits for callback,
/// exchanges tokens, and attempts to obtain an API key.
pub async fn login() -> Result<LoginResult> {
    let (verifier, challenge) = generate_pkce();
    let state = generate_state();
    let url = build_auth_url(&state, &challenge);

    println!("Opening browser for authentication...");
    open::that(&url).context("failed to open browser")?;
    println!("Waiting for OAuth callback on 127.0.0.1:{}...", CALLBACK_PORT);

    let code = run_callback_server(&state).await?;

    println!("Exchanging authorization code for tokens...");
    let tokens = exchange_code(&code, &verifier).await?;

    if tokens.id_token.is_empty() {
        return Err(anyhow!("no id_token received from token exchange"));
    }

    // Check if the id_token contains an organization_id
    let claims = jwt_auth_claims(&tokens.id_token);
    let has_org = claims
        .get("organization_id")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    if has_org {
        println!("Obtaining API key...");
        match obtain_api_key(&tokens.id_token).await {
            Ok(api_key) => return Ok(LoginResult::ApiKey(api_key)),
            Err(e) => {
                eprintln!("Warning: API key exchange failed ({}), falling back to OAuth tokens.", e);
            }
        }
    } else {
        eprintln!(
            "Note: Your account does not have an OpenAI API platform organization.\n\
             Using OAuth access token directly (works with ChatGPT Plus/Pro)."
        );
    }

    // Fall back to using the access_token directly (like Codex CLI does)
    Ok(LoginResult::OAuthTokens {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    })
}
