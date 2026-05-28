//! El cliente: consume un daemon presentándose como un `Provider`.
//!
//! Un [`DaemonClient`] implementa `rimay_verbo_core::Provider`, así que
//! cualquier consumidor (`pluma_app-semantic`, `khipu_app`, `chasqui`) lo usa sin
//! saber que el modelo vive en otro proceso. Cada llamada es un
//! round-trip independiente: sin estado de conexión que reparar.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::net::UnixStream;
use rimay_verbo_core::{EmbedError, EmbeddingVector, ModelId, Provider};

use crate::wire::{read_frame, write_frame, Request, Response};

/// Cliente de un [`crate::Daemon`]. Se comporta como un `Provider`
/// local — los consumidores no notan que el modelo es remoto.
pub struct DaemonClient {
    path: PathBuf,
    model: ModelId,
}

impl DaemonClient {
    /// Conecta a un daemon y hace el handshake del modelo. El `ModelId`
    /// queda cacheado: marca los vectores y nunca cambia en vida del
    /// daemon.
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, EmbedError> {
        let path = path.as_ref().to_path_buf();
        let model = match round_trip(&path, &Request::ModelId).await? {
            Response::ModelId(m) => m,
            other => return Err(unexpected(other)),
        };
        Ok(Self { path, model })
    }

    /// Health check sin invocar al modelo. Útil para distinguir "no hay
    /// daemon" de "el modelo falló al embedir" antes de pegarle de verdad.
    pub async fn ping(&self) -> Result<(), EmbedError> {
        match round_trip(&self.path, &Request::Ping).await? {
            Response::Pong => Ok(()),
            other => Err(unexpected(other)),
        }
    }
}

/// Mapea una respuesta fuera de contrato a un `EmbedError`.
fn unexpected(r: Response) -> EmbedError {
    match r {
        Response::Error(e) => EmbedError::Backend(e),
        _ => EmbedError::Backend("respuesta del daemon verbo inesperada".into()),
    }
}

/// Una transmisión del request en un socket recién abierto. Devuelve
/// `Ok(Some(resp))` con la respuesta normal, `Ok(None)` cuando el peer
/// cerró antes de mandar nada (transitorio — el caller decide reintentar)
/// y `Err` cuando algo falló sin ambigüedad.
async fn intentar(path: &Path, req: &Request) -> Result<Option<Response>, EmbedError> {
    let mut stream = match UnixStream::connect(path).await {
        Ok(s) => s,
        Err(e) if es_transitorio(&e) => return Ok(None),
        Err(e) => {
            return Err(EmbedError::Backend(format!("conexión al daemon verbo: {e}")))
        }
    };
    if let Err(e) = write_frame(&mut stream, req).await {
        if es_transitorio(&e) {
            return Ok(None);
        }
        return Err(EmbedError::Backend(format!("envío al daemon verbo: {e}")));
    }
    match read_frame::<_, Response>(&mut stream).await {
        Ok(Some(resp)) => Ok(Some(resp)),
        Ok(None) => Ok(None), // peer cerró antes de responder: transitorio
        Err(e) if es_transitorio(&e) => Ok(None),
        Err(e) => Err(EmbedError::Backend(format!("lectura del daemon verbo: {e}"))),
    }
}

/// Un round-trip completo con un reintento corto. Si la primera vuelta
/// falla por una causa transitoria (daemon reiniciando, conexión cortada
/// antes de la respuesta), espera 100 ms y reintenta una sola vez. Si la
/// segunda también vuelve vacía se reporta como error de backend.
async fn round_trip(path: &Path, req: &Request) -> Result<Response, EmbedError> {
    if let Some(resp) = intentar(path, req).await? {
        return Ok(resp);
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    match intentar(path, req).await? {
        Some(resp) => Ok(resp),
        None => Err(EmbedError::Backend(
            "el daemon verbo no respondió tras un reintento (¿caído?)".into(),
        )),
    }
}

/// ¿El error pinta a "el daemon se cayó / reinició" en vez de un fallo
/// duro de aplicación? Estos justifican un reintento corto; el resto
/// debe propagarse tal cual.
fn es_transitorio(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(
        e.kind(),
        ConnectionRefused
            | ConnectionReset
            | ConnectionAborted
            | BrokenPipe
            | UnexpectedEof
            | NotFound
    )
}

#[async_trait]
impl Provider for DaemonClient {
    fn model_id(&self) -> &ModelId {
        &self.model
    }

    async fn embed(&self, text: &str) -> Result<EmbeddingVector, EmbedError> {
        match round_trip(&self.path, &Request::Embed(text.to_string())).await? {
            Response::Embed(v) => Ok(v),
            other => Err(unexpected(other)),
        }
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<EmbeddingVector>, EmbedError> {
        match round_trip(&self.path, &Request::EmbedBatch(texts.to_vec())).await? {
            Response::EmbedBatch(v) => Ok(v),
            other => Err(unexpected(other)),
        }
    }
}
