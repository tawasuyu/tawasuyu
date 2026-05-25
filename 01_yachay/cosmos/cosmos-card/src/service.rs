//! Service socket de Tahuantinsuyu — protocolo y server.
//!
//! La Card de Tahuantinsuyu declara desde fase 1 los flows
//! `chart-request` (input) y `chart-result` (output). Acá vive el
//! **data plane** real que los implementa: un Unix socket sobre el que
//! cualquier módulo brahman puede pedir un cómputo de carta y recibir
//! el RenderModel ya armado.
//!
//! ## Protocolo
//!
//! Frame: `u32 length` little-endian + `postcard`-serialized payload.
//! Misma forma que `brahman-handshake` para reducir sorpresas.
//!
//! ## Endpoints
//!
//! - `ComputeRequest::Natal { birth, config, offset_minutes }` →
//!   `ComputeResponse::Render { render }` o `Error { message }`.
//! - `ComputeRequest::Ping` → `ComputeResponse::Pong`.
//!
//! El service no expone los overlays (transit / synastry / etc) por
//! ahora — son una pasada futura. Cubre el caso 80%: "necesito la
//! carta natal de estos datos".

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use cosmos_engine::{compose_with_options, NatalOptions, RenderModel};
use cosmos_model::{Chart, ChartId, ChartKind, ContactId, StoredBirthData, StoredChartConfig};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, error, info, warn};

/// Path canónico del service socket. Usa `XDG_RUNTIME_DIR` si está
/// (por usuario, no persistente), sino cae a `/tmp/cosmos_app.sock`.
pub fn default_service_socket() -> PathBuf {
    if let Some(rt) = directories::ProjectDirs::from("net", "gioser", "cosmos_app") {
        // ProjectDirs no expone runtime_dir directo en todas las
        // plataformas — usamos cache_dir como fallback estable.
        let mut p = rt.cache_dir().to_path_buf();
        std::fs::create_dir_all(&p).ok();
        p.push("service.sock");
        return p;
    }
    PathBuf::from("/tmp/cosmos_app.sock")
}

// =====================================================================
// Tipos del protocolo
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComputeRequest {
    /// Salud del server. Usá para verificar que el sidecar está vivo.
    Ping,
    /// Pide el cómputo de una carta natal pura (sin overlays).
    Natal {
        birth: StoredBirthData,
        config: StoredChartConfig,
        /// Offset en minutos sobre el instante natal — útil para
        /// rectificación rápida sin guardar variantes.
        #[serde(default)]
        offset_minutes: i64,
        /// Label opcional para que el render lo lleve en su title.
        #[serde(default)]
        label: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComputeResponse {
    Pong,
    Render { render: RenderModel },
    Error { message: String },
}

// =====================================================================
// Errores
// =====================================================================

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("postcard: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("frame demasiado grande: {0} bytes")]
    FrameTooLarge(u32),
    #[error("connect a {path}: {source}")]
    Connect {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Cap de tamaño de frame — defensivo contra peers malformados.
const MAX_FRAME_BYTES: u32 = 1024 * 1024; // 1 MiB

// =====================================================================
// Server
// =====================================================================

/// Arranca el server async sobre `socket_path`. Cada conexión nueva
/// procesa una secuencia de Request/Response hasta que el peer cierra.
pub async fn serve(socket_path: PathBuf) -> Result<(), ServiceError> {
    // Si quedó un socket viejo del run anterior, lo borramos.
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path)?;
    info!(socket = %socket_path.display(), "cosmos_app service socket arriba");

    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(e) = serve_connection(stream).await {
                warn!(?e, "connection terminó con error");
            }
        });
    }
}

async fn serve_connection(mut stream: UnixStream) -> Result<(), ServiceError> {
    loop {
        let request: ComputeRequest = match read_frame(&mut stream).await {
            Ok(r) => r,
            Err(ServiceError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                debug!("peer cerró");
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        let response = handle(request);
        write_frame(&mut stream, &response).await?;
    }
}

fn handle(req: ComputeRequest) -> ComputeResponse {
    match req {
        ComputeRequest::Ping => ComputeResponse::Pong,
        ComputeRequest::Natal {
            birth,
            config,
            offset_minutes,
            label,
        } => {
            let chart = Chart {
                id: ChartId::new(),
                contact_id: ContactId::new(),
                kind: ChartKind::Natal,
                label: label.unwrap_or_else(|| "Service request".into()),
                birth_data: birth,
                config,
                related_chart_id: None,
                created_at_ms: 0,
            };
            match compose_with_options(&chart, offset_minutes, &[], &NatalOptions::default()) {
                Ok(render) => ComputeResponse::Render { render },
                Err(e) => ComputeResponse::Error {
                    message: format!("{}", e),
                },
            }
        }
    }
}

// =====================================================================
// Client helper
// =====================================================================

/// Cliente async: abre el socket, envía un request, espera la response.
/// Cierra la conexión al volver (no reusable; útil para CLI/tests).
pub async fn request(
    socket: &Path,
    req: &ComputeRequest,
) -> Result<ComputeResponse, ServiceError> {
    let mut stream = UnixStream::connect(socket).await.map_err(|source| {
        ServiceError::Connect {
            path: socket.to_path_buf(),
            source,
        }
    })?;
    write_frame(&mut stream, req).await?;
    read_frame(&mut stream).await
}

// =====================================================================
// Framing
// =====================================================================

async fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<(), ServiceError> {
    let bytes = postcard::to_allocvec(value)?;
    let len = u32::try_from(bytes.len()).map_err(|_| ServiceError::FrameTooLarge(u32::MAX))?;
    if len > MAX_FRAME_BYTES {
        return Err(ServiceError::FrameTooLarge(len));
    }
    stream.write_u32_le(len).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame<T: for<'de> Deserialize<'de>>(
    stream: &mut UnixStream,
) -> Result<T, ServiceError> {
    let len = stream.read_u32_le().await?;
    if len > MAX_FRAME_BYTES {
        return Err(ServiceError::FrameTooLarge(len));
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    let value = postcard::from_bytes(&buf)?;
    Ok(value)
}

// =====================================================================
// Spawn helper para uso desde el binario GUI
// =====================================================================

/// Spawn fire-and-forget: thread aparte con tokio runtime current_thread
/// corriendo el server. Si la initialización falla, loggea warn y el
/// thread termina. El binario GUI sigue funcionando standalone.
pub fn spawn_service_thread(socket_path: PathBuf) {
    std::thread::Builder::new()
        .name("cosmos_app-service".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    error!(?e, "no pude crear runtime para service thread");
                    return;
                }
            };
            if let Err(e) = rt.block_on(serve(socket_path)) {
                error!(?e, "service server terminó con error");
            }
        })
        .map(|_| ())
        .unwrap_or_else(|e| {
            error!(?e, "no pude spawnear thread del service socket");
        });
}
