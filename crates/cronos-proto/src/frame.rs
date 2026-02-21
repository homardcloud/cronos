use crate::message::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024; // 16 MB

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame too large: {size} bytes (max {MAX_FRAME_SIZE})")]
    TooLarge { size: u32 },
    #[error("connection closed")]
    ConnectionClosed,
}

pub async fn write_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    msg: &Message,
) -> Result<(), FrameError> {
    let payload = serde_json::to_vec(msg)?;
    let len = payload.len() as u32;
    if len > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge { size: len });
    }
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<Message, FrameError> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(FrameError::ConnectionClosed);
        }
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge { size: len });
    }
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload).await?;
    let msg = serde_json::from_slice(&payload)?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, MessageKind, PROTOCOL_VERSION};

    #[tokio::test]
    async fn frame_roundtrip() {
        let msg = Message::new("req-1".to_string(), MessageKind::Status);
        let mut buf = Vec::new();
        write_frame(&mut buf, &msg).await.unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let decoded = read_frame(&mut cursor).await.unwrap();
        assert_eq!(decoded.version, PROTOCOL_VERSION);
        assert_eq!(decoded.id, "req-1");
    }

    #[tokio::test]
    async fn frame_ack_roundtrip() {
        let msg = Message::ack("req-42");
        let mut buf = Vec::new();
        write_frame(&mut buf, &msg).await.unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let decoded = read_frame(&mut cursor).await.unwrap();
        match decoded.kind {
            MessageKind::Ack { request_id } => assert_eq!(request_id, "req-42"),
            other => panic!("expected Ack, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn frame_connection_closed() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let result = read_frame(&mut cursor).await;
        assert!(matches!(result, Err(FrameError::ConnectionClosed)));
    }
}
