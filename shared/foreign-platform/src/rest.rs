//! El **motor data-driven**: un descriptor de texto (RON) describe una API
//! REST limpia (Invidious, PeerTube, Piped) y [`RestProvider`] lo interpreta
//! para cumplir el trait [`PlatformProvider`](crate::PlatformProvider). El día
//! que querés sumar un proveedor compatible NO escribís Rust: escribís un
//! `.ron` (ver `descriptors/`). Esa es la apuesta de este crate.
//!
//! Lo que el descriptor NO puede expresar (descifrado de firmas, challenges
//! JS móviles tipo YouTube Innertube) queda explícitamente fuera: para eso se
//! enruta por una instancia Invidious (que sí es REST) o por `foreign-ytdlp`.

use serde::Deserialize;
use serde_json::Value;

use crate::json;
use crate::model::{SearchQuery, StreamSet, VideoCard, VideoDetail};
use crate::provider::{PlatformError, PlatformProvider, PlatformResult};

// ===========================================================================
//  Descriptor — el "script de texto" deserializable desde RON
// ===========================================================================

/// La descripción completa de un proveedor REST. Se deserializa de un `.ron`.
#[derive(Debug, Clone, Deserialize)]
pub struct RestDescriptor {
    /// Nombre legible ("Invidious", "PeerTube").
    pub name: String,
    /// Familia de API (informativo / para agrupar instancias compatibles).
    pub kind: String,
    /// Instancias por defecto sugeridas (la primera se usa si no se da otra).
    #[serde(default)]
    pub default_instances: Vec<String>,
    /// Endpoints soportados. Los ausentes ⇒ operación no soportada.
    pub endpoints: Endpoints,
}

/// El conjunto de endpoints que un proveedor puede declarar.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Endpoints {
    pub search: Option<ListEndpoint>,
    pub trending: Option<ListEndpoint>,
    pub channel_videos: Option<ListEndpoint>,
    pub video: Option<VideoEndpoint>,
}

/// Un endpoint que devuelve una **lista** de tarjetas (search/trending/canal).
#[derive(Debug, Clone, Deserialize)]
pub struct ListEndpoint {
    /// Path relativo a la instancia. Admite placeholders `{query}`, `{page}`,
    /// `{channel}` (se sustituyen URL-encodeados).
    pub path: String,
    /// Query string como pares clave→valor; los valores admiten los mismos
    /// placeholders.
    #[serde(default)]
    pub query: Vec<(String, String)>,
    /// Dónde vive el array en la respuesta. `""` ⇒ la raíz ES el array.
    #[serde(default)]
    pub list_path: String,
    /// Cómo mapear cada item del array a un [`VideoCard`].
    pub fields: VideoFields,
}

/// Un endpoint que devuelve **un** video con su stream.
#[derive(Debug, Clone, Deserialize)]
pub struct VideoEndpoint {
    /// Path relativo; admite placeholder `{id}`.
    pub path: String,
    #[serde(default)]
    pub query: Vec<(String, String)>,
    /// Mapeo de metadatos.
    pub fields: VideoFields,
    /// Mapeo de los streams reproducibles.
    pub streams: StreamFields,
}

/// Mapeo campo-de-dominio → path-JSON. Los `Option` ausentes en el descriptor
/// dejan el campo correspondiente del [`VideoCard`] en `None`. `id` y `title`
/// son obligatorios.
#[derive(Debug, Clone, Deserialize)]
pub struct VideoFields {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub duration_secs: Option<String>,
    #[serde(default)]
    pub views: Option<String>,
    #[serde(default)]
    pub published: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Cómo extraer un [`StreamSet`] del JSON de un video. MVP: toma el primer
/// formato muxeado si existe; si no, el primer video + primer audio (DASH).
#[derive(Debug, Clone, Deserialize)]
pub struct StreamFields {
    /// Array de formatos muxeados (audio+video en una URL). Opcional.
    #[serde(default)]
    pub muxed_list: Option<String>,
    /// Array de formatos sólo-video (DASH). Opcional.
    #[serde(default)]
    pub video_list: Option<String>,
    /// Array de formatos sólo-audio (DASH). Opcional.
    #[serde(default)]
    pub audio_list: Option<String>,
    /// Path a la URL dentro de cada item de formato (común a las tres listas).
    pub url: String,
    /// Path a la etiqueta de calidad dentro de cada item (opcional).
    #[serde(default)]
    pub quality: Option<String>,
}

impl RestDescriptor {
    /// Parsea un descriptor desde texto RON.
    pub fn from_ron(src: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(src)
    }
}

// ===========================================================================
//  Transporte HTTP — abstracto para poder testear sin red
// ===========================================================================

/// Abstracción mínima del fetch HTTP. La real es [`UreqFetch`]; los tests
/// inyectan una que devuelve fixtures, así el mapeo se valida en CI sin red
/// (igual que el resto de los núcleos del repo).
pub trait HttpFetch {
    /// GET de una URL completa → JSON parseado.
    fn get_json(&self, url: &str) -> PlatformResult<Value>;
}

/// Fetch real sobre `ureq` (HTTP síncrono, sin tokio).
#[derive(Debug, Clone, Default)]
pub struct UreqFetch;

impl HttpFetch for UreqFetch {
    fn get_json(&self, url: &str) -> PlatformResult<Value> {
        let resp = ureq::get(url)
            .call()
            .map_err(|e| PlatformError::Network(e.to_string()))?;
        // Leemos a texto y parseamos con serde_json: así no dependemos del
        // feature `json` de ureq (el workspace lo trae con default-features off).
        let body = resp
            .into_string()
            .map_err(|e| PlatformError::Network(e.to_string()))?;
        serde_json::from_str::<Value>(&body).map_err(|e| PlatformError::Parse(e.to_string()))
    }
}

// ===========================================================================
//  RestProvider — interpreta el descriptor sobre una instancia concreta
// ===========================================================================

/// Un proveedor concreto: un descriptor + una instancia base + un fetcher.
/// Genérico sobre el fetcher para inyectar fixtures en tests.
#[derive(Debug, Clone)]
pub struct RestProvider<F: HttpFetch = UreqFetch> {
    descriptor: RestDescriptor,
    /// URL base de la instancia, sin barra final (p. ej. "https://inv.example").
    base: String,
    fetch: F,
}

impl RestProvider<UreqFetch> {
    /// Construye un proveedor sobre la instancia `base` (red real). Si `base`
    /// está vacía, usa la primera `default_instances` del descriptor.
    pub fn new(descriptor: RestDescriptor, base: impl Into<String>) -> Self {
        Self::with_fetch(descriptor, base, UreqFetch)
    }
}

impl<F: HttpFetch> RestProvider<F> {
    /// Construye con un fetcher explícito (tests).
    pub fn with_fetch(descriptor: RestDescriptor, base: impl Into<String>, fetch: F) -> Self {
        let mut base = base.into();
        if base.is_empty() {
            base = descriptor
                .default_instances
                .first()
                .cloned()
                .unwrap_or_default();
        }
        let base = base.trim_end_matches('/').to_string();
        Self { descriptor, base, fetch }
    }

    /// Sustituye placeholders `{k}` en `tpl` con `params` (URL-encode simple).
    fn fill(tpl: &str, params: &[(&str, String)]) -> String {
        let mut out = tpl.to_string();
        for (k, v) in params {
            out = out.replace(&format!("{{{k}}}"), &url_encode(v));
        }
        out
    }

    /// Arma la URL completa de un endpoint de lista.
    fn list_url(&self, ep: &ListEndpoint, params: &[(&str, String)]) -> String {
        let path = Self::fill(&ep.path, params);
        let mut url = format!("{}{}", self.base, path);
        if !ep.query.is_empty() {
            let qs: Vec<String> = ep
                .query
                .iter()
                .map(|(k, v)| format!("{}={}", url_encode(k), Self::fill(v, params)))
                .collect();
            url.push('?');
            url.push_str(&qs.join("&"));
        }
        url
    }

    /// Ejecuta un endpoint de lista y mapea cada item a [`VideoCard`].
    fn run_list(&self, ep: &ListEndpoint, params: &[(&str, String)]) -> PlatformResult<Vec<VideoCard>> {
        let url = self.list_url(ep, params);
        let body = self.fetch.get_json(&url)?;
        let arr = json::get_array(&body, &ep.list_path).ok_or_else(|| {
            PlatformError::Mapping(format!("no hay array en «{}»", ep.list_path))
        })?;
        Ok(arr.iter().filter_map(|item| map_card(item, &ep.fields)).collect())
    }
}

/// Mapea un item JSON a [`VideoCard`] según `f`. Devuelve `None` si faltan los
/// campos obligatorios (`id`/`title`) — así un item roto no tumba la página.
fn map_card(item: &Value, f: &VideoFields) -> Option<VideoCard> {
    let id = json::get_string(item, &f.id)?;
    let title = json::get_string(item, &f.title)?;
    Some(VideoCard {
        id,
        title,
        author: f.author.as_deref().and_then(|p| json::get_string(item, p)),
        channel_id: f.channel_id.as_deref().and_then(|p| json::get_string(item, p)),
        duration_secs: f.duration_secs.as_deref().and_then(|p| json::get_u64(item, p)),
        views: f.views.as_deref().and_then(|p| json::get_u64(item, p)),
        published: f.published.as_deref().and_then(|p| json::get_string(item, p)),
        thumbnail: f.thumbnail.as_deref().and_then(|p| json::get_string(item, p)),
        description: f.description.as_deref().and_then(|p| json::get_string(item, p)),
    })
}

/// Extrae el [`StreamSet`] de un video JSON según `s`. Prefiere muxeado;
/// si no, primer video + primer audio (DASH).
fn map_stream(body: &Value, s: &StreamFields) -> Option<StreamSet> {
    let first_url = |list: &Option<String>| -> Option<(String, Option<String>)> {
        let arr = json::get_array(body, list.as_deref()?)?;
        let item = arr.first()?;
        let url = json::get_string(item, &s.url)?;
        let q = s.quality.as_deref().and_then(|p| json::get_string(item, p));
        Some((url, q))
    };
    // 1) muxeado directo
    if let Some((url, quality)) = first_url(&s.muxed_list) {
        return Some(StreamSet { video_url: url, audio_url: None, quality });
    }
    // 2) DASH: video + audio por separado
    let (video_url, quality) = first_url(&s.video_list)?;
    let audio_url = first_url(&s.audio_list).map(|(u, _)| u);
    Some(StreamSet { video_url, audio_url, quality })
}

impl<F: HttpFetch> PlatformProvider for RestProvider<F> {
    fn name(&self) -> &str {
        &self.descriptor.name
    }

    fn search(&self, query: &SearchQuery) -> PlatformResult<Vec<VideoCard>> {
        let ep = self.descriptor.endpoints.search.as_ref().ok_or(PlatformError::Unsupported("search"))?;
        let params = [("query", query.text.clone()), ("page", query.page.to_string())];
        self.run_list(ep, &params)
    }

    fn trending(&self) -> PlatformResult<Vec<VideoCard>> {
        let ep = self.descriptor.endpoints.trending.as_ref().ok_or(PlatformError::Unsupported("trending"))?;
        self.run_list(ep, &[])
    }

    fn channel_videos(&self, channel_id: &str) -> PlatformResult<Vec<VideoCard>> {
        let ep = self.descriptor.endpoints.channel_videos.as_ref().ok_or(PlatformError::Unsupported("channel_videos"))?;
        let params = [("channel", channel_id.to_string())];
        self.run_list(ep, &params)
    }

    fn video(&self, id: &str) -> PlatformResult<VideoDetail> {
        let ep = self.descriptor.endpoints.video.as_ref().ok_or(PlatformError::Unsupported("video"))?;
        let params = [("id", id.to_string())];
        let path = Self::fill(&ep.path, &params);
        let mut url = format!("{}{}", self.base, path);
        if !ep.query.is_empty() {
            let qs: Vec<String> = ep
                .query
                .iter()
                .map(|(k, v)| format!("{}={}", url_encode(k), Self::fill(v, &params)))
                .collect();
            url.push('?');
            url.push_str(&qs.join("&"));
        }
        let body = self.fetch.get_json(&url)?;
        let card = map_card(&body, &ep.fields)
            .ok_or_else(|| PlatformError::Mapping("faltan id/title del video".into()))?;
        let stream = map_stream(&body, &ep.streams)
            .ok_or_else(|| PlatformError::Mapping("no se halló un stream reproducible".into()))?;
        Ok(VideoDetail { card, stream })
    }
}

/// URL-encode mínimo de un componente (RFC 3986 unreserved se deja igual).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
