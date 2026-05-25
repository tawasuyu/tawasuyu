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
}

/// Mapea una respuesta fuera de contrato a un `EmbedError`.
fn unexpected(r: Response) -> EmbedError {
    match r {
        Response::Error(e) => EmbedError::Backend(e),
        _ => EmbedError::Backend("respuesta del daemon verbo inesperada".into()),
    }
}

/// Un round-trip completo: conecta, manda el request, lee la respuesta.
async fn round_trip(path: &Path, req: &Request) -> Result<Response, EmbedError> {
    let mut stream = UnixStream::connect(path)
        .await
        .map_err(|e| EmbedError::Backend(format!("conexión al daemon verbo: {e}")))?;
    write_frame(&mut stream, req)
        .await
        .map_err(|e| EmbedError::Backend(format!("envío al daemon verbo: {e}")))?;
    match read_frame::<_, Response>(&mut stream).await {
        Ok(Some(resp)) => Ok(resp),
        Ok(None) => Err(EmbedError::Backend(
            "el daemon verbo cerró la conexión sin responder".into(),
        )),
        Err(e) => Err(EmbedError::Backend(format!("lectura del daemon verbo: {e}"))),
    }
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
