//! Protocolo del socket de control. Encoding postcard, framing
//! length-prefixed `u32` LE — mismo patrón que el wire de sandokan/shuma.

use pacha_core::{Lifecycle, Pacha};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Request de un cliente (CLI/UI) al daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Req {
    /// Cambiar de contexto. `fresh` ignora `last_session` y usa la receta.
    Switch { to: String, fresh: bool },
    /// Cerrar un contexto (liberar recursos) sin cambiar el foco.
    Close { id: String },
    /// Listar contextos definidos + su estado.
    List,
    /// Estado completo (activo + lista).
    Status,
    /// Alta/edición de una definición (persistida).
    Define(Box<Pacha>),
    /// Baja de una definición.
    Remove { id: String },
}

/// Una línea de contexto para la UI/CLI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PachaInfo {
    pub id: String,
    pub label: String,
    pub lifecycle: Lifecycle,
    pub active: bool,
}

/// Response del daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Resp {
    Ok,
    /// Transición hecha: contexto activo resultante + warnings de efectos
    /// best-effort que fallaron (cgroup sin delegación, compositor ausente…).
    Switched { active: Option<String>, warnings: Vec<String> },
    List(Vec<PachaInfo>),
    Err(String),
}

/// Límite defensivo de frame (4 MiB; una `Pacha` cabe de sobra).
pub const MAX_FRAME: u32 = 4 * 1024 * 1024;

/// Escribe un valor como frame length-prefixed postcard.
pub async fn write_frame<W, T>(w: &mut W, value: &T) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = postcard::to_stdvec(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if bytes.len() as u64 > MAX_FRAME as u64 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame excede MAX_FRAME"));
    }
    w.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    w.write_all(&bytes).await?;
    w.flush().await
}

/// Lee y deserializa un frame length-prefixed.
pub async fn read_frame<R, T>(r: &mut R) -> std::io::Result<T>
where
    R: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len);
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame entrante excede MAX_FRAME"));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    postcard::from_bytes(&buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Cliente: abre el socket, manda `req`, devuelve la respuesta. Usado por la
/// CLI y la UI de pata.
pub async fn request(socket: &std::path::Path, req: &Req) -> std::io::Result<Resp> {
    let mut stream = tokio::net::UnixStream::connect(socket).await?;
    write_frame(&mut stream, req).await?;
    read_frame(&mut stream).await
}
