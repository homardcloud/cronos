use anyhow::{Context, Result};
use cronos_common::{CronosConfig, CronosPaths};
use cronos_model::*;
use cronos_proto::{read_frame, write_frame, Message, MessageKind, PROTOCOL_VERSION};
use glob_match::glob_match;
use notify::{EventKind as NotifyEventKind, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::net::UnixStream;
use tracing::{debug, error, info, warn};

/// Project-root marker files. Walking up from a changed file, the first
/// directory that contains one of these is considered the project root.
const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "go.mod",
    "pyproject.toml",
    "Makefile",
];

/// Expand a leading `~` to `$HOME`.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(path)
}

/// Walk upward from `path` looking for a project-root marker.
fn detect_project_root(path: &Path) -> Option<PathBuf> {
    let mut dir = if path.is_file() {
        path.parent()?.to_path_buf()
    } else {
        path.to_path_buf()
    };

    loop {
        for marker in PROJECT_MARKERS {
            if dir.join(marker).exists() {
                return Some(dir);
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Check whether a path matches any of the ignore patterns.
fn is_ignored(path: &Path, patterns: &[String]) -> bool {
    let path_str = path.to_string_lossy();
    for pattern in patterns {
        if glob_match(pattern, &path_str) {
            return true;
        }
    }
    false
}

/// Map a `notify::EventKind` to a `cronos_model::EventKind`, if applicable.
fn map_event_kind(kind: &NotifyEventKind) -> Option<EventKind> {
    match kind {
        NotifyEventKind::Create(_) => Some(EventKind::FileCreated),
        NotifyEventKind::Modify(_) => Some(EventKind::FileModified),
        NotifyEventKind::Remove(_) => Some(EventKind::FileDeleted),
        _ => None,
    }
}

/// Build a cronos `Event` from a file-system notification.
fn build_event(path: &Path, kind: EventKind) -> Event {
    let project_ctx: Vec<EntityRef> = detect_project_root(path)
        .into_iter()
        .map(|root| EntityRef {
            kind: EntityKind::Project,
            identity: root.to_string_lossy().into_owned(),
            attributes: HashMap::new(),
        })
        .collect();

    Event {
        id: EventId::new(),
        timestamp: cronos_common::now_ms(),
        source: CollectorSource::Filesystem,
        kind,
        subject: EntityRef {
            kind: EntityKind::File,
            identity: path.to_string_lossy().into_owned(),
            attributes: HashMap::new(),
        },
        context: project_ctx,
        metadata: HashMap::new(),
    }
}

/// Perform the handshake with the daemon and then stream events.
async fn run_session(
    socket_path: &Path,
    rx: &mut tokio::sync::mpsc::Receiver<notify::Event>,
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
            name: "cronos-collect-fs".into(),
            collector_version: env!("CARGO_PKG_VERSION").into(),
            source: CollectorSource::Filesystem,
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
        let notify_event = match rx.recv().await {
            Some(ev) => ev,
            None => {
                info!("watcher channel closed, exiting session");
                return Ok(());
            }
        };

        let cronos_kind = match map_event_kind(&notify_event.kind) {
            Some(k) => k,
            None => continue,
        };

        for path in &notify_event.paths {
            let event = build_event(path, cronos_kind.clone());
            let msg_id = event.id.to_string();
            let msg = Message::new(msg_id, MessageKind::EmitEvent { event });

            debug!(path = %path.display(), "sending event");
            write_frame(&mut writer, &msg).await?;
            let _ack = read_frame(&mut reader).await?;
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let paths = CronosPaths::resolve().context("resolve XDG paths")?;
    let config = CronosConfig::load(&paths.config_file).context("load config")?;

    cronos_common::init_tracing(&config.daemon.log_level);

    let fs_cfg = &config.collectors.fs;
    if !fs_cfg.enabled {
        info!("filesystem collector disabled in config, exiting");
        return Ok(());
    }

    // Resolve socket path (config override or default).
    let socket_path = if config.daemon.socket_path.is_empty() {
        paths.socket_file.clone()
    } else {
        PathBuf::from(&config.daemon.socket_path)
    };

    // --- set up file watcher ---------------------------------------------
    let ignore_patterns = fs_cfg.ignore_patterns.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<notify::Event>(4096);

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        match res {
            Ok(event) => {
                // Pre-filter: skip events on ignored paths.
                let dominated_by_ignore = event
                    .paths
                    .iter()
                    .all(|p| is_ignored(p, &ignore_patterns));
                if dominated_by_ignore {
                    return;
                }
                if tx.blocking_send(event).is_err() {
                    error!("event channel closed");
                }
            }
            Err(e) => {
                warn!("watcher error: {e}");
            }
        }
    })
    .context("create file watcher")?;

    let watch_paths: Vec<PathBuf> = fs_cfg.watch_paths.iter().map(|p| expand_tilde(p)).collect();

    for path in &watch_paths {
        if path.exists() {
            watcher
                .watch(path, RecursiveMode::Recursive)
                .with_context(|| format!("watch {}", path.display()))?;
            info!(path = %path.display(), "watching directory");
        } else {
            warn!(path = %path.display(), "watch path does not exist, skipping");
        }
    }

    // --- connect with reconnection loop ----------------------------------
    info!(
        version = PROTOCOL_VERSION,
        socket = %socket_path.display(),
        "cronos-collect-fs starting"
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
