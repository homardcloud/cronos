use anyhow::{Context, Result};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Check whether the daemon is reachable on its Unix socket.
pub fn is_daemon_running(socket_path: &Path) -> bool {
    UnixStream::connect(socket_path).is_ok()
}

/// Ensure the daemon is running, spawning it in the background if needed.
/// Polls the socket every 100ms for up to 5 seconds.
pub async fn ensure_daemon(socket_path: &Path) -> Result<()> {
    if is_daemon_running(socket_path) {
        return Ok(());
    }

    eprintln!("  Starting Cronos daemon in background...");

    let exe = std::env::current_exe().context("locating cronos binary")?;
    Command::new(&exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawning daemon process")?;

    // Poll for readiness
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if is_daemon_running(socket_path) {
            eprintln!("  Daemon ready.");
            return Ok(());
        }
    }

    anyhow::bail!("daemon did not become ready within 5 seconds")
}

/// Attempt to spawn the filesystem collector if the binary exists alongside cronos.
pub fn spawn_collector_if_absent() {
    spawn_binary_if_present("cronos-collect-fs");
}

/// Attempt to spawn the app monitor collector if the binary exists alongside cronos.
pub fn spawn_appmon_if_absent() {
    spawn_binary_if_present("cronos-collect-appmon");
}

fn spawn_binary_if_present(name: &str) {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    let binary = dir.join(name);

    if !binary.exists() {
        return;
    }

    // Fire-and-forget
    let _ = Command::new(&binary)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
