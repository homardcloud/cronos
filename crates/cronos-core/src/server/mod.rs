use crate::engine::Engine;
use cronos_proto::{read_frame, write_frame};
use std::path::Path;
use std::sync::Arc;
use tokio::net::UnixListener;

pub async fn run(engine: Arc<Engine>, socket_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let _ = tokio::fs::remove_file(socket_path).await;
    let listener = UnixListener::bind(socket_path)?;
    tracing::info!(path = %socket_path.display(), "listening on unix socket");

    loop {
        let (stream, _addr) = listener.accept().await?;
        let engine = Arc::clone(&engine);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(engine, stream).await {
                tracing::debug!("connection closed: {}", e);
            }
        });
    }
}

async fn handle_connection(
    engine: Arc<Engine>,
    stream: tokio::net::UnixStream,
) -> anyhow::Result<()> {
    let (mut reader, mut writer) = stream.into_split();
    loop {
        let msg = read_frame(&mut reader).await?;
        let response = engine.handle_message(msg);
        write_frame(&mut writer, &response).await?;
    }
}
