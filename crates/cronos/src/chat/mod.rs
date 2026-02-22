mod autostart;
mod credentials;
mod daemon_client;
mod oauth;
mod openai;
mod repl;
mod tools;

use cronos_common::{CronosConfig, CronosPaths};
use std::path::PathBuf;

pub async fn cmd_login() -> anyhow::Result<()> {
    let paths = CronosPaths::resolve()?;
    std::fs::create_dir_all(&paths.config_dir)?;

    let result = oauth::login().await?;

    let creds = match result {
        oauth::LoginResult::ApiKey(key) => credentials::StoredCredentials {
            api_key: Some(key),
            access_token: None,
            refresh_token: None,
            chatgpt_account_id: None,
        },
        oauth::LoginResult::OAuthTokens {
            access_token,
            refresh_token,
            chatgpt_account_id,
        } => credentials::StoredCredentials {
            api_key: None,
            access_token: Some(access_token),
            refresh_token: Some(refresh_token),
            chatgpt_account_id: Some(chatgpt_account_id),
        },
    };

    credentials::save(&paths.config_dir, &creds)?;
    println!("Logged in successfully. Credentials saved.");
    Ok(())
}

pub async fn cmd_logout() -> anyhow::Result<()> {
    let paths = CronosPaths::resolve()?;
    credentials::remove_credentials(&paths.config_dir)?;
    println!("Logged out. Credentials removed.");
    Ok(())
}

pub async fn cmd_chat(model_override: Option<String>) -> anyhow::Result<()> {
    let paths = CronosPaths::resolve()?;
    let config = CronosConfig::load(&paths.config_file)?;

    // Resolve auth: either a standard API key or ChatGPT OAuth tokens
    let auth = if !config.ai.api_key.is_empty() {
        openai::Auth::ApiKey(config.ai.api_key.clone())
    } else if let Some(creds) = credentials::load(&paths.config_dir)? {
        if let Some(ref key) = creds.api_key {
            if !key.is_empty() {
                openai::Auth::ApiKey(key.clone())
            } else {
                build_chatgpt_auth(&creds)?
            }
        } else {
            build_chatgpt_auth(&creds)?
        }
    } else {
        anyhow::bail!(
            "No OpenAI API key found.\n\
             Run `cronos login` to authenticate with your ChatGPT account,\n\
             or set OPENAI_API_KEY / add api_key to [ai] in config."
        );
    };

    // For ChatGPT OAuth, default to a supported Codex model instead of config default
    let model = model_override.unwrap_or_else(|| {
        if matches!(auth, openai::Auth::ChatGpt { .. }) && config.ai.model == "gpt-4o" {
            "gpt-5-codex-mini".to_string()
        } else {
            config.ai.model.clone()
        }
    });
    let socket_path = if config.daemon.socket_path.is_empty() {
        paths.socket_file.clone()
    } else {
        PathBuf::from(&config.daemon.socket_path)
    };

    autostart::ensure_daemon(&socket_path).await?;
    autostart::spawn_collector_if_absent();
    autostart::spawn_appmon_if_absent();

    repl::run_repl(auth, model, socket_path).await
}

fn build_chatgpt_auth(creds: &credentials::StoredCredentials) -> anyhow::Result<openai::Auth> {
    let token = creds
        .access_token
        .as_ref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("No access token in stored credentials. Run `cronos login`."))?;
    let account_id = creds
        .chatgpt_account_id
        .as_ref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("No ChatGPT account ID in stored credentials. Run `cronos login` again."))?;
    Ok(openai::Auth::ChatGpt {
        access_token: token.clone(),
        account_id: account_id.clone(),
    })
}
