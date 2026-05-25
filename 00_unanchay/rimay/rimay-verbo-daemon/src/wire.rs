//! Protocolo de cable del daemon — frames postcard con prefijo de largo.
//!
//! Cada mensaje va como `u32` little-endian (largo) + bytes postcard.
//! Es el mismo encuadre que usa el resto de brahman para sockets.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{self, ErrorKind};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use rimay_verbo_core::{EmbeddingVector, ModelId};

/// Tope de tamaño de un frame (8 MiB). Un lote grande de embeddings
/// cabe holgado; cualquier cosa mayor se trata como frame corrupto.
const MAX_FRAME: usize = 8 * 1024 * 1024;

/// Petición del cliente al daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Handshake: pide la identidad del modelo servido.
    ModelId,
    /// Embebe un texto.
    Embed(String),
    /// Embebe un lote en un solo round-trip.
    EmbedBatch(Vec<String>),
}

/// Respuesta del daemon al cliente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    ModelId(ModelId),
    Embed(EmbeddingVector),
    EmbedBatch(Vec<EmbeddingVector>),
    /// El backend falló; el texto es el `Display` del `EmbedError`.
    Error(String),
}

/// Serializa `msg` y lo escribe como frame con prefijo de largo.
pub async fn write_frame<W, T>(w: &mut W, msg: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = postcard::to_stdvec(msg)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(ErrorKind::InvalidData, "frame demasiado grande"));
    }
    w.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

/// Lee un frame y lo deserializa. `Ok(None)` si el peer cerró limpio
/// antes de empezar un frame nuevo (EOF esperado).
pub async fn read_frame<R, T>(r: &mut R) -> io::Result<Option<T>>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(ErrorKind::InvalidData, "frame demasiado grande"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    let msg = postcard::from_bytes(&buf)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    Ok(Some(msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frame_roundtrips_through_a_buffer() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &Request::Embed("hola".into())).await.unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let got: Request = read_frame(&mut cursor).await.unwrap().unwrap();
        assert!(matches!(got, Request::Embed(t) if t == "hola"));
    }

    #[tokio::test]
    async fn empty_stream_reads_as_none() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let got: Option<Request> = read_frame(&mut cursor).await.unwrap();
        assert!(got.is_none());
    }
}
