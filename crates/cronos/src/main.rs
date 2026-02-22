mod chat;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cronos_common::{CronosConfig, CronosPaths};
use cronos_core::engine::Engine;
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "cronos", about = "Developer activity tracker", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the daemon in the foreground
    #[command(hide = true)]
    Daemon,

    /// Log in with your ChatGPT account (OAuth)
    Login,

    /// Log out and remove stored credentials
    Logout,
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Daemon) => cmd_daemon().await,
        Some(Commands::Login) => chat::cmd_login().await,
        Some(Commands::Logout) => chat::cmd_logout().await,
        None => {
            let paths = CronosPaths::resolve()?;
            std::fs::create_dir_all(&paths.config_dir)?;
            let first_run = cronos_common::consent::check_consent(&paths.config_dir)?;

            if first_run {
                if cronos_common::consent::prompt_openai_connect()? {
                    chat::cmd_login().await?;
                }
                if cronos_common::consent::prompt_desktop_install()? {
                    install_desktop_app()?;
                }
            }
            chat::cmd_chat(None).await
        }
    }
}

// ---------------------------------------------------------------------------
// Desktop app installation
// ---------------------------------------------------------------------------

fn install_desktop_app() -> Result<()> {
    let ui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../cronos-ui");
    if !ui_dir.join("Cargo.toml").exists() {
        println!("Desktop app source not found at {}. Skipping.", ui_dir.display());
        return Ok(());
    }

    // Ensure cargo-tauri CLI is available
    let has_tauri = std::process::Command::new("cargo")
        .args(["tauri", "--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !has_tauri {
        println!("Installing Tauri CLI...");
        let status = std::process::Command::new("cargo")
            .args(["install", "tauri-cli", "--locked"])
            .status()
            .context("failed to install tauri-cli")?;
        if !status.success() {
            println!("Failed to install Tauri CLI. Skipping desktop app.");
            return Ok(());
        }
    }

    println!("Building Cronos desktop app (this may take a few minutes)...");
    let status = std::process::Command::new("cargo")
        .args(["tauri", "build", "--bundles", "app"])
        .current_dir(&ui_dir)
        .status()
        .context("failed to run cargo tauri build")?;

    if !status.success() {
        println!("Desktop app build failed. You can try again later with:");
        println!("  cd {} && cargo tauri build --bundles app", ui_dir.display());
        return Ok(());
    }

    // Find the .app bundle in the workspace target directory
    let workspace_dir = ui_dir.join("../..");
    let app_bundle = workspace_dir.join("target/release/bundle/macos/Cronos.app");

    if !app_bundle.exists() {
        println!("Build succeeded but could not find Cronos.app bundle. Skipping install.");
        return Ok(());
    }

    let dest = PathBuf::from("/Applications/Cronos.app");

    // Remove old version if present
    if dest.exists() {
        std::fs::remove_dir_all(&dest).context("removing old Cronos.app")?;
    }

    // Copy the .app bundle to /Applications
    let status = std::process::Command::new("cp")
        .args(["-R"])
        .arg(&app_bundle)
        .arg(&dest)
        .status()
        .context("copying Cronos.app to /Applications")?;

    if status.success() {
        println!("Cronos desktop app installed to /Applications/Cronos.app!");
        // Open the app
        let _ = std::process::Command::new("open")
            .arg(&dest)
            .status();
    } else {
        println!("Could not copy to /Applications. You can manually move:");
        println!("  {} -> /Applications/", app_bundle.display());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Daemon command
// ---------------------------------------------------------------------------

async fn cmd_daemon() -> Result<()> {
    let paths = CronosPaths::resolve().context("resolving paths")?;
    let config = CronosConfig::load(&paths.config_file).context("loading config")?;

    cronos_common::init_tracing(&config.daemon.log_level);

    let db_path = resolve_db_path(&config, &paths);
    let socket_path = resolve_socket_path(&config, &paths);

    tracing::info!(db = %db_path.display(), socket = %socket_path.display(), "starting daemon");

    let engine = Engine::open(&db_path, &config.daemon).context("opening engine")?;
    let engine = Arc::new(engine);

    // Spawn background session aggregator
    cronos_core::aggregator::spawn_aggregator(
        Arc::clone(&engine),
        config.daemon.aggregator.interval_secs,
        config.daemon.aggregator.session_gap_ms,
    );

    let engine_ref = Arc::clone(&engine);
    let server_handle = tokio::spawn(async move {
        cronos_core::server::run(engine_ref, &socket_path).await
    });

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    server_handle.abort();
    let _ = server_handle.await;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers: path resolution
// ---------------------------------------------------------------------------

fn resolve_db_path(config: &CronosConfig, paths: &CronosPaths) -> PathBuf {
    if config.daemon.db_path.is_empty() {
        paths.db_file.clone()
    } else {
        PathBuf::from(&config.daemon.db_path)
    }
}

fn resolve_socket_path(config: &CronosConfig, paths: &CronosPaths) -> PathBuf {
    if config.daemon.socket_path.is_empty() {
        paths.socket_file.clone()
    } else {
        PathBuf::from(&config.daemon.socket_path)
    }
}
