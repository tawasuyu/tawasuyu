//! Clipboard por zona: cada escritorio virtual ("zona") tiene su propio
//! portapapeles, de modo que lo copiado en la zona "código" no lo lee una app de
//! la zona "comunicación". mirada (que ya es el broker del clipboard) guarda por
//! zona el último texto copiado y, al cambiar de escritorio, vuelve a ofrecerlo
//! como una selección **server-side** (`set_data_device_selection`).
//!
//! Este módulo es el **almacén puro** (zona → contenido) más los helpers de mime;
//! el data-plane (leer la selección del cliente por un pipe en `new_selection`,
//! re-ofrecerla en `cambiar_workspace`, servir los bytes en `send_selection`) lo
//! cablea `App` sobre smithay. Sólo se maneja **texto** — el caso de la regla
//! ("código" vs "comunicación"); selecciones binarias (imágenes) se ignoran.
//!
//! **Sin verificar headless** (norma de mirada): la lógica del almacén está
//! testeada; el comportamiento Wayland se valida en sesión gráfica. Va detrás del
//! flag `MIRADA_CLIPBOARD_POR_ZONA` (apagado por defecto).

use std::collections::HashMap;

/// Una selección de texto capturada: los mime types de texto que ofrecía y los
/// bytes (un único contenido textual que sirve para cualquiera de esos mimes).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipContent {
    /// Mimes de texto a anunciar al re-ofrecer la selección (todos respaldados
    /// por los mismos `bytes`).
    pub mime_types: Vec<String>,
    /// El texto copiado, tal cual (UTF-8, sin transformar).
    pub bytes: Vec<u8>,
}

/// Almacén de portapapeles por zona (índice de escritorio).
#[derive(Debug, Default)]
pub struct ZoneClipboard {
    per_zone: HashMap<usize, ClipContent>,
}

impl ZoneClipboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra lo copiado en `zone`. `None` o un contenido vacío **borra** la
    /// zona (la selección se vació): al volver a ella, el portapapeles queda
    /// limpio en vez de mostrar algo viejo.
    pub fn record(&mut self, zone: usize, content: Option<ClipContent>) {
        match content {
            Some(c) if !c.bytes.is_empty() => {
                self.per_zone.insert(zone, c);
            }
            _ => {
                self.per_zone.remove(&zone);
            }
        }
    }

    /// El contenido a re-ofrecer al entrar a `zone`, o `None` si esa zona nunca
    /// copió nada (el compositor entonces limpia la selección).
    pub fn for_zone(&self, zone: usize) -> Option<&ClipContent> {
        self.per_zone.get(&zone)
    }
}

/// `true` si `mime` es un tipo de **texto** que tiene sentido particionar por
/// zona. Cubre `text/*` y los alias clásicos de X (`UTF8_STRING`, `STRING`,
/// `TEXT`). Lo binario (imágenes, etc.) se deja pasar sin tocar.
pub fn is_text_mime(mime: &str) -> bool {
    let m = mime.to_ascii_uppercase();
    mime.starts_with("text/") || matches!(m.as_str(), "UTF8_STRING" | "STRING" | "TEXT")
}

/// Elige el mejor mime de **texto** para leer la selección de un cliente, entre
/// los que ofrece. Prefiere UTF-8 explícito, luego `text/plain`, luego cualquier
/// texto. `None` si no ofrece texto (selección binaria: no se particiona).
pub fn pick_text_mime(mimes: &[String]) -> Option<String> {
    let pref = |needle: &str| {
        mimes
            .iter()
            .find(|m| m.eq_ignore_ascii_case(needle))
            .cloned()
    };
    pref("text/plain;charset=utf-8")
        .or_else(|| pref("UTF8_STRING"))
        .or_else(|| pref("text/plain"))
        .or_else(|| mimes.iter().find(|m| is_text_mime(m)).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(text: &str) -> ClipContent {
        ClipContent {
            mime_types: vec!["text/plain;charset=utf-8".into()],
            bytes: text.as_bytes().to_vec(),
        }
    }

    #[test]
    fn zonas_aisladas() {
        let mut zc = ZoneClipboard::new();
        zc.record(0, Some(clip("código")));
        zc.record(2, Some(clip("comunicación")));
        assert_eq!(zc.for_zone(0).unwrap().bytes, "código".as_bytes());
        assert_eq!(zc.for_zone(2).unwrap().bytes, "comunicación".as_bytes());
        // Una zona sin copias nunca ve la de otra.
        assert!(zc.for_zone(1).is_none());
    }

    #[test]
    fn record_vacio_borra_la_zona() {
        let mut zc = ZoneClipboard::new();
        zc.record(0, Some(clip("algo")));
        assert!(zc.for_zone(0).is_some());
        zc.record(0, None);
        assert!(zc.for_zone(0).is_none(), "None vacía la zona");
        zc.record(0, Some(clip("x")));
        zc.record(0, Some(ClipContent { mime_types: vec![], bytes: vec![] }));
        assert!(zc.for_zone(0).is_none(), "bytes vacíos también vacían");
    }

    #[test]
    fn pick_text_mime_prefiere_utf8() {
        let mimes = vec![
            "text/plain".to_string(),
            "text/plain;charset=utf-8".to_string(),
            "image/png".to_string(),
        ];
        assert_eq!(pick_text_mime(&mimes).as_deref(), Some("text/plain;charset=utf-8"));
        // Sólo binario: no se particiona.
        assert_eq!(pick_text_mime(&["image/png".to_string()]), None);
        // Alias de X cuando no hay text/*.
        assert_eq!(
            pick_text_mime(&["UTF8_STRING".to_string()]).as_deref(),
            Some("UTF8_STRING")
        );
    }

    #[test]
    fn is_text_mime_reconoce_texto_y_alias() {
        assert!(is_text_mime("text/plain"));
        assert!(is_text_mime("text/html"));
        assert!(is_text_mime("UTF8_STRING"));
        assert!(!is_text_mime("image/png"));
        assert!(!is_text_mime("application/octet-stream"));
    }
}
