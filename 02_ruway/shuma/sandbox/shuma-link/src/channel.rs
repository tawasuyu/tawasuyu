//! Canal cifrado post-handshake.
//!
//! Tras `client_handshake`/`server_handshake`, ambas partes tienen una
//! `snow::TransportState`. Este módulo expone un wrapper que envía y
//! recibe **payloads opacos** sobre un `AsyncRead + AsyncWrite`,
//! delegando en Noise el cifrado/auth de cada mensaje.
//!
//! Wire por mensaje: `[u32 BE length][N bytes ciphertext]`. El cipher
//! es ChaCha20-Poly1305 con autenticación implícita (Poly1305 tag de
//! 16 B incluido en `ciphertext`). Tope de payload por mensaje: 65 519
//! bytes (Noise max 65 535 − 16 de tag). Llamadas con payload mayor
//! devuelven `FrameError::Oversize`.

use std::sync::{Arc, Mutex};

use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};

/// Tope del payload claro por mensaje Noise (en bytes).
pub const MAX_PAYLOAD: usize = 65535 - 16;

/// Canal cifrado bidireccional sobre cualquier transporte
/// `AsyncRead + AsyncWrite`. Tras el handshake, todo va por aquí.
pub struct FramedChannel<S> {
    stream: S,
    // `snow::TransportState` no es `Sync` ni soporta usar el mismo
    // estado para encrypt y decrypt sin sincronizar el counter — lo
    // metemos en un Mutex porque send/recv pueden invocarse desde
    // tareas distintas. Cada operación toma el lock corto.
    noise: Mutex<snow::TransportState>,
}

impl<S> FramedChannel<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    /// Construye el canal sobre `stream`. El `noise` debe venir de un
    /// handshake completado (`into_transport_mode()` ya llamado).
    pub fn new(stream: S, noise: snow::TransportState) -> Self {
        Self { stream, noise: Mutex::new(noise) }
    }

    /// Envía `payload` cifrado y autenticado. Devuelve error si supera
    /// `MAX_PAYLOAD`.
    pub async fn send(&mut self, payload: &[u8]) -> Result<(), FrameError> {
        if payload.len() > MAX_PAYLOAD {
            return Err(FrameError::Oversize(payload.len()));
        }
        let mut ct = vec![0u8; payload.len() + 16];
        let n = {
            let mut noise = self.noise.lock().expect("noise mutex");
            noise.write_message(payload, &mut ct).map_err(FrameError::Snow)?
        };
        ct.truncate(n);
        let len = (n as u32).to_be_bytes();
        self.stream.write_all(&len).await.map_err(FrameError::Io)?;
        self.stream.write_all(&ct).await.map_err(FrameError::Io)?;
        self.stream.flush().await.map_err(FrameError::Io)?;
        Ok(())
    }

    /// Recibe el próximo mensaje. Bloquea hasta que llega o el peer
    /// cierra (en cuyo caso retorna `Closed`).
    pub async fn recv(&mut self) -> Result<Vec<u8>, FrameError> {
        let mut len_buf = [0u8; 4];
        match self.stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(FrameError::Closed)
            }
            Err(e) => return Err(FrameError::Io(e)),
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_PAYLOAD + 16 {
            return Err(FrameError::Oversize(len));
        }
        let mut ct = vec![0u8; len];
        self.stream.read_exact(&mut ct).await.map_err(FrameError::Io)?;
        let mut pt = vec![0u8; len];
        let n = {
            let mut noise = self.noise.lock().expect("noise mutex");
            noise.read_message(&ct, &mut pt).map_err(FrameError::Snow)?
        };
        pt.truncate(n);
        Ok(pt)
    }

    /// Devuelve el stream subyacente, descartando el estado Noise.
    /// Útil al cerrar para drenar el TCP/Unix subyacente.
    pub fn into_inner(self) -> S {
        self.stream
    }

    /// Parte el canal en mitades **lectura** y **escritura** que se pueden
    /// usar concurrentemente desde tareas/ramas `select!` distintas —
    /// necesario para full-duplex (PTY remoto: leer teclas mientras se
    /// escribe la salida del terminal). El estado Noise se comparte por
    /// `Arc<Mutex>`: cada operación toma el lock sólo para la transformación
    /// cripto en memoria (no a través de un `await`), así que un `recv`
    /// bloqueado esperando bytes nunca frena al `send` del otro lado.
    pub fn split(self) -> (FramedReader<S>, FramedWriter<S>) {
        let (rd, wr) = tokio::io::split(self.stream);
        let noise = Arc::new(self.noise);
        (
            FramedReader { rd, noise: Arc::clone(&noise) },
            FramedWriter { wr, noise },
        )
    }

    /// Conveniencia: serializa `msg` con postcard y lo envía como un
    /// frame cifrado. El daemon y `shuma-remote-exec` lo usan para
    /// emitir `Request`/`Response` sobre la conexión autenticada,
    /// reemplazando los `write_frame`/`read_frame` de `shuma-protocol`
    /// cuando hablan por la red.
    pub async fn send_postcard<T: serde::Serialize>(
        &mut self,
        msg: &T,
    ) -> Result<(), FrameError> {
        let bytes = postcard::to_allocvec(msg).map_err(FrameError::Postcard)?;
        self.send(&bytes).await
    }

    /// Variante de [`FramedChannel::recv`] que deserializa con postcard.
    pub async fn recv_postcard<T>(&mut self) -> Result<T, FrameError>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        let bytes = self.recv().await?;
        postcard::from_bytes(&bytes).map_err(FrameError::Postcard)
    }
}

/// Mitad de **lectura** de un [`FramedChannel`] splitteado. Sólo recibe.
/// El estado Noise lo comparte (vía `Arc<Mutex>`) con su
/// [`FramedWriter`] hermano.
pub struct FramedReader<S> {
    rd: ReadHalf<S>,
    noise: Arc<Mutex<snow::TransportState>>,
}

/// Mitad de **escritura** de un [`FramedChannel`] splitteado. Sólo envía.
pub struct FramedWriter<S> {
    wr: WriteHalf<S>,
    noise: Arc<Mutex<snow::TransportState>>,
}

impl<S> FramedReader<S>
where
    S: tokio::io::AsyncRead + Unpin,
{
    /// Espejo de [`FramedChannel::recv`] sobre la mitad de lectura.
    pub async fn recv(&mut self) -> Result<Vec<u8>, FrameError> {
        let mut len_buf = [0u8; 4];
        match self.rd.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(FrameError::Closed)
            }
            Err(e) => return Err(FrameError::Io(e)),
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_PAYLOAD + 16 {
            return Err(FrameError::Oversize(len));
        }
        let mut ct = vec![0u8; len];
        self.rd.read_exact(&mut ct).await.map_err(FrameError::Io)?;
        let mut pt = vec![0u8; len];
        let n = {
            let mut noise = self.noise.lock().expect("noise mutex");
            noise.read_message(&ct, &mut pt).map_err(FrameError::Snow)?
        };
        pt.truncate(n);
        Ok(pt)
    }

    /// Espejo de [`FramedChannel::recv_postcard`] sobre la mitad de lectura.
    pub async fn recv_postcard<T>(&mut self) -> Result<T, FrameError>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        let bytes = self.recv().await?;
        postcard::from_bytes(&bytes).map_err(FrameError::Postcard)
    }
}

impl<S> FramedWriter<S>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    /// Espejo de [`FramedChannel::send`] sobre la mitad de escritura.
    pub async fn send(&mut self, payload: &[u8]) -> Result<(), FrameError> {
        if payload.len() > MAX_PAYLOAD {
            return Err(FrameError::Oversize(payload.len()));
        }
        let mut ct = vec![0u8; payload.len() + 16];
        let n = {
            let mut noise = self.noise.lock().expect("noise mutex");
            noise.write_message(payload, &mut ct).map_err(FrameError::Snow)?
        };
        ct.truncate(n);
        let len = (n as u32).to_be_bytes();
        self.wr.write_all(&len).await.map_err(FrameError::Io)?;
        self.wr.write_all(&ct).await.map_err(FrameError::Io)?;
        self.wr.flush().await.map_err(FrameError::Io)?;
        Ok(())
    }

    /// Espejo de [`FramedChannel::send_postcard`] sobre la mitad de escritura.
    pub async fn send_postcard<T: serde::Serialize>(
        &mut self,
        msg: &T,
    ) -> Result<(), FrameError> {
        let bytes = postcard::to_allocvec(msg).map_err(FrameError::Postcard)?;
        self.send(&bytes).await
    }
}

/// Errores del canal cifrado.
#[derive(Debug, Error)]
pub enum FrameError {
    #[error("payload oversize: {0} bytes (max {MAX_PAYLOAD})")]
    Oversize(usize),
    #[error("io: {0}")]
    Io(std::io::Error),
    #[error("noise: {0}")]
    Snow(snow::Error),
    #[error("postcard: {0}")]
    Postcard(postcard::Error),
    #[error("conexión cerrada")]
    Closed,
}
