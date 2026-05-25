//! Driver de sincronización sobre I/O asíncrona.
//!
//! Bridge entre la `SyncSession` puramente lógica y cualquier
//! transporte que implemente `AsyncRead + AsyncWrite`. Encuadre
//! length-prefixed: cada `Message` se serializa con postcard y se
//! envía precedido de un `u32 LE` con su longitud en bytes.
//!
//! La estructura del bucle es:
//! 1. Drenar todos los `Message`s pendientes a la salida.
//! 2. Si la sesión declara `is_done`, salir.
//! 3. Bloquear esperando un `Message` entrante; alimentarlo a la
//!    sesión y volver al paso 1.
//!
//! Esto funciona porque cada paso del state machine emite los
//! mensajes que necesita inmediatamente — nunca quedan colgados
//! mensajes por un `Message` futuro. La única espera real ocurre en
//! el paso 3, cuando estamos esperando que el peer responda.

use std::collections::VecDeque;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::message::Message;
use crate::session::SyncSession;

/// Cota dura sobre el tamaño de un frame, para evitar que un peer
/// malicioso (o un bug) cause asignaciones desbocadas. 16 MB es de
/// sobra para mensajes de sync — un `AttestPush` de cien mil
/// atestaciones cabe en ~13 MB.
const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum AsyncSyncError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("decode postcard: {0}")]
    Decode(#[from] postcard::Error),

    #[error("frame demasiado grande: {0} bytes")]
    FrameTooLarge(u32),

    #[error("la sesión cerró sin alcanzar `is_done`")]
    UnexpectedClose,
}

/// Ejecuta una sesión de sincronización completa sobre una stream
/// duplex. Devuelve la `SyncSession` resultante (con el `Mst`,
/// `MemStore` y `AttestationStore` ya mergeados con el peer).
pub async fn run_sync_async<S>(
    mut session: SyncSession,
    mut stream: S,
) -> Result<SyncSession, AsyncSyncError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut outbound: VecDeque<Message> = session.start().into();

    loop {
        while let Some(msg) = outbound.pop_front() {
            send_frame(&mut stream, &msg).await?;
        }

        if session.is_done() {
            return Ok(session);
        }

        let msg = recv_frame(&mut stream).await?;
        outbound.extend(session.handle(msg));
    }
}

async fn send_frame<S>(stream: &mut S, msg: &Message) -> Result<(), AsyncSyncError>
where
    S: AsyncWrite + Unpin,
{
    let bytes = msg.encode();
    let len = bytes.len() as u32;
    if len > MAX_FRAME_SIZE {
        return Err(AsyncSyncError::FrameTooLarge(len));
    }
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_frame<S>(stream: &mut S) -> Result<Message, AsyncSyncError>
where
    S: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_SIZE {
        return Err(AsyncSyncError::FrameTooLarge(len));
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    Ok(Message::decode(&buf)?)
}
