//! El daemon: sirve un par STT+TTS sobre un socket Unix.
//!
//! Los modelos se cargan una vez en memoria del daemon; N procesos los consumen
//! vía [`crate::DaemonClient`]. Para coexistencia multi-modelo se levanta un
//! daemon por par, cada uno en su propio socket — convención operativa, no de
//! código.
//!
//! El daemon sirve **dos** traits a la vez, como [`Arc<dyn Transcriptor>`] y
//! [`Arc<dyn Locutor>`]: así un mismo proceso puede cargar whisper para STT y
//! piper para TTS (o mock en el lado que aún no tenga backend real) y servir
//! ambos por una sola conexión.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rimay_voz_core::{Locutor, Transcriptor};

use crate::transport::{Listener, Stream};
use crate::wire::{read_frame, write_frame, Request, Response};

/// Daemon de voz ligado al transporte de la plataforma (socket Unix en Unix,
/// TCP de loopback en el resto — ver [`crate::transport`]).
pub struct Daemon {
    listener: Listener,
    path: PathBuf,
}

impl Daemon {
    /// Bindea el daemon en `path`. Si quedó un recurso huérfano de una corrida
    /// anterior (socket o sidecar de puerto), se remueve antes.
    pub fn bind(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let listener = Listener::bind(&path)?;
        Ok(Self { listener, path })
    }

    /// Ruta del socket que este daemon escucha.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Atiende conexiones para siempre, sirviendo `stt` + `tts`. Cada conexión
    /// corre en su propia task; los backends se comparten por `Arc` — un par de
    /// modelos, muchos clientes concurrentes.
    pub async fn serve(
        self,
        stt: Arc<dyn Transcriptor>,
        tts: Arc<dyn Locutor>,
    ) -> std::io::Result<()> {
        self.serve_with_shutdown(stt, tts, std::future::pending::<()>())
            .await
    }

    /// Como [`serve`](Self::serve) pero con apagado cooperativo: cuando
    /// `shutdown` resuelve, el daemon deja de aceptar conexiones nuevas y
    /// devuelve `Ok(())`. Las tasks ya despachadas terminan por su cuenta.
    pub async fn serve_with_shutdown<S>(
        self,
        stt: Arc<dyn Transcriptor>,
        tts: Arc<dyn Locutor>,
        shutdown: S,
    ) -> std::io::Result<()>
    where
        S: std::future::Future<Output = ()>,
    {
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let stream = accepted?;
                    let stt = stt.clone();
                    let tts = tts.clone();
                    tokio::spawn(async move {
                        // Una conexión muerta no debe tumbar el daemon.
                        let _ = handle_conn(stream, stt, tts).await;
                    });
                }
                _ = &mut shutdown => return Ok(()),
            }
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // Sin esto el recurso de nombre (socket Unix o sidecar de puerto) queda
        // huérfano.
        self.listener.cleanup();
    }
}

/// Bucle de una conexión: lee requests hasta EOF, responde cada uno.
async fn handle_conn(
    mut stream: Stream,
    stt: Arc<dyn Transcriptor>,
    tts: Arc<dyn Locutor>,
) -> std::io::Result<()> {
    while let Some(req) = read_frame::<_, Request>(&mut stream).await? {
        let resp = dispatch(&*stt, &*tts, req).await;
        write_frame(&mut stream, &resp).await?;
    }
    Ok(())
}

/// Traduce un `Request` a una llamada al backend correspondiente y arma el
/// `Response`.
async fn dispatch(stt: &dyn Transcriptor, tts: &dyn Locutor, req: Request) -> Response {
    match req {
        Request::ModeloStt => Response::ModeloStt(stt.modelo().to_string()),
        Request::ModeloTts => Response::ModeloTts(tts.modelo().to_string()),
        Request::Ping => Response::Pong,
        Request::Transcribir(audio) => match stt.transcribir(&audio).await {
            Ok(t) => Response::Transcripcion(t),
            Err(e) => Response::Error(e.to_string()),
        },
        Request::Sintetizar(texto) => match tts.sintetizar(&texto).await {
            Ok(a) => Response::Audio(a),
            Err(e) => Response::Error(e.to_string()),
        },
    }
}
