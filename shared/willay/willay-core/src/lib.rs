//! `willay-core` — el esquema agnóstico del **centro de eventos**.
//!
//! Un [`Evento`] es la entrada liviana del índice federado: identidad
//! direccionada por contenido (BLAKE3), cuándo pasó, de qué clase es, quién lo
//! emitió, un texto para buscar/embeber, y una **referencia** ([`Payload`]) al
//! dato pesado — que se queda en su productor (el PNG en hapiy, la notificación
//! completa en el sled de `pata-notify`). willay no centraliza payloads grandes.
//!
//! Es `#![no_std]` sobre `alloc`: el `Evento` es direccionado por contenido, así
//! que por la ley de Wawa (todo lo que vive en disco por hash o cruza a el
//! kernel compila sin std) este esquema se mantiene wawa-compatible. El índice
//! `willay-store` (sled) sí es std, pero no cruza la frontera. Ver
//! `shared/willay/SDD.md`.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

use alloc::string::String;
use core::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// Identidad de un evento: BLAKE3 (32 bytes) sobre su contenido canónico. El
/// mismo contenido produce el mismo id — re-emitir un evento idéntico no lo
/// duplica en el índice (dedup natural).
pub type EventoId = [u8; 32];

/// La clase de un evento — la faceta primaria por la que el centro lo agrupa y
/// filtra. v1 cubre las tres que el usuario nombró; las clases v2 (correo,
/// unidad, sistema, nota…) se agregan acá sin tocar el resto del esquema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Clase {
    /// Notificación de escritorio (freedesktop) — espejo compacto de la que
    /// `pata-notify` guarda entera en su propio store.
    Notificacion,
    /// Captura de pantalla (hapiy): el payload referencia el PNG en disco.
    Captura,
    /// Entrada del historial de clipboard: texto inline, o un archivo si es
    /// imagen/recorte.
    Clip,
}

impl Clase {
    /// Slug estable en minúsculas — para claves, filtros por URL/CLI y rótulos.
    pub fn slug(self) -> &'static str {
        match self {
            Clase::Notificacion => "notificacion",
            Clase::Captura => "captura",
            Clase::Clip => "clip",
        }
    }

    /// Byte discriminante estable para el cómputo del id (no depende del orden
    /// de declaración del enum, así reordenar variantes no cambia ids viejos).
    fn tag(self) -> u8 {
        match self {
            Clase::Notificacion => 1,
            Clase::Captura => 2,
            Clase::Clip => 3,
        }
    }

    /// Parsea un [`Self::slug`] de vuelta a la clase. `None` si no matchea.
    pub fn desde_slug(s: &str) -> Option<Self> {
        match s {
            "notificacion" => Some(Clase::Notificacion),
            "captura" => Some(Clase::Captura),
            "clip" => Some(Clase::Clip),
            _ => None,
        }
    }
}

/// Referencia al dato pesado del evento. La federación vive acá: lo grande se
/// queda en su productor y el índice sólo apunta. Lo chico (un clip de texto)
/// puede ir inline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Payload {
    /// Sin payload — el evento se agota en `titulo`/`cuerpo` (p. ej. una notif).
    Nada,
    /// Texto corto inline (un clip de texto, un cuerpo breve).
    Texto(String),
    /// Un archivo en disco — el PNG de una captura, un recorte de clipboard. El
    /// índice guarda la ruta y el mime, **no** los bytes.
    Archivo { ruta: String, mime: String },
}

impl Payload {
    /// Bytes canónicos para el cómputo del id (discriminante + contenido).
    fn hash_en(&self, h: &mut blake3::Hasher) {
        match self {
            Payload::Nada => {
                h.update(&[0]);
            }
            Payload::Texto(t) => {
                h.update(&[1]);
                h.update(t.as_bytes());
            }
            Payload::Archivo { ruta, mime } => {
                h.update(&[2]);
                h.update(ruta.as_bytes());
                h.update(&[0]); // separador, evita colisión ruta/mime concatenados
                h.update(mime.as_bytes());
            }
        }
    }
}

/// Una entrada del índice de eventos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evento {
    /// BLAKE3 del contenido canónico (ver [`Evento::nuevo`]). Estable y dedupable.
    pub id: EventoId,
    /// La faceta primaria.
    pub clase: Clase,
    /// Cuándo pasó: µs desde epoch. Es la clave de orden del timeline.
    pub ts_usec: u64,
    /// Quién lo emitió: `app_name` de la notif, `"hapiy"`, el conector del
    /// monitor capturado, la app que copió al clipboard.
    pub origen: String,
    /// La línea principal (summary de la notif, "Captura DP-1", inicio del clip).
    pub titulo: String,
    /// El texto que se busca/embebe (body de la notif, texto del clip, OCR futuro).
    pub cuerpo: String,
    /// Referencia al dato pesado, o el dato chico inline.
    pub payload: Payload,
}

impl Evento {
    /// Construye un evento computando su `id` por BLAKE3 sobre el contenido
    /// canónico `(clase, ts_usec, origen, titulo, cuerpo, payload)`. Dos eventos
    /// con el mismo contenido obtienen el mismo id (dedup); cambiar cualquier
    /// campo cambia el id.
    pub fn nuevo(
        clase: Clase,
        ts_usec: u64,
        origen: impl Into<String>,
        titulo: impl Into<String>,
        cuerpo: impl Into<String>,
        payload: Payload,
    ) -> Self {
        let origen = origen.into();
        let titulo = titulo.into();
        let cuerpo = cuerpo.into();
        let mut h = blake3::Hasher::new();
        h.update(&[clase.tag()]);
        h.update(&ts_usec.to_be_bytes());
        // Cada campo con su longitud delante, así no hay ambigüedad por
        // concatenación (p. ej. titulo="ab"+cuerpo="c" vs "a"+"bc").
        for campo in [origen.as_str(), titulo.as_str(), cuerpo.as_str()] {
            h.update(&(campo.len() as u64).to_be_bytes());
            h.update(campo.as_bytes());
        }
        payload.hash_en(&mut h);
        let id: EventoId = *h.finalize().as_bytes();
        Self { id, clase, ts_usec, origen, titulo, cuerpo, payload }
    }

    /// El id en hex — para logs, claves textuales y URLs.
    pub fn id_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.id {
            let _ = write!(s, "{b:02x}");
        }
        s
    }

    /// `true` si `aguja` (ya en minúsculas) aparece en título, cuerpo u origen —
    /// el filtro literal del centro. Búsqueda case-insensitive simple.
    pub fn coincide(&self, aguja_min: &str) -> bool {
        if aguja_min.is_empty() {
            return true;
        }
        self.titulo.to_lowercase().contains(aguja_min)
            || self.cuerpo.to_lowercase().contains(aguja_min)
            || self.origen.to_lowercase().contains(aguja_min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ts: u64, titulo: &str) -> Evento {
        Evento::nuevo(Clase::Clip, ts, "test", titulo, "cuerpo", Payload::Nada)
    }

    #[test]
    fn id_estable_para_mismo_contenido() {
        let a = ev(100, "hola");
        let b = ev(100, "hola");
        assert_eq!(a.id, b.id, "mismo contenido ⇒ mismo id (dedup)");
    }

    #[test]
    fn id_cambia_con_cualquier_campo() {
        let base = ev(100, "hola");
        assert_ne!(base.id, ev(101, "hola").id, "cambia el ts");
        assert_ne!(base.id, ev(100, "chau").id, "cambia el título");
        let otra_clase =
            Evento::nuevo(Clase::Captura, 100, "test", "hola", "cuerpo", Payload::Nada);
        assert_ne!(base.id, otra_clase.id, "cambia la clase");
        let otro_payload =
            Evento::nuevo(Clase::Clip, 100, "test", "hola", "cuerpo", Payload::Texto("x".into()));
        assert_ne!(base.id, otro_payload.id, "cambia el payload");
    }

    #[test]
    fn longitud_delante_evita_colision_por_concatenacion() {
        // ("ab","c") y ("a","bc") sólo difieren por dónde corta título/cuerpo.
        let ab_c = Evento::nuevo(Clase::Clip, 1, "o", "ab", "c", Payload::Nada);
        let a_bc = Evento::nuevo(Clase::Clip, 1, "o", "a", "bc", Payload::Nada);
        assert_ne!(ab_c.id, a_bc.id);
    }

    #[test]
    fn slug_round_trip() {
        for c in [Clase::Notificacion, Clase::Captura, Clase::Clip] {
            assert_eq!(Clase::desde_slug(c.slug()), Some(c));
        }
        assert_eq!(Clase::desde_slug("inexistente"), None);
    }

    #[test]
    fn coincide_es_case_insensitive_sobre_los_tres_campos() {
        let e = Evento::nuevo(Clase::Clip, 1, "Firefox", "API Key", "secreto", Payload::Nada);
        assert!(e.coincide("api"));
        assert!(e.coincide("firefox"));
        assert!(e.coincide("secreto"));
        assert!(!e.coincide("ausente"));
        assert!(e.coincide(""), "aguja vacía matchea todo");
    }

    #[test]
    fn id_hex_tiene_64_chars() {
        assert_eq!(ev(1, "x").id_hex().len(), 64);
    }

    #[test]
    fn round_trip_postcard() {
        let e = Evento::nuevo(
            Clase::Captura,
            42,
            "hapiy",
            "Captura DP-1",
            "",
            Payload::Archivo { ruta: "/tmp/x.png".into(), mime: "image/png".into() },
        );
        let bytes = postcard::to_stdvec(&e).unwrap();
        let back: Evento = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(e, back);
    }
}
