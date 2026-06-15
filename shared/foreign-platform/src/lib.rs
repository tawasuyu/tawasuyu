//! `foreign-platform` — puente **agnóstico** a plataformas de video.
//!
//! Dos capas, según la decisión de diseño (ver `DECISION.md`):
//!
//! 1. **Core agnóstico** ([`PlatformProvider`] + [`model`]): el frontend habla
//!    sólo con esto y no sabe quién hay detrás (regla #2 del repo).
//! 2. **Motor data-driven** ([`rest::RestProvider`]): interpreta un descriptor
//!    de texto RON que describe una API REST limpia. Sumar un proveedor o una
//!    instancia compatible (Invidious/PeerTube/Piped) = un archivo `.ron`, cero
//!    Rust. Lo que no se datifica (YouTube Innertube) no entra acá: se enruta
//!    por Invidious o por [`foreign-ytdlp`](../foreign_ytdlp).
//!
//! Es la capa de **navegación/descubrimiento** (buscar, canales, trending) que
//! un cliente tipo FreeTube monta encima del reproductor `media`. La
//! reproducción del stream resuelto ya la cubren R1/R2 de PARIDAD.md.
//!
//! ```no_run
//! use foreign_platform::{descriptors, rest::RestProvider, model::SearchQuery, PlatformProvider};
//!
//! let prov = descriptors::invidious("https://invidious.example");
//! let resultados = prov.search(&SearchQuery::new("blender open movie"))?;
//! for v in resultados { println!("{} — {}", v.title, v.id); }
//! # Ok::<(), foreign_platform::PlatformError>(())
//! ```

pub mod json;
pub mod model;
pub mod provider;
pub mod rest;

pub use provider::{PlatformError, PlatformProvider, PlatformResult};

/// Descriptores de proveedores embebidos en el binario y constructores cómodos.
/// Cada uno es un archivo de texto RON: ESA es la prueba de que sumar un
/// proveedor REST no requiere código nuevo.
pub mod descriptors {
    use crate::rest::{RestDescriptor, RestProvider};

    /// Descriptor de Invidious (familia YouTube-via-proxy, API `/api/v1`).
    pub const INVIDIOUS_RON: &str = include_str!("descriptors/invidious.ron");
    /// Descriptor de PeerTube (federado, ActivityPub, API `/api/v1`).
    pub const PEERTUBE_RON: &str = include_str!("descriptors/peertube.ron");

    /// Parsea el descriptor de Invidious (panic si el `.ron` embebido está
    /// roto — es un bug de compilación, no de runtime).
    pub fn invidious_descriptor() -> RestDescriptor {
        RestDescriptor::from_ron(INVIDIOUS_RON).expect("descriptor invidious.ron inválido")
    }

    /// Parsea el descriptor de PeerTube.
    pub fn peertube_descriptor() -> RestDescriptor {
        RestDescriptor::from_ron(PEERTUBE_RON).expect("descriptor peertube.ron inválido")
    }

    /// Proveedor Invidious sobre la instancia `base` (vacío ⇒ default).
    pub fn invidious(base: impl Into<String>) -> RestProvider {
        RestProvider::new(invidious_descriptor(), base)
    }

    /// Proveedor PeerTube sobre la instancia `base` (vacío ⇒ default).
    pub fn peertube(base: impl Into<String>) -> RestProvider {
        RestProvider::new(peertube_descriptor(), base)
    }
}

#[cfg(test)]
mod tests {
    //! Tests sobre **fixtures**: validan el camino data-driven (descriptor →
    //! mapeo) sin tocar la red, igual que los núcleos agnósticos del repo.

    use crate::descriptors;
    use crate::model::SearchQuery;
    use crate::provider::{PlatformProvider, PlatformResult};
    use crate::rest::{HttpFetch, RestProvider};
    use serde_json::Value;

    /// Fetch falso: devuelve siempre el mismo JSON, ignorando la URL.
    struct StubFetch(Value);
    impl HttpFetch for StubFetch {
        fn get_json(&self, _url: &str) -> PlatformResult<Value> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn los_descriptores_embebidos_parsean() {
        // Si un .ron embebido está roto, esto truena al construir.
        let _ = descriptors::invidious_descriptor();
        let _ = descriptors::peertube_descriptor();
    }

    #[test]
    fn invidious_mapea_search_desde_fixture() {
        // Forma real de /api/v1/search de Invidious: array en la raíz.
        let fixture = serde_json::json!([
            {
                "type": "video",
                "videoId": "aqz-KE-bpKQ",
                "title": "Big Buck Bunny",
                "author": "Blender",
                "authorId": "UCSMOQeBJ2RAnuFungnQOxLg",
                "lengthSeconds": 635,
                "viewCount": 1234567,
                "publishedText": "10 years ago",
                "videoThumbnails": [{ "url": "https://inv/t.jpg", "quality": "high" }]
            },
            {
                "type": "video",
                "videoId": "second",
                "title": "Sintel",
                "author": "Blender",
                "lengthSeconds": 888,
                "viewCount": "42",
                "videoThumbnails": [{ "url": "https://inv/s.jpg" }]
            }
        ]);
        let prov = RestProvider::with_fetch(
            descriptors::invidious_descriptor(),
            "https://inv.example",
            StubFetch(fixture),
        );
        let res = prov.search(&SearchQuery::new("blender")).unwrap();
        assert_eq!(res.len(), 2);
        let a = &res[0];
        assert_eq!(a.id, "aqz-KE-bpKQ");
        assert_eq!(a.title, "Big Buck Bunny");
        assert_eq!(a.author.as_deref(), Some("Blender"));
        assert_eq!(a.channel_id.as_deref(), Some("UCSMOQeBJ2RAnuFungnQOxLg"));
        assert_eq!(a.duration_secs, Some(635));
        assert_eq!(a.views, Some(1_234_567));
        assert_eq!(a.thumbnail.as_deref(), Some("https://inv/t.jpg"));
        // segundo item con viewCount como string numérica:
        assert_eq!(res[1].views, Some(42));
    }

    #[test]
    fn invidious_resuelve_stream_muxeado() {
        // /api/v1/videos/{id}: objeto con formatStreams (muxeado).
        let fixture = serde_json::json!({
            "videoId": "x",
            "title": "Demo",
            "formatStreams": [
                { "url": "https://inv/muxed.mp4", "qualityLabel": "720p" }
            ],
            "adaptiveFormats": [
                { "url": "https://inv/video.mp4", "qualityLabel": "1080p" }
            ]
        });
        let prov = RestProvider::with_fetch(
            descriptors::invidious_descriptor(),
            "https://inv.example",
            StubFetch(fixture),
        );
        let d = prov.video("x").unwrap();
        assert_eq!(d.card.title, "Demo");
        // prefiere el muxeado:
        assert_eq!(d.stream.video_url, "https://inv/muxed.mp4");
        assert_eq!(d.stream.audio_url, None);
        assert_eq!(d.stream.quality.as_deref(), Some("720p"));
    }

    #[test]
    fn peertube_mapea_search_con_array_anidado() {
        // PeerTube anida la lista en "data" y usa "name"/"uuid".
        let fixture = serde_json::json!({
            "total": 1,
            "data": [
                {
                    "uuid": "9c9de5e8-0a1e-484a-b099-e80766180a6d",
                    "name": "Federated clip",
                    "duration": 300,
                    "views": 17,
                    "account": { "displayName": "Alice", "name": "alice" },
                    "thumbnailPath": "/static/thumbnails/abc.jpg"
                }
            ]
        });
        let prov = RestProvider::with_fetch(
            descriptors::peertube_descriptor(),
            "https://peertube.example",
            StubFetch(fixture),
        );
        let res = prov.search(&SearchQuery::new("clip")).unwrap();
        assert_eq!(res.len(), 1);
        let v = &res[0];
        assert_eq!(v.id, "9c9de5e8-0a1e-484a-b099-e80766180a6d");
        assert_eq!(v.title, "Federated clip");
        assert_eq!(v.author.as_deref(), Some("Alice"));
        assert_eq!(v.duration_secs, Some(300));
        assert_eq!(v.thumbnail.as_deref(), Some("/static/thumbnails/abc.jpg"));
    }
}
