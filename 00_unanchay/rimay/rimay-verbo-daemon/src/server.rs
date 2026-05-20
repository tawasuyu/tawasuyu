//! El daemon: sirve un `Provider` sobre un socket Unix.
//!
//! Un modelo se carga una vez en memoria del daemon; N procesos lo
//! consumen vía [`crate::DaemonClient`]. Para coexistencia multi-modelo
//! se levanta un daemon por modelo, cada uno en su propio socket —
//! convención operativa, no de código.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::net::{UnixListener, UnixStream};
use verbo_core::Provider;

use crate::wire::{read_frame, write_frame, Request, Response};

/// Daemon de embeddings ligado a un socket Unix.
pub struct Daemon {
    listener: UnixListener,
    path: PathBuf,
}

impl Daemon {
    /// Bindea el socket Unix en `path`. Si quedó un socket huérfano de
    /// una corrida anterior, se remueve antes de bindear.
    pub fn bind(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        Ok(Self { listener, path })
    }

    /// Ruta del socket que este daemon escucha.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Atiende conexiones para siempre, sirviendo `provider`. Cada
    /// conexión corre en su propia task; el provider se comparte por
    /// `Arc` — un modelo, muchos clientes concurrentes.
    pub async fn serve<P: Provider + 'static>(self, provider: Arc<P>) -> std::io::Result<()> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            let provider = provider.clone();
            tokio::spawn(async move {
                // Una conexión muerta no debe tumbar el daemon.
                let _ = handle_conn(stream, provider).await;
            });
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // Sin esto el socket Unix queda como archivo huérfano.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Bucle de una conexión: lee requests hasta EOF, responde cada uno.
async fn handle_conn<P: Provider>(
    mut stream: UnixStream,
    provider: Arc<P>,
) -> std::io::Result<()> {
    while let Some(req) = read_frame::<_, Request>(&mut stream).await? {
        let resp = dispatch(&*provider, req).await;
        write_frame(&mut stream, &resp).await?;
    }
    Ok(())
}

/// Traduce un `Request` a una llamada al provider y arma el `Response`.
async fn dispatch<P: Provider>(provider: &P, req: Request) -> Response {
    match req {
        Request::ModelId => Response::ModelId(provider.model_id().clone()),
        Request::Embed(text) => match provider.embed(&text).await {
            Ok(v) => Response::Embed(v),
            Err(e) => Response::Error(e.to_string()),
        },
        Request::EmbedBatch(texts) => match provider.embed_batch(&texts).await {
            Ok(v) => Response::EmbedBatch(v),
            Err(e) => Response::Error(e.to_string()),
        },
    }
}
