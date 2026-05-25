//! Codec de wire: frames length-prefixed con cuerpo postcard.
//!
//! Cada frame en el stream tiene la forma:
//! ```text
//! [4 bytes LE: longitud N] [N bytes: postcard(Frame)]
//! ```
//!
//! El `MAX_FRAME_BYTES` evita que un cliente malicioso/buggy reserve memoria
//! arbitraria al anunciar un length absurdo.

use std::io::{Error, ErrorKind, Result};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::messages::Frame;

/// Tamaño máximo de un frame antes de que el reader rechace la conexión.
/// 4 MiB cubre cualquier Card razonable con margen amplio.
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Escribe un frame al stream.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, frame: &Frame) -> Result<()> {
    let bytes = postcard::to_allocvec(frame)
        .map_err(|e| Error::new(ErrorKind::InvalidData, format!("postcard encode: {e}")))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("frame demasiado grande: {} bytes", bytes.len()),
        ));
    }
    let len = bytes.len() as u32;
    w.write_all(&len.to_le_bytes()).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

/// Lee un frame del stream.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Frame> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("frame anunciado demasiado grande: {len} bytes"),
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    postcard::from_bytes(&buf)
        .map_err(|e| Error::new(ErrorKind::InvalidData, format!("postcard decode: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{Frame, HandshakeError};

    #[tokio::test]
    async fn frame_roundtrip() {
        let frame = Frame::Error(HandshakeError::Rejected("test".into()));
        let mut buf = Vec::new();
        write_frame(&mut buf, &frame).await.unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let decoded = read_frame(&mut cursor).await.unwrap();
        match decoded {
            Frame::Error(HandshakeError::Rejected(s)) => assert_eq!(s, "test"),
            _ => panic!("variant mismatch"),
        }
    }
}
