//! Protocolo de cable del daemon de voz — frames postcard con prefijo de largo.
//!
//! Cada mensaje va como `u32` little-endian (largo) + bytes postcard. Es el
//! mismo encuadre que usa `rimay-verbo-daemon` y el resto de la suite para
//! sockets.
//!
//! A diferencia de verbo (que sirve **un** trait `Provider`), voz sirve **dos**
//! — STT ([`Transcriptor`](rimay_voz_core::Transcriptor)) y TTS
//! ([`Locutor`](rimay_voz_core::Locutor)) —, así que el `Request` multiplexa
//! ambos sobre la misma conexión. Un mismo daemon puede tener un backend real
//! para uno y mock para el otro (ej. whisper + piper, o whisper + mock).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{self, ErrorKind};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use rimay_voz_core::{Audio, Transcripcion};

/// Tope de tamaño de un frame (16 MiB). Un fragmento de audio de varios
/// segundos a 16/24 kHz cabe holgado; cualquier cosa mayor se trata como frame
/// corrupto.
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Petición del cliente al daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Handshake: etiqueta del backend de STT servido.
    ModeloStt,
    /// Handshake: etiqueta del backend de TTS servido.
    ModeloTts,
    /// Health check sin invocar ningún modelo.
    Ping,
    /// Transcribe un fragmento de audio (STT).
    Transcribir(Audio),
    /// Sintetiza voz para un texto (TTS).
    Sintetizar(String),
}

/// Respuesta del daemon al cliente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    ModeloStt(String),
    ModeloTts(String),
    Pong,
    Transcripcion(Transcripcion),
    Audio(Audio),
    /// El backend falló; el texto es el `Display` del `VozError`.
    Error(String),
}

/// Serializa `msg` y lo escribe como frame con prefijo de largo.
pub async fn write_frame<W, T>(w: &mut W, msg: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes =
        postcard::to_stdvec(msg).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(ErrorKind::InvalidData, "frame demasiado grande"));
    }
    w.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

/// Lee un frame y lo deserializa. `Ok(None)` si el peer cerró limpio antes de
/// empezar un frame nuevo (EOF esperado).
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
    let msg = postcard::from_bytes(&buf).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    Ok(Some(msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frame_de_audio_roundtrips() {
        let audio = Audio::new(vec![1, -2, 3, -4], 16_000);
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &Request::Transcribir(audio.clone()))
            .await
            .unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let got: Request = read_frame(&mut cursor).await.unwrap().unwrap();
        match got {
            Request::Transcribir(a) => {
                assert_eq!(a.muestras, audio.muestras);
                assert_eq!(a.hz, 16_000);
            }
            otro => panic!("esperaba Transcribir, vino {otro:?}"),
        }
    }

    #[tokio::test]
    async fn stream_vacio_lee_none() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let got: Option<Request> = read_frame(&mut cursor).await.unwrap();
        assert!(got.is_none());
    }
}
