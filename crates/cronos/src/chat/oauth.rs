use anyhow::{anyhow, Context, Result};
use rand::Rng;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const ISSUER: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REDIRECT_URI: &str = "http://127.0.0.1:1455/auth/callback";
const SCOPES: &str = "openid profile email offline_access";

const CALLBACK_PORT: u16 = 1455;

/// Generate a PKCE verifier (43 random chars) and its SHA256 base64url challenge.
fn generate_pkce() -> (String, String) {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::rng();
    let verifier: String = (0..43)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();

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

fn build_auth_url(state: &str, challenge: &str) -> String {
    format!(
        "{}/oauth/authorize?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        ISSUER,
        CLIENT_ID,
        urlencoding::encode(REDIRECT_URI),
        urlencoding::encode(SCOPES),
        state,
        challenge,
    )
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
            Some((k.to_string(), v.to_string()))
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

#[derive(serde::Deserialize)]
struct TokenResponse {
    #[serde(default)]
    #[allow(dead_code)]
    access_token: String,
    #[serde(default)]
    id_token: String,
}

/// Exchange the authorization code for tokens.
async fn exchange_code(code: &str, verifier: &str) -> Result<TokenResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/oauth/token", ISSUER))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .context("sending token exchange request")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("token exchange failed ({}): {}", status, body));
    }

    resp.json::<TokenResponse>()
        .await
        .context("parsing token response")
}

#[derive(serde::Deserialize)]
struct ApiKeyResponse {
    access_token: String,
}

/// Exchange the id_token for an OpenAI API key via RFC 8693 token exchange.
async fn obtain_api_key(id_token: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/oauth/token", ISSUER))
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:token-exchange"),
            ("client_id", CLIENT_ID),
            ("requested_token_type", "urn:ietf:params:oauth:token-type:access_token"),
            ("subject_token", id_token),
            ("subject_token_type", "urn:ietf:params:oauth:token-type:id_token"),
            ("audience", "https://api.openai.com/v1"),
        ])
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

/// Run the full OAuth PKCE login flow. Opens the browser, waits for callback,
/// exchanges tokens, and returns the API key.
pub async fn login() -> Result<String> {
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

    println!("Obtaining API key...");
    let api_key = obtain_api_key(&tokens.id_token).await?;

    Ok(api_key)
}
