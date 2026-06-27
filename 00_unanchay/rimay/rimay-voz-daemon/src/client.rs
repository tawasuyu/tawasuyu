//! El cliente: consume un daemon presentándose como STT **y** TTS locales.
//!
//! Un [`DaemonClient`] implementa [`Transcriptor`] y [`Locutor`], así que
//! cualquier consumidor (la máquina de escucha de shuma, mirada, pluma) lo usa
//! sin saber que el modelo vive en otro proceso. Cada llamada es un round-trip
//! independiente: sin estado de conexión que reparar.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use rimay_voz_core::{Audio, Locutor, Transcripcion, Transcriptor, VozError};

use crate::transport;
use crate::wire::{read_frame, write_frame, Request, Response};

/// Cliente de un [`crate::Daemon`]. Se comporta como un par STT+TTS local — los
/// consumidores no notan que los modelos son remotos. Las etiquetas de modelo
/// se cachean en el handshake.
pub struct DaemonClient {
    path: PathBuf,
    modelo_stt: String,
    modelo_tts: String,
}

impl DaemonClient {
    /// Conecta a un daemon y hace el handshake de ambos modelos. Las etiquetas
    /// quedan cacheadas (rotulan la UI y no cambian en vida del daemon). Un lado
    /// que el daemon no sirva queda con la etiqueta de error del backend — no
    /// impide conectar (podés usar el otro lado).
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, VozError> {
        let path = path.as_ref().to_path_buf();
        let modelo_stt = match round_trip(&path, &Request::ModeloStt).await? {
            Response::ModeloStt(m) => m,
            Response::Error(e) => e,
            other => return Err(inesperado(other)),
        };
        let modelo_tts = match round_trip(&path, &Request::ModeloTts).await? {
            Response::ModeloTts(m) => m,
            Response::Error(e) => e,
            other => return Err(inesperado(other)),
        };
        Ok(Self { path, modelo_stt, modelo_tts })
    }

    /// Health check sin invocar a ningún modelo. Distingue "no hay daemon" de
    /// "el modelo falló" antes de pegarle de verdad.
    pub async fn ping(&self) -> Result<(), VozError> {
        match round_trip(&self.path, &Request::Ping).await? {
            Response::Pong => Ok(()),
            other => Err(inesperado(other)),
        }
    }
}

/// Mapea una respuesta fuera de contrato a un `VozError`.
fn inesperado(r: Response) -> VozError {
    match r {
        Response::Error(e) => VozError::Stt(e),
        _ => VozError::Stt("respuesta del daemon de voz inesperada".into()),
    }
}

/// Una transmisión del request en un socket recién abierto. `Ok(None)` cuando
/// el peer cerró antes de responder (transitorio — el caller reintenta).
async fn intentar(path: &Path, req: &Request) -> Result<Option<Response>, VozError> {
    let mut stream = match transport::connect(path).await {
        Ok(s) => s,
        Err(e) if es_transitorio(&e) => return Ok(None),
        Err(e) => return Err(VozError::Stt(format!("conexión al daemon de voz: {e}"))),
    };
    if let Err(e) = write_frame(&mut stream, req).await {
        if es_transitorio(&e) {
            return Ok(None);
        }
        return Err(VozError::Stt(format!("envío al daemon de voz: {e}")));
    }
    match read_frame::<_, Response>(&mut stream).await {
        Ok(Some(resp)) => Ok(Some(resp)),
        Ok(None) => Ok(None), // peer cerró antes de responder: transitorio
        Err(e) if es_transitorio(&e) => Ok(None),
        Err(e) => Err(VozError::Stt(format!("lectura del daemon de voz: {e}"))),
    }
}

/// Un round-trip completo con un reintento corto ante causas transitorias
/// (daemon reiniciando, conexión cortada antes de responder).
async fn round_trip(path: &Path, req: &Request) -> Result<Response, VozError> {
    if let Some(resp) = intentar(path, req).await? {
        return Ok(resp);
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    match intentar(path, req).await? {
        Some(resp) => Ok(resp),
        None => Err(VozError::Stt(
            "el daemon de voz no respondió tras un reintento (¿caído?)".into(),
        )),
    }
}

/// ¿El error pinta a "el daemon se cayó / reinició" en vez de un fallo duro?
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
impl Transcriptor for DaemonClient {
    fn modelo(&self) -> &str {
        &self.modelo_stt
    }

    async fn transcribir(&self, audio: &Audio) -> Result<Transcripcion, VozError> {
        match round_trip(&self.path, &Request::Transcribir(audio.clone())).await? {
            Response::Transcripcion(t) => Ok(t),
            Response::Error(e) => Err(VozError::Stt(e)),
            other => Err(inesperado(other)),
        }
    }
}

#[async_trait]
impl Locutor for DaemonClient {
    fn modelo(&self) -> &str {
        &self.modelo_tts
    }

    async fn sintetizar(&self, texto: &str) -> Result<Audio, VozError> {
        match round_trip(&self.path, &Request::Sintetizar(texto.to_string())).await? {
            Response::Audio(a) => Ok(a),
            Response::Error(e) => Err(VozError::Tts(e)),
            other => Err(inesperado(other)),
        }
    }
}
