mod autostart;
mod daemon_client;
mod openai;
mod repl;
mod tools;

use cronos_common::{CronosConfig, CronosPaths};
use std::path::PathBuf;

pub async fn cmd_login() -> anyhow::Result<()> {
    anyhow::bail!("not yet implemented")
}

pub async fn cmd_logout() -> anyhow::Result<()> {
    anyhow::bail!("not yet implemented")
}

pub async fn cmd_chat(model_override: Option<String>) -> anyhow::Result<()> {
    let paths = CronosPaths::resolve()?;
    let config = CronosConfig::load(&paths.config_file)?;

    let api_key = config.ai.api_key.clone();
    if api_key.is_empty() {
        anyhow::bail!(
            "No OpenAI API key found.\nSet OPENAI_API_KEY or add api_key to [ai] in config."
        );
    }

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
