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
    /// Reproductor de video (`nahual-video-viewer-llimphi`); abre
    /// WebM/MKV (AV1) e IVF con el decoder nativo puro-Rust.
    Video,
    /// Reproductor de audio (`nahual-audio-viewer-llimphi`); WAV/MP3/
    /// FLAC/Opus/Vorbis por cpal, con espectro en vivo.
    Audio,
    /// Visor estructurado de Cards (`nahual-card-viewer-llimphi`); pinta
    /// los campos de una `shared/card` en vez del JSON crudo.
    Card,
    /// Visor de árbol JSON/TOML (`nahual-tree-viewer-llimphi`); indenta
    /// la estructura, legible aun para JSON minificado.
    Tree,
    /// Volcado hex/ASCII (`nahual-hex-viewer-llimphi`) para binarios que
    /// shuma reconoce pero no tienen visor propio (ELF/wasm/gzip/zip).
    Hex,
    /// Tabla CSV/TSV (`nahual-table-viewer-llimphi`); columnas alineadas.
    Table,
    /// Markdown renderizado (`nahual-markdown-viewer-llimphi`); encabezados,
    /// listas, código y citas con estilo en vez de la sintaxis cruda.
    Markdown,
    /// Listado de un ZIP (`nahual-archive-viewer-llimphi`); muestra las
    /// entradas (nombre/tamaño/ratio) en vez del volcado hex. Cubre la
    /// familia ZIP entera: .jar/.apk/.epub/.docx/.xlsx/.pptx.
    Archive,
    /// Visor de texto (`nahual-text-viewer-llimphi`); degrada a "binario"
    /// si el contenido no es UTF-8. Es el fallback universal.
    Text,
}

/// Elige el visor para un discernimiento. La regla, en orden:
///
/// 1. Si el `lens` lo dice explícitamente (`gallery` → imagen,
///    `video` → reproductor, `audio` → audio, `card` → visor de cards,
///    `tree` → árbol JSON/TOML).
/// 2. Si el `mime` arranca con `image/`, `video/` o `audio/` (cubre
///    formatos que magic-bytes detecta sin asignar lens).
/// 3. Fallback a texto — el visor que nunca falla feo.
///
/// Un `None` (no se pudo discernir, p.ej. archivo ilegible) cae a texto.
pub fn pick(discernment: Option<&Discernment>) -> ViewerKind {
    let Some(d) = discernment else {
        return ViewerKind::Text;
    };
    // GIF: shuma lo marca `gallery` (es una imagen), pero un GIF animado
    // se ve mejor reproducido. El video viewer acepta su `FrameSource` y
    // lo anima en loop; un GIF de un solo frame se ve igual que estático.
    if d.mime.as_deref() == Some("image/gif") {
        return ViewerKind::Video;
    }
    match d.lens.as_deref() {
        Some("gallery") => return ViewerKind::Image,
        Some("video") => return ViewerKind::Video,
        Some("audio") => return ViewerKind::Audio,
        Some("card") => return ViewerKind::Card,
        Some("tree") => return ViewerKind::Tree,
        Some("table") => return ViewerKind::Table,
        Some("markdown") => return ViewerKind::Markdown,
        _ => {}
    }
    match d.mime.as_deref() {
        Some(m) if m.starts_with("image/") => return ViewerKind::Image,
        Some(m) if m.starts_with("video/") => return ViewerKind::Video,
        Some(m) if m.starts_with("audio/") => return ViewerKind::Audio,
        // Binarios que shuma detecta por magic-bytes (sin lens) pero que
        // ningún visor "rico" cubre: un dump hex es mejor que el text
        // viewer diciendo "(binario — sin preview)".
        // Un ZIP es un contenedor: listamos sus entradas en vez del hex.
        // Cubre .zip y toda la familia basada en ZIP (jar/apk/epub/OOXML).
        Some("application/zip") => return ViewerKind::Archive,
        Some(
            "application/x-executable"
            | "application/wasm"
            | "application/gzip",
        ) => return ViewerKind::Hex,
        _ => {}
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
    fn gif_va_a_video_aunque_sea_gallery() {
        // shuma marca el GIF como gallery; igual lo anima el video viewer.
        assert_eq!(pick(Some(&disc(Some("gallery"), Some("image/gif")))), ViewerKind::Video);
    }

    #[test]
    fn video_lens_va_a_video() {
        assert_eq!(pick(Some(&disc(Some("video"), Some("video/webm")))), ViewerKind::Video);
    }

    #[test]
    fn mime_video_sin_lens_va_a_video() {
        assert_eq!(pick(Some(&disc(None, Some("video/x-ivf")))), ViewerKind::Video);
    }

    #[test]
    fn audio_lens_y_mime_van_a_audio() {
        assert_eq!(pick(Some(&disc(Some("audio"), Some("audio/wav")))), ViewerKind::Audio);
        assert_eq!(pick(Some(&disc(None, Some("audio/mpeg")))), ViewerKind::Audio);
    }

    #[test]
    fn card_lens_va_a_card() {
        assert_eq!(pick(Some(&disc(Some("card"), Some("application/json")))), ViewerKind::Card);
    }

    #[test]
    fn tree_lens_va_a_tree() {
        assert_eq!(pick(Some(&disc(Some("tree"), Some("application/json")))), ViewerKind::Tree);
        assert_eq!(pick(Some(&disc(Some("tree"), Some("application/toml")))), ViewerKind::Tree);
    }

    #[test]
    fn table_lens_va_a_table() {
        assert_eq!(pick(Some(&disc(Some("table"), Some("text/csv")))), ViewerKind::Table);
    }

    #[test]
    fn binarios_van_a_hex() {
        assert_eq!(pick(Some(&disc(None, Some("application/x-executable")))), ViewerKind::Hex);
        assert_eq!(pick(Some(&disc(None, Some("application/wasm")))), ViewerKind::Hex);
        assert_eq!(pick(Some(&disc(None, Some("application/gzip")))), ViewerKind::Hex);
    }

    #[test]
    fn zip_va_a_archive() {
        assert_eq!(pick(Some(&disc(None, Some("application/zip")))), ViewerKind::Archive);
    }

    #[test]
    fn markdown_va_a_markdown() {
        assert_eq!(pick(Some(&disc(Some("markdown"), Some("text/plain")))), ViewerKind::Markdown);
    }

    #[test]
    fn code_va_a_texto() {
        assert_eq!(pick(Some(&disc(Some("code"), Some("text/plain")))), ViewerKind::Text);
    }

    #[test]
    fn sin_discernimiento_cae_a_texto() {
        assert_eq!(pick(None), ViewerKind::Text);
    }
}
