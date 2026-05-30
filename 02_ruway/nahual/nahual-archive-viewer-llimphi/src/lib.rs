//! `nahual-archive-viewer-llimphi` — visor de archivos comprimidos.
//!
//! Décimo visor del shell meta-app. Un `.zip`/`.tar`/`.tar.gz` lo detecta
//! `shuma-discern` por su magic, pero hasta ahora caían al **hex viewer**
//! (o al texto) — bytes ilegibles. Un archivo comprimido es un
//! *contenedor*: lo útil es ver qué hay **dentro**, no su entropía. Este
//! visor lista cada entrada con su tamaño (y, para ZIP, su ratio).
//!
//! Soporta tres formatos, decidido por el **contenido** (no la extensión):
//! - **ZIP** (`PK`): lee el directorio central con `by_index_raw`, sin
//!   descomprimir. Cubre la familia entera — `.jar`/`.apk`/`.epub` y los
//!   ofimáticos OOXML (`.docx`/`.xlsx`/`.pptx`) son ZIPs.
//! - **tar** (`ustar` en off 257): recorre los headers en streaming.
//! - **tar.gz** (`1f 8b`): descomprime en streaming con `flate2` y recorre
//!   el tar interno; salta los datos de cada entrada (sólo lee headers),
//!   así no carga el archivo entero en memoria.
//!
//! Patrón fino de los otros viewers: carga sync en [`load_archive`],
//! render en [`archive_viewer_view`]. No conoce el AppBus: el caller
//! pasa el path. MVP feo-primero: la lista es un bloque de texto
//! monoespaciado, estático (sin extraer entradas con click todavía).

#![forbid(unsafe_code)]

use std::io::Read;
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

/// Qué formato de contenedor se abrió. Cambia cómo se rotula el resumen
/// (el ratio de compresión sólo tiene sentido por-entrada en ZIP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    Tar,
    TarGz,
}

/// Resumen + listado de un archivo abierto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveListing {
    pub kind: ArchiveKind,
    pub entries: Vec<ArchiveEntry>,
    /// Total de entradas listadas. Para ZIP es el total real del archivo;
    /// para tar/tar.gz (streaming) es lo que alcanzamos a leer.
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

/// Abre el archivo, olfatea su magic y despacha al lister del formato. La
/// detección es por **contenido**: `PK` → ZIP, `ustar` en off 257 → tar,
/// `1f 8b` → gzip (que asumimos envuelve un tar).
pub fn load_archive(path: &Path) -> ArchivePreview {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return ArchivePreview::Error(e.to_string()),
    };
    // 512 bytes alcanzan para el magic de ZIP/gzip (off 0) y el de tar
    // (off 257); es además el tamaño de un header tar.
    let mut head = [0u8; 512];
    let n = match file.read(&mut head) {
        Ok(n) => n,
        Err(e) => return ArchivePreview::Error(e.to_string()),
    };
    let head = &head[..n];

    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        // ZIP necesita el directorio central (al final): reabrimos por path.
        return load_zip(path);
    }
    if head.len() >= 262 && &head[257..262] == b"ustar" {
        return match std::fs::File::open(path) {
            Ok(f) => list_tar(f, ArchiveKind::Tar),
            Err(e) => ArchivePreview::Error(e.to_string()),
        };
    }
    if head.starts_with(&[0x1F, 0x8B]) {
        return match std::fs::File::open(path) {
            Ok(f) => list_tar(flate2::read::GzDecoder::new(f), ArchiveKind::TarGz),
            Err(e) => ArchivePreview::Error(e.to_string()),
        };
    }
    ArchivePreview::Error("formato de archivo no reconocido".to_string())
}

/// Lee el directorio central de un ZIP. No descomprime nada: usa
/// `by_index_raw`, que sólo lee los headers (metadata), así que es barato
/// aun para archivos grandes y no necesita el backend de compresión.
fn load_zip(path: &Path) -> ArchivePreview {
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
        kind: ArchiveKind::Zip,
        truncated: entries.len() < total_entries,
        entries,
        total_entries,
        total_size,
        total_compressed,
    })
}

/// Recorre los headers de un tar (posiblemente envuelto en un decoder gzip
/// en streaming). `tar::Archive::entries` salta los datos de cada entrada,
/// así que no carga el archivo entero en memoria. tar no comprime
/// por-entrada, así que `compressed == size`.
fn list_tar<R: Read>(reader: R, kind: ArchiveKind) -> ArchivePreview {
    let mut archive = tar::Archive::new(reader);
    let iter = match archive.entries() {
        Ok(it) => it,
        Err(e) => return ArchivePreview::Error(e.to_string()),
    };
    let mut entries = Vec::new();
    let mut total_size = 0u64;
    let mut truncated = false;
    for item in iter {
        let entry = match item {
            Ok(e) => e,
            Err(e) => return ArchivePreview::Error(e.to_string()),
        };
        let size = entry.header().size().unwrap_or(0);
        total_size = total_size.saturating_add(size);
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let is_dir = entry.header().entry_type().is_dir();
        let name = entry
            .path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "(nombre ilegible)".to_string());
        entries.push(ArchiveEntry { name, is_dir, size, compressed: size });
    }
    ArchivePreview::Listing(ArchiveListing {
        kind,
        total_entries: entries.len(),
        total_size,
        total_compressed: total_size,
        truncated,
        entries,
    })
}

/// Renderiza el listado a un bloque de texto monoespaciado: una línea por
/// entrada con `tamaño  ratio  nombre`. Las carpetas se marcan con `/`.
fn render_listing(l: &ArchiveListing) -> String {
    let mut out = String::new();
    match l.kind {
        ArchiveKind::Zip => {
            let ratio = if l.total_size > 0 {
                100 - (l.total_compressed * 100 / l.total_size).min(100)
            } else {
                0
            };
            out.push_str(&format!(
                "zip · {} entradas · {} → {} ({}% ahorro)\n",
                l.total_entries,
                fmt_bytes(l.total_size),
                fmt_bytes(l.total_compressed),
                ratio,
            ));
        }
        ArchiveKind::Tar => {
            out.push_str(&format!(
                "tar · {} entradas · {} (sin compresión)\n",
                l.total_entries,
                fmt_bytes(l.total_size),
            ));
        }
        ArchiveKind::TarGz => {
            out.push_str(&format!(
                "tar.gz · {} entradas · {} sin comprimir\n",
                l.total_entries,
                fmt_bytes(l.total_size),
            ));
        }
    }
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
        } else if l.kind == ArchiveKind::Zip {
            // El ratio por-entrada sólo tiene sentido en ZIP (compresión
            // por archivo). En tar todos los datos están sin comprimir.
            let r = if e.size > 0 {
                format!("{}%", 100 - (e.compressed * 100 / e.size).min(100))
            } else {
                "—".to_string()
            };
            out.push_str(&format!("{:>10}  {:>5}  {}\n", fmt_bytes(e.size), r, name));
        } else {
            out.push_str(&format!("{:>10}         {}\n", fmt_bytes(e.size), name));
        }
    }
    if l.truncated {
        let suffix = if l.kind == ArchiveKind::Zip {
            format!("{} entradas más sin listar", l.total_entries - l.entries.len())
        } else {
            "hay más entradas sin listar".to_string()
        };
        out.push_str(&format!("… ({suffix})\n"));
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
        None => "(seleccioná un ZIP/tar/tar.gz)".to_string(),
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
            kind: ArchiveKind::Zip,
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

    /// Construye un tar en memoria con dos entradas.
    fn tar_de_prueba() -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_size(5);
        h.set_mode(0o644);
        b.append_data(&mut h, "hola.txt", &b"hello"[..]).unwrap();
        let mut h2 = tar::Header::new_gnu();
        h2.set_size(3);
        h2.set_mode(0o644);
        b.append_data(&mut h2, "dir/x.bin", &b"abc"[..]).unwrap();
        b.into_inner().unwrap()
    }

    #[test]
    fn lista_tar_en_memoria() {
        let data = tar_de_prueba();
        match list_tar(&data[..], ArchiveKind::Tar) {
            ArchivePreview::Listing(l) => {
                assert_eq!(l.kind, ArchiveKind::Tar);
                assert_eq!(l.total_entries, 2);
                assert_eq!(l.total_size, 8);
                assert!(l.entries.iter().any(|e| e.name == "hola.txt" && e.size == 5));
                assert!(l.entries.iter().any(|e| e.name == "dir/x.bin"));
            }
            other => panic!("esperaba Listing, obtuve {other:?}"),
        }
    }

    #[test]
    fn lista_tar_gz_descomprimiendo() {
        use std::io::Write;
        let tar = tar_de_prueba();
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&tar).unwrap();
        let gz = enc.finish().unwrap();
        // Lo escribimos a disco y lo abrimos por load_archive (sniff real).
        let tmp = std::env::temp_dir().join("nahual-archive-viewer-test.tar.gz");
        std::fs::write(&tmp, &gz).unwrap();
        match load_archive(&tmp) {
            ArchivePreview::Listing(l) => {
                assert_eq!(l.kind, ArchiveKind::TarGz);
                assert_eq!(l.total_entries, 2);
                assert_eq!(l.total_size, 8);
            }
            other => panic!("esperaba Listing TarGz, obtuve {other:?}"),
        }
        let _ = std::fs::remove_file(&tmp);
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
