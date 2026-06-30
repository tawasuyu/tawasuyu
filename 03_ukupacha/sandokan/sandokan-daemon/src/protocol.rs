//! Protocolo wire del daemon: requests/responses + framing.
//!
//! Encoding: postcard. Framing: prefijo de longitud `u32` little-endian
//! seguido de los bytes postcard. Mismo patrón que el wire de shuma.

use sandokan_core::{EngineError, ExecHandle, Intent, PtySize, TelemetryFrame};
use sandokan_lifecycle::LifecycleState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use ulid::Ulid;

/// Request del cliente al daemon. Espeja los métodos de `Engine` +
/// `InteractiveEngine`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonRequest {
    Run(Intent),
    Stop { card_id: Ulid, grace_ms: u64 },
    List,
    Status { card_id: Ulid },
    Telemetry { card_id: Ulid },
    /// Encarna una sesión interactiva. El attach NO va por acá: la respuesta
    /// trae el `socket_path` y el front se conecta ahí directamente.
    RunInteractive { intent: Intent, size: PtySize },
    /// Reweight en caliente de un cgroup ya existente (slice de un contexto).
    SetCpuWeight { cgroup_path: String, weight: u32 },
    /// Freeze/unfreeze de un cgroup (freezer v2, jerárquico).
    Freeze { cgroup_path: String, frozen: bool },
    /// Reinicia una unidad (stop→run del intent retenido). Append al final del
    /// enum: postcard numera por posición y un cliente viejo no debe correrse.
    Restart { card_id: Ulid, grace_ms: u64 },
}

/// Response del daemon al cliente. Una variante por resultado posible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonResponse {
    Ran(ExecHandle),
    Stopped,
    Listed(Vec<ExecHandle>),
    Status(LifecycleState),
    Telemetry(TelemetryFrame),
    Err(EngineError),
    /// Sesión interactiva encarnada: handle + el socket canónico donde el
    /// front attacha (`<run_dir>/<card_id>.sock`).
    RanInteractive {
        handle: ExecHandle,
        socket_path: PathBuf,
    },
    /// Ack genérico de una operación sin payload (reweight/freeze de cgroup).
    Done,
}

/// Límite defensivo de tamaño de frame (16 MiB). Un Intent con una Card
/// grande sigue cabiendo; protege contra frames corruptos/maliciosos.
pub const MAX_FRAME: u32 = 16 * 1024 * 1024;

/// Escribe un valor serializable como frame length-prefixed.
pub async fn write_frame<W, T>(w: &mut W, value: &T) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = postcard::to_stdvec(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if bytes.len() as u64 > MAX_FRAME as u64 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame excede MAX_FRAME",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

/// Lee un frame length-prefixed y lo deserializa.
pub async fn read_frame<R, T>(r: &mut R) -> std::io::Result<T>
where
    R: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame entrante excede MAX_FRAME",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    postcard::from_bytes(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
