//! `nahual-archive-viewer-llimphi` — visor de archivos ZIP.
//!
//! Décimo visor del shell meta-app. Un `.zip` lo detecta `shuma-discern`
//! por su magic `PK\x03\x04`, pero hasta ahora caía al **hex viewer** —
//! que vuelca bytes ilegibles. Un ZIP es un *contenedor*: lo útil es ver
//! qué hay **dentro**, no su entropía. Este visor lee el directorio
//! central (sin descomprimir) y lista cada entrada con su tamaño y ratio
//! de compresión.
//!
//! Cubre además la familia entera basada en ZIP — `.jar`, `.apk`, `.epub`,
//! y los ofimáticos OOXML (`.docx`/`.xlsx`/`.pptx`) son ZIPs, así que
//! abrirlos acá muestra su estructura interna (p.ej. `word/document.xml`).
//!
//! Patrón fino de los otros viewers: carga sync en [`load_archive`],
//! render en [`archive_viewer_view`]. No conoce el AppBus: el caller
//! pasa el path. MVP feo-primero: la lista es un bloque de texto
//! monoespaciado, estático (sin extraer entradas con click todavía).

#![forbid(unsafe_code)]

use std::path::Path;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Entradas máximas a listar. Un ZIP con más se trunca para que parley no
/// se atragante; igual mostramos el conteo total en el resumen.
const MAX_ENTRIES: usize = 2000;
/// Ancho máximo del nombre en el render (se trunca con `…` por la
/// izquierda para conservar el sufijo, que suele ser lo distintivo).
const MAX_NAME: usize = 64;

/// Una entrada del archivo (sin su contenido).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    pub name: String,
    pub is_dir: bool,
    /// Tamaño sin comprimir, en bytes.
    pub size: u64,
    /// Tamaño comprimido en el archivo, en bytes.
    pub compressed: u64,
}

/// Resumen + listado de un archivo abierto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveListing {
    pub entries: Vec<ArchiveEntry>,
    /// Total de entradas en el archivo (puede superar a `entries.len()`
    /// si se truncó en [`MAX_ENTRIES`]).
    pub total_entries: usize,
    pub total_size: u64,
    pub total_compressed: u64,
    pub truncated: bool,
}

/// Estado del visor. Replica la forma de los otros para que el shell lo
/// trate igual.
#[derive(Debug, Clone, Default)]
pub enum ArchivePreview {
    /// Sin archivo seleccionado.
    #[default]
    Empty,
    /// Archivo abierto y listado.
    Listing(ArchiveListing),
    /// No es un ZIP válido o falló la E/S.
    Error(String),
}

/// Abre el ZIP y lee su directorio central. No descomprime nada: usa
/// `by_index_raw`, que sólo lee los headers (metadata), así que es barato
/// aun para archivos grandes y no necesita el backend de compresión.
pub fn load_archive(path: &Path) -> ArchivePreview {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return ArchivePreview::Error(e.to_string()),
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => return ArchivePreview::Error(format!("no es un ZIP válido: {e}")),
    };
    let total_entries = archive.len();
    let mut entries = Vec::with_capacity(total_entries.min(MAX_ENTRIES));
    let mut total_size = 0u64;
    let mut total_compressed = 0u64;
    for i in 0..total_entries {
        let entry = match archive.by_index_raw(i) {
            Ok(e) => e,
            Err(e) => return ArchivePreview::Error(e.to_string()),
        };
        total_size = total_size.saturating_add(entry.size());
        total_compressed = total_compressed.saturating_add(entry.compressed_size());
        if entries.len() < MAX_ENTRIES {
            entries.push(ArchiveEntry {
                name: entry.name().to_string(),
                is_dir: entry.is_dir(),
                size: entry.size(),
                compressed: entry.compressed_size(),
            });
        }
    }
    ArchivePreview::Listing(ArchiveListing {
        truncated: entries.len() < total_entries,
        entries,
        total_entries,
        total_size,
        total_compressed,
    })
}

/// Renderiza el listado a un bloque de texto monoespaciado: una línea por
/// entrada con `tamaño  ratio  nombre`. Las carpetas se marcan con `/`.
fn render_listing(l: &ArchiveListing) -> String {
    let mut out = String::new();
    let ratio = if l.total_size > 0 {
        100 - (l.total_compressed * 100 / l.total_size).min(100)
    } else {
        0
    };
    out.push_str(&format!(
        "{} entradas · {} → {} ({}% ahorro)\n",
        l.total_entries,
        fmt_bytes(l.total_size),
        fmt_bytes(l.total_compressed),
        ratio,
    ));
    out.push_str("────────────────────────────────────────\n");
    for e in &l.entries {
        let name = if e.is_dir {
            format!("{}/", e.name.trim_end_matches('/'))
        } else {
            e.name.clone()
        };
        let name = ellipsize_left(&name, MAX_NAME);
        if e.is_dir {
            out.push_str(&format!("{:>10}         {}\n", "—", name));
        } else {
            let r = if e.size > 0 {
                format!("{}%", 100 - (e.compressed * 100 / e.size).min(100))
            } else {
                "—".to_string()
            };
            out.push_str(&format!("{:>10}  {:>5}  {}\n", fmt_bytes(e.size), r, name));
        }
    }
    if l.truncated {
        out.push_str(&format!(
            "… ({} entradas más sin listar)\n",
            l.total_entries - l.entries.len()
        ));
    }
    out
}

/// Tamaño humano-legible (KiB/MiB/GiB), una cifra decimal.
fn fmt_bytes(n: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if n >= GIB {
        format!("{:.1} GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        format!("{:.1} KiB", n as f64 / KIB as f64)
    } else {
        format!("{n} B")
    }
}

/// Trunca por la izquierda conservando el sufijo (`…rd/document.xml`), que
/// es lo que distingue rutas largas con prefijo común.
fn ellipsize_left(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(count - (max - 1)).collect();
    format!("…{tail}")
}

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
}

impl Default for ArchiveViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl ArchiveViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
        }
    }
}

/// Pinta header (nombre del archivo) + body con el listado monoespaciado.
pub fn archive_viewer_view<Msg>(
    state: &ArchivePreview,
    path: Option<&Path>,
    palette: &ArchiveViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let header_text = match path {
        Some(p) => format!(
            "archive · {}",
            p.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string())
        ),
        None => "(seleccioná un ZIP)".to_string(),
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let (body_text, body_color) = match state {
        ArchivePreview::Empty => ("—".to_string(), palette.fg_muted),
        ArchivePreview::Listing(l) => (render_listing(l), palette.fg_text),
        ArchivePreview::Error(e) => (format!("(no se pudo abrir: {e})"), palette.fg_error),
    };

    let body = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned_full(
        body_text,
        12.0,
        body_color,
        Alignment::Start,
        false,
        Some("monospace".to_string()),
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(vec![header, body])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_humanos() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(2048), "2.0 KiB");
        assert_eq!(fmt_bytes(1024 * 1024 * 3), "3.0 MiB");
    }

    #[test]
    fn ellipsize_conserva_sufijo() {
        let s = "word/very/long/path/to/document.xml";
        let e = ellipsize_left(s, 16);
        assert!(e.starts_with('…'));
        assert!(e.ends_with("document.xml"));
        assert!(e.chars().count() <= 16);
    }

    #[test]
    fn ellipsize_no_toca_cortos() {
        assert_eq!(ellipsize_left("a.txt", 16), "a.txt");
    }

    #[test]
    fn listing_resume_y_lista() {
        let l = ArchiveListing {
            entries: vec![
                ArchiveEntry { name: "dir/".into(), is_dir: true, size: 0, compressed: 0 },
                ArchiveEntry { name: "a.txt".into(), is_dir: false, size: 1000, compressed: 400 },
            ],
            total_entries: 2,
            total_size: 1000,
            total_compressed: 400,
            truncated: false,
        };
        let out = render_listing(&l);
        assert!(out.contains("2 entradas"));
        assert!(out.contains("60% ahorro")); // 1 - 400/1000
        assert!(out.contains("a.txt"));
        assert!(out.contains("dir/"));
    }

    #[test]
    fn archivo_inexistente_es_error() {
        let p = std::path::Path::new("/no/existe/x.zip");
        assert!(matches!(load_archive(p), ArchivePreview::Error(_)));
    }

    #[test]
    fn basura_no_es_zip() {
        let tmp = std::env::temp_dir().join("nahual-archive-viewer-test-bad.zip");
        std::fs::write(&tmp, b"no soy un zip").unwrap();
        assert!(matches!(load_archive(&tmp), ArchivePreview::Error(_)));
        let _ = std::fs::remove_file(&tmp);
    }
}
