use anyhow::{Context, Result};
use cronos_proto::*;
use std::path::Path;
use tokio::net::UnixStream;

/// Send a request to the daemon over the Unix socket and return the response.
pub async fn send_request(msg: Message, socket_path: &Path) -> Result<Message> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| {
            format!(
                "cannot connect to daemon at {}. Is it running?",
                socket_path.display()
            )
        })?;

    let (mut reader, mut writer) = stream.into_split();
    write_frame(&mut writer, &msg)
        .await
        .context("sending request")?;
    let response = read_frame(&mut reader)
        .await
        .context("reading response")?;
    Ok(response)
}
