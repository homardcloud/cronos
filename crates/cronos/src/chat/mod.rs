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
    let api_key = oauth::login().await?;
    credentials::save_api_key(&paths.config_dir, &api_key)?;
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

    // API key resolution order:
    // 1. OPENAI_API_KEY env var / config.ai.api_key (already resolved by CronosConfig)
    // 2. Stored OAuth credentials from `cronos login`
    let api_key = if !config.ai.api_key.is_empty() {
        config.ai.api_key.clone()
    } else if let Some(stored) = credentials::load_api_key(&paths.config_dir)? {
        stored
    } else {
        anyhow::bail!(
            "No OpenAI API key found.\n\
             Run `cronos login` to authenticate with your ChatGPT account,\n\
             or set OPENAI_API_KEY / add api_key to [ai] in config."
        );
    };

    let model = model_override.unwrap_or(config.ai.model.clone());
    let socket_path = if config.daemon.socket_path.is_empty() {
        paths.socket_file.clone()
    } else {
        PathBuf::from(&config.daemon.socket_path)
    };

    autostart::ensure_daemon(&socket_path).await?;
    autostart::spawn_collector_if_absent();

    repl::run_repl(api_key, model, socket_path).await
}
