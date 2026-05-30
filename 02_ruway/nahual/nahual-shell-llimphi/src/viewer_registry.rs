//! Registro de visores — el germen del "open-with universal".
//!
//! Toma el [`Discernment`] que `shuma-discern` produce sobre una muestra
//! del archivo (detección por **contenido**, no por extensión) y lo
//! despacha al [`ViewerKind`] que sabe pintar esa naturaleza de dato.
//!
//! Hoy el shell embebe sólo dos visores (texto / imagen), así que la tabla
//! es corta; pero la forma es la correcta: cuando lleguen más visores
//! (database, deck, reader nativo de PDF, card) se agregan filas acá sin
//! tocar el resto del shell. La decisión deja de vivir en un `match ext`
//! y pasa a ser una función de la semántica discernida — el `lens` que ya
//! comparten chasqui (`dominant_lens`) y shuma-discern.
//!
//! Cuando exista un `AppBus` con `EntityType` y visores fuera de proceso,
//! este registro se vuelve su tabla de ruteo: `lens`/`mime` → handler.

use shuma_discern::Discernment;

/// Qué visor del shell pinta el panel derecho.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerKind {
    /// Visor de imágenes (`nahual-image-viewer-llimphi`).
    Image,
    /// Visor de texto (`nahual-text-viewer-llimphi`); degrada a "binario"
    /// si el contenido no es UTF-8. Es el fallback universal.
    Text,
}

/// Elige el visor para un discernimiento. La regla, en orden:
///
/// 1. Si el `lens` lo dice explícitamente (`gallery` → imagen).
/// 2. Si el `mime` arranca con `image/` (cubre formatos que magic-bytes
///    detecta sin asignar lens).
/// 3. Fallback a texto — el visor que nunca falla feo.
///
/// Un `None` (no se pudo discernir, p.ej. archivo ilegible) cae a texto.
pub fn pick(discernment: Option<&Discernment>) -> ViewerKind {
    let Some(d) = discernment else {
        return ViewerKind::Text;
    };
    if matches!(d.lens.as_deref(), Some("gallery")) {
        return ViewerKind::Image;
    }
    if d
        .mime
        .as_deref()
        .is_some_and(|m| m.starts_with("image/"))
    {
        return ViewerKind::Image;
    }
    ViewerKind::Text
}

#[cfg(test)]
mod tests {
    use super::*;
    use card_core::TypeRef;

    fn disc(lens: Option<&str>, mime: Option<&str>) -> Discernment {
        Discernment {
            ty: TypeRef::Primitive { name: "x".into() },
            confidence: 0.9,
            mime: mime.map(String::from),
            lens: lens.map(String::from),
        }
    }

    #[test]
    fn gallery_lens_va_a_imagen() {
        assert_eq!(pick(Some(&disc(Some("gallery"), Some("image/png")))), ViewerKind::Image);
    }

    #[test]
    fn mime_image_sin_lens_va_a_imagen() {
        assert_eq!(pick(Some(&disc(None, Some("image/webp")))), ViewerKind::Image);
    }

    #[test]
    fn markdown_y_code_van_a_texto() {
        assert_eq!(pick(Some(&disc(Some("markdown"), Some("text/plain")))), ViewerKind::Text);
        assert_eq!(pick(Some(&disc(Some("code"), Some("text/plain")))), ViewerKind::Text);
    }

    #[test]
    fn sin_discernimiento_cae_a_texto() {
        assert_eq!(pick(None), ViewerKind::Text);
    }
}
