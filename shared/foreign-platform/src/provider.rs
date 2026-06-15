//! El **core agnóstico**: el trait [`PlatformProvider`] que el frontend usa y
//! el error común. Cualquier backend (data-driven REST o, el día que haga
//! falta, uno escrito a mano) implementa esto; el frontend nunca sabe cuál.

use crate::model::{SearchQuery, VideoCard, VideoDetail};

/// Qué puede salir mal al hablar con una plataforma.
#[derive(Debug)]
pub enum PlatformError {
    /// Falló la red / el transporte HTTP (instancia caída, DNS, TLS…).
    Network(String),
    /// La respuesta no era el JSON esperado (no parseó).
    Parse(String),
    /// El JSON parseó pero faltó un campo obligatorio según el descriptor
    /// (típico cuando la API cambió o el descriptor quedó viejo).
    Mapping(String),
    /// El proveedor no implementa esta operación (el descriptor no la define).
    Unsupported(&'static str),
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformError::Network(e) => write!(f, "red de plataforma: {e}"),
            PlatformError::Parse(e) => write!(f, "respuesta no parseó: {e}"),
            PlatformError::Mapping(e) => write!(f, "campo ausente al mapear: {e}"),
            PlatformError::Unsupported(op) => {
                write!(f, "el proveedor no soporta la operación «{op}»")
            }
        }
    }
}

impl std::error::Error for PlatformError {}

/// Resultado de toda operación de plataforma.
pub type PlatformResult<T> = Result<T, PlatformError>;

/// Una plataforma de video, vista de forma agnóstica. El frontend Llimphi
/// habla SOLO con este trait (regla #2 del repo). Las implementaciones viven
/// detrás del puente (`foreign-*`, regla #4); hoy la principal es el motor
/// data-driven [`RestProvider`](crate::rest::RestProvider).
///
/// Síncrono a propósito, igual que [`foreign-ytdlp`](../foreign_ytdlp): el
/// frontend lo invoca desde un worker (`Handle::spawn`) y reentra al `update`
/// con el resultado, sin arrastrar tokio al núcleo.
pub trait PlatformProvider {
    /// Nombre legible del proveedor (p. ej. "Invidious", "PeerTube").
    fn name(&self) -> &str;

    /// Busca videos. Devuelve una página de tarjetas.
    fn search(&self, query: &SearchQuery) -> PlatformResult<Vec<VideoCard>>;

    /// Videos en tendencia / portada de la instancia.
    fn trending(&self) -> PlatformResult<Vec<VideoCard>>;

    /// Videos de un canal, por su `channel_id`.
    fn channel_videos(&self, channel_id: &str) -> PlatformResult<Vec<VideoCard>>;

    /// Detalle de un video (metadatos + stream resuelto), por su `id`.
    fn video(&self, id: &str) -> PlatformResult<VideoDetail>;
}
