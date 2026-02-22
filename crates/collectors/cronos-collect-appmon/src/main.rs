use anyhow::{Context, Result};
use cronos_common::{CronosConfig, CronosPaths};
use cronos_model::*;
use cronos_proto::{read_frame, write_frame, Message, MessageKind, PROTOCOL_VERSION};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::net::UnixStream;
use tracing::{debug, error, info, warn};

/// Get the currently focused application name and its front window title via osascript.
///
/// Returns `None` if the query fails (e.g. no window focused, permission denied).
fn get_active_window() -> Option<(String, String)> {
    let app_name = Command::new("osascript")
        .args([
            "-e",
            "tell application \"System Events\" to get name of first application process whose frontmost is true",
        ])
        .output()
        .ok()?;

    if !app_name.status.success() {
        return None;
    }

    let app = String::from_utf8_lossy(&app_name.stdout)
        .trim()
        .to_string();

    if app.is_empty() {
        return None;
    }

    let window_title = Command::new("osascript")
        .args([
            "-e",
            "tell application \"System Events\" to get name of front window of first application process whose frontmost is true",
        ])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();

    Some((app, window_title))
}

/// Build a cronos `Event` for an app focus change.
fn build_event(app_name: &str, window_title: &str) -> Event {
    let mut metadata = HashMap::new();
    if !window_title.is_empty() {
        metadata.insert(
            "window_title".to_string(),
            serde_json::Value::String(window_title.to_string()),
        );
    }

    Event {
        id: EventId::new(),
        timestamp: cronos_common::now_ms(),
        source: CollectorSource::AppMonitor,
        kind: EventKind::AppFocused,
        subject: EntityRef {
            kind: EntityKind::App,
            identity: app_name.to_string(),
            attributes: HashMap::new(),
        },
        context: vec![],
        metadata,
    }
}

/// Perform the handshake with the daemon and then stream events.
async fn run_session(
    socket_path: &Path,
    rx: &mut tokio::sync::mpsc::Receiver<Event>,
) -> Result<()> {
    info!(path = %socket_path.display(), "connecting to daemon");

    let stream = UnixStream::connect(socket_path)
        .await
        .context("failed to connect to daemon socket")?;

    let (mut reader, mut writer) = tokio::io::split(stream);

    // --- handshake --------------------------------------------------------
    let handshake = Message::new(
        ulid::Ulid::new().to_string(),
        MessageKind::CollectorHandshake {
            name: "cronos-collect-appmon".into(),
            collector_version: env!("CARGO_PKG_VERSION").into(),
            source: CollectorSource::AppMonitor,
        },
    );
    write_frame(&mut writer, &handshake).await?;

    let ack = read_frame(&mut reader).await?;
    match &ack.kind {
        MessageKind::Ack { .. } => info!("handshake accepted"),
        MessageKind::Error { message, .. } => {
            anyhow::bail!("handshake rejected: {message}");
        }
        other => {
            anyhow::bail!("unexpected handshake response: {other:?}");
        }
    }

    // --- event loop -------------------------------------------------------
    loop {
        let event = match rx.recv().await {
            Some(ev) => ev,
            None => {
                info!("poll channel closed, exiting session");
                return Ok(());
            }
        };

        let msg_id = event.id.to_string();
        let msg = Message::new(msg_id, MessageKind::EmitEvent { event });

        debug!("sending app focus event");
        write_frame(&mut writer, &msg).await?;
        let _ack = read_frame(&mut reader).await?;
    }
}

/// Poll active window at a fixed interval, emitting events only on change.
async fn poll_loop(tx: tokio::sync::mpsc::Sender<Event>, poll_interval_ms: u64) {
    let mut last_app = String::new();
    let mut last_title = String::new();

    let interval = std::time::Duration::from_millis(poll_interval_ms);

    loop {
        if let Some((app, title)) = get_active_window() {
            if app != last_app || title != last_title {
                debug!(app = %app, title = %title, "focus changed");
                let event = build_event(&app, &title);
                if tx.send(event).await.is_err() {
                    error!("event channel closed");
                    return;
                }
                last_app = app;
                last_title = title;
            }
        }

        tokio::time::sleep(interval).await;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let paths = CronosPaths::resolve().context("resolve XDG paths")?;
    let config = CronosConfig::load(&paths.config_file).context("load config")?;

    cronos_common::init_tracing(&config.daemon.log_level);

    // Resolve socket path (config override or default).
    let socket_path = if config.daemon.socket_path.is_empty() {
        paths.socket_file.clone()
    } else {
        PathBuf::from(&config.daemon.socket_path)
    };

    // --- set up polling channel -------------------------------------------
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Event>(4096);

    // Default 3s poll interval
    let poll_interval_ms = 3000;

    tokio::spawn(async move {
        poll_loop(tx, poll_interval_ms).await;
    });

    // --- connect with reconnection loop ----------------------------------
    info!(
        version = PROTOCOL_VERSION,
        socket = %socket_path.display(),
        "cronos-collect-appmon starting"
    );

    loop {
        match run_session(&socket_path, &mut rx).await {
            Ok(()) => {
                info!("session ended cleanly");
                break;
            }
            Err(e) => {
                warn!("session error: {e:#}");
                info!("reconnecting in 5 seconds...");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }

    Ok(())
}
