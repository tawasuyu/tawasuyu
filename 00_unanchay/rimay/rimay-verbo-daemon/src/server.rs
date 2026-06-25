//! El daemon: sirve un `Provider` sobre un socket Unix.
//!
//! Un modelo se carga una vez en memoria del daemon; N procesos lo
//! consumen vía [`crate::DaemonClient`]. Para coexistencia multi-modelo
//! se levanta un daemon por modelo, cada uno en su propio socket —
//! convención operativa, no de código.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rimay_verbo_core::Provider;

use crate::transport::{Listener, Stream};
use crate::wire::{read_frame, write_frame, Request, Response};

/// Daemon de embeddings ligado al transporte de la plataforma (socket
/// Unix en Unix, TCP de loopback en el resto — ver [`crate::transport`]).
pub struct Daemon {
    listener: Listener,
    path: PathBuf,
}

impl Daemon {
    /// Bindea el daemon en `path`. Si quedó un recurso huérfano de una
    /// corrida anterior (socket o sidecar de puerto), se remueve antes.
    pub fn bind(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let listener = Listener::bind(&path)?;
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
        self.serve_with_shutdown(provider, std::future::pending::<()>()).await
    }

    /// Como [`serve`](Self::serve) pero con apagado cooperativo: cuando
    /// `shutdown` resuelve, el daemon deja de aceptar conexiones nuevas
    /// y devuelve `Ok(())`. Las tasks ya despachadas terminan por su
    /// cuenta (el `Drop` del listener libera el socket).
    pub async fn serve_with_shutdown<P, S>(
        self,
        provider: Arc<P>,
        shutdown: S,
    ) -> std::io::Result<()>
    where
        P: Provider + 'static,
        S: std::future::Future<Output = ()>,
    {
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let stream = accepted?;
                    let provider = provider.clone();
                    tokio::spawn(async move {
                        // Una conexión muerta no debe tumbar el daemon.
                        let _ = handle_conn(stream, provider).await;
                    });
                }
                _ = &mut shutdown => return Ok(()),
            }
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // Sin esto el recurso de nombre (socket Unix o sidecar de puerto)
        // queda huérfano.
        self.listener.cleanup();
    }
}

/// Bucle de una conexión: lee requests hasta EOF, responde cada uno.
async fn handle_conn<P: Provider>(
    mut stream: Stream,
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
        Request::Ping => Response::Pong,
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
