//! Registro de visores — el "open-with universal", ahora dirigido por datos.
//!
//! Toma el [`Discernment`] que `shuma-discern` produce sobre una muestra
//! del archivo (detección por **contenido**, no por extensión) y lo
//! despacha al [`ViewerKind`] que sabe pintar esa naturaleza de dato.
//!
//! ## La tabla es una Card, no un `match`
//!
//! Cada visor se describe como un [`ViewerCard`]: qué `lens` acepta, qué
//! `mime` (prefijo o exacto) cubre, y con qué [`Priority`] compite. La
//! decisión NO vive en ramas de control sino en [`registry`], una tabla de
//! datos. Agregar un visor = agregar una fila. Esto es la "Capa 2" de
//! Brahman a nivel de UI (ver `/BRAHMAN.md`, Fase 2a): el `lens` es el mismo
//! `presentation_hint` que `card_core::DataFacet` comparte con chasqui
//! (`dominant_lens`) y shuma-discern, y `Priority` es el mismo tiebreaker
//! que usa `chasqui-broker` para rankear productores.
//!
//! ## La costura hacia el AppBus
//!
//! Hoy [`registry`] devuelve una tabla estática y los visores viven
//! in-process. Cuando exista el AppBus (visores fuera de proceso), esta
//! función pasa a poblarse desde el broker / `card-discovery`: cada visor
//! publica una `Card` con su `(lens, mime, priority)` y `pick` rankea
//! exactamente igual. El algoritmo de matching no cambia — sólo el origen
//! de las filas.

use card_core::Priority;
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
    /// Listado de un archivo comprimido (`nahual-archive-viewer-llimphi`);
    /// muestra las entradas (nombre/tamaño/ratio) en vez del volcado hex.
    /// Cubre ZIP (y .jar/.apk/.epub/OOXML), tar y tar.gz.
    Archive,
    /// Visor de fuentes (`nahual-font-viewer-llimphi`); metadatos + una
    /// muestra dibujada con los contornos de la propia fuente (TTF/OTF).
    Font,
    /// Visor de texto (`nahual-text-viewer-llimphi`); degrada a "binario"
    /// si el contenido no es UTF-8. Es el fallback universal.
    Text,
}

/// Descripción declarativa de un visor: qué naturaleza de dato cubre y con
/// qué prioridad compite. Es el equivalente UI de una `card_core::Card` de
/// tipo `Data` cuyo `presentation_hint` (lens) y `mime` declaran qué sabe
/// pintar. Bajo el AppBus, estas filas vendrían del broker; hoy son
/// estáticas (ver [`registry`]).
#[derive(Debug, Clone, Copy)]
pub struct ViewerCard {
    /// Visor concreto que se monta si esta fila gana.
    pub kind: ViewerKind,
    /// Lenses (`presentation_hint`) que el visor reclama. Match exacto.
    pub lenses: &'static [&'static str],
    /// Prefijos de `mime` que cubre (p. ej. `"image/"`).
    pub mime_prefixes: &'static [&'static str],
    /// `mime` exactos que cubre (p. ej. `"application/zip"`).
    pub mime_exact: &'static [&'static str],
    /// Prioridad de desempate cuando más de un visor matchea con la misma
    /// especificidad. Mismo orden que usa el broker (`Low<Normal<High<Critical`).
    pub priority: Priority,
}

/// Tabla de ruteo: un [`ViewerCard`] por visor montado en el shell.
///
/// El orden importa sólo como desempate final (especificidad y `Priority`
/// mandan primero). `Text` NO está acá: es el fallback explícito de [`pick`]
/// cuando ninguna fila matchea.
pub fn registry() -> &'static [ViewerCard] {
    use Priority::Normal;
    &[
        ViewerCard {
            kind: ViewerKind::Image,
            lenses: &["gallery"],
            mime_prefixes: &["image/"],
            mime_exact: &[],
            priority: Normal,
        },
        ViewerCard {
            kind: ViewerKind::Video,
            lenses: &["video"],
            mime_prefixes: &["video/"],
            mime_exact: &[],
            priority: Normal,
        },
        ViewerCard {
            kind: ViewerKind::Audio,
            lenses: &["audio"],
            mime_prefixes: &["audio/"],
            mime_exact: &[],
            priority: Normal,
        },
        ViewerCard {
            kind: ViewerKind::Card,
            lenses: &["card"],
            mime_prefixes: &[],
            mime_exact: &[],
            priority: Normal,
        },
        ViewerCard {
            kind: ViewerKind::Tree,
            lenses: &["tree"],
            mime_prefixes: &[],
            mime_exact: &[],
            priority: Normal,
        },
        ViewerCard {
            kind: ViewerKind::Table,
            lenses: &["table"],
            mime_prefixes: &[],
            mime_exact: &[],
            priority: Normal,
        },
        ViewerCard {
            kind: ViewerKind::Markdown,
            lenses: &["markdown"],
            mime_prefixes: &[],
            mime_exact: &[],
            priority: Normal,
        },
        ViewerCard {
            kind: ViewerKind::Font,
            lenses: &["font"],
            mime_prefixes: &[],
            mime_exact: &[],
            priority: Normal,
        },
        // Contenedores: un comprimido se lista (entradas) en vez de volcarse.
        // ZIP cubre .jar/.apk/.epub/OOXML; gzip se asume envolviendo un tar.
        ViewerCard {
            kind: ViewerKind::Archive,
            lenses: &[],
            mime_prefixes: &[],
            mime_exact: &["application/zip", "application/x-tar", "application/gzip"],
            priority: Normal,
        },
        // Binarios que shuma detecta por magic-bytes sin lens y que ningún
        // visor rico cubre: un dump hex es mejor que "(binario — sin preview)".
        ViewerCard {
            kind: ViewerKind::Hex,
            lenses: &[],
            mime_prefixes: &[],
            mime_exact: &["application/x-executable", "application/wasm"],
            priority: Normal,
        },
    ]
}

/// Especificidad de un match: a mayor número, más concreta la coincidencia.
/// El orden replica el de la versión hardcoded previa: `lens` manda sobre
/// `mime` exacto, y éste sobre el prefijo de `mime`.
const SCORE_LENS: u8 = 3;
const SCORE_MIME_EXACT: u8 = 2;
const SCORE_MIME_PREFIX: u8 = 1;
const SCORE_NONE: u8 = 0;

fn score(card: &ViewerCard, d: &Discernment) -> u8 {
    if let Some(lens) = d.lens.as_deref() {
        if card.lenses.contains(&lens) {
            return SCORE_LENS;
        }
    }
    if let Some(mime) = d.mime.as_deref() {
        if card.mime_exact.contains(&mime) {
            return SCORE_MIME_EXACT;
        }
        if card.mime_prefixes.iter().any(|p| mime.starts_with(p)) {
            return SCORE_MIME_PREFIX;
        }
    }
    SCORE_NONE
}

/// Elige el visor para un discernimiento consultando [`registry`].
///
/// Reglas, en orden:
/// 1. Caso especial GIF: shuma lo marca `gallery` (es imagen), pero un GIF
///    animado se ve mejor reproducido. Va al video viewer (que acepta su
///    `FrameSource` y lo anima en loop; un GIF de un frame se ve estático).
/// 2. Match por la tabla: gana la fila con mayor especificidad
///    (`lens` > `mime` exacto > prefijo de `mime`), desempatada por
///    `Priority` y, en última instancia, por orden en la tabla.
/// 3. Fallback a [`ViewerKind::Text`] — el visor que nunca falla feo.
///
/// Un `None` (no se pudo discernir, p.ej. archivo ilegible) cae a texto.
pub fn pick(discernment: Option<&Discernment>) -> ViewerKind {
    let Some(d) = discernment else {
        return ViewerKind::Text;
    };
    if d.mime.as_deref() == Some("image/gif") {
        return ViewerKind::Video;
    }
    registry()
        .iter()
        .map(|card| (score(card, d), card))
        .filter(|(s, _)| *s > SCORE_NONE)
        // mayor especificidad, luego mayor Priority; el orden de la tabla
        // queda como desempate estable.
        .max_by_key(|(s, card)| (*s, card.priority))
        .map(|(_, card)| card.kind)
        .unwrap_or(ViewerKind::Text)
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
    }

    #[test]
    fn comprimidos_van_a_archive() {
        assert_eq!(pick(Some(&disc(None, Some("application/zip")))), ViewerKind::Archive);
        assert_eq!(pick(Some(&disc(None, Some("application/x-tar")))), ViewerKind::Archive);
        assert_eq!(pick(Some(&disc(None, Some("application/gzip")))), ViewerKind::Archive);
    }

    #[test]
    fn markdown_va_a_markdown() {
        assert_eq!(pick(Some(&disc(Some("markdown"), Some("text/plain")))), ViewerKind::Markdown);
    }

    #[test]
    fn font_lens_va_a_font() {
        assert_eq!(pick(Some(&disc(Some("font"), Some("font/sfnt")))), ViewerKind::Font);
    }

    #[test]
    fn code_va_a_texto() {
        assert_eq!(pick(Some(&disc(Some("code"), Some("text/plain")))), ViewerKind::Text);
    }

    #[test]
    fn sin_discernimiento_cae_a_texto() {
        assert_eq!(pick(None), ViewerKind::Text);
    }

    #[test]
    fn lens_gana_sobre_mime_prefijo() {
        // lens explícito (especificidad 3) debe ganar aunque el mime también
        // matchee un prefijo de otro visor distinto.
        assert_eq!(pick(Some(&disc(Some("tree"), Some("image/png")))), ViewerKind::Tree);
    }

    #[test]
    fn cada_kind_de_la_tabla_es_alcanzable_por_su_lens() {
        // garantía de cobertura: ninguna fila queda muerta.
        for card in registry() {
            if let Some(&lens) = card.lenses.first() {
                assert_eq!(pick(Some(&disc(Some(lens), None))), card.kind);
            } else if let Some(&mime) = card.mime_exact.first() {
                assert_eq!(pick(Some(&disc(None, Some(mime)))), card.kind);
            }
        }
    }
}
