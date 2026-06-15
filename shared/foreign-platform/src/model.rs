//! Tipos de dominio **agnósticos**: el vocabulario común con el que el
//! frontend (un futuro cliente tipo FreeTube en Llimphi) habla, sin saber si
//! detrás hay Invidious, PeerTube, Piped o lo que sea. Todo proveedor traduce
//! su JSON propio a estos tipos (ver [`crate::rest`]).

use serde::{Deserialize, Serialize};

/// Una tarjeta de video: lo que aparece en una grilla de resultados, en la
/// página de un canal o en trending. Es el denominador común de toda
/// plataforma; campos opcionales porque no todas exponen lo mismo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VideoCard {
    /// Identificador del video dentro de la plataforma (lo que después se le
    /// pasa a [`crate::PlatformProvider::video`] para resolver el stream).
    pub id: String,
    /// Título del video.
    pub title: String,
    /// Nombre del autor/canal.
    pub author: Option<String>,
    /// Identificador del canal (para abrir su página).
    pub channel_id: Option<String>,
    /// Duración en segundos.
    pub duration_secs: Option<u64>,
    /// Vistas acumuladas.
    pub views: Option<u64>,
    /// Fecha de publicación, como la entregue la plataforma (texto libre o
    /// timestamp; el frontend la normaliza si quiere).
    pub published: Option<String>,
    /// URL de la miniatura.
    pub thumbnail: Option<String>,
    /// Descripción corta, si vino en el listado.
    pub description: Option<String>,
}

/// Un stream resoluble: el resultado de pedir el video concreto. Espeja la
/// forma de [`foreign_ytdlp::Resolved`](../foreign_ytdlp): una URL de video
/// (muxeada si `audio_url` es `None`) más, en DASH, una URL de audio aparte.
/// Lo consume el decoder de red de `media` (R1/R2 de PARIDAD.md).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamSet {
    /// URL de stream de video (o muxeado audio+video si `audio_url` es `None`).
    pub video_url: String,
    /// URL de stream de **audio** separada (DASH); `None` ⇒ `video_url` ya es
    /// muxeado.
    pub audio_url: Option<String>,
    /// Etiqueta de calidad si la plataforma la informó (p. ej. "1080p").
    pub quality: Option<String>,
}

/// El detalle de un video: su tarjeta más el stream resuelto.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VideoDetail {
    /// Metadatos del video.
    pub card: VideoCard,
    /// Stream resuelto, listo para `media`.
    pub stream: StreamSet,
}

/// Parámetros de una búsqueda. Minimal por ahora: query + paginación.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchQuery {
    /// Texto a buscar.
    pub text: String,
    /// Página (1-based); las plataformas REST suelen paginar así.
    pub page: u32,
}

impl SearchQuery {
    /// Atajo para una búsqueda de la primera página.
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), page: 1 }
    }
}
