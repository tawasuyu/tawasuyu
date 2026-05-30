//! `nahual-hex-viewer-llimphi` — volcado hex/ASCII de binarios.
//!
//! Séptimo visor del shell meta-app. Los binarios que `shuma-discern`
//! reconoce por magic-bytes (ELF, wasm, gzip, zip…) hasta ahora caían al
//! text viewer, que sólo dice "(binario — sin preview)". Este visor los
//! vuelca como un clásico dump `offset  hex  |ascii|`: alcanza para
//! inspeccionar una cabecera, confirmar un magic number o ver la forma
//! de un blob, sin salir del shell.
//!
//! Patrón fino de los otros viewers: carga sync en [`load_hex`], render
//! en [`hex_viewer_view`]. Lee sólo los primeros KB (un dump más largo
//! no se escanea a ojo). El cuerpo se pide en fuente **monoespaciada**
//! (`font_family = "monospace"`) para que las columnas cuadren.

#![forbid(unsafe_code)]

use std::path::Path;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Bytes que se vuelcan por defecto (4 KiB = 256 filas). El caller puede
/// pedir más; pasado cierto punto un dump deja de ser legible a ojo.
pub const DEFAULT_HEX_BYTES_MAX: usize = 4 * 1024;

/// Bytes por fila del dump.
const COLS: usize = 16;

/// Estado del visor.
#[derive(Debug, Clone)]
pub enum HexPreview {
    /// Sin archivo seleccionado.
    Empty,
    /// Dump listo. `total` es el tamaño real del archivo (puede exceder
    /// los bytes volcados → el header lo señala).
    Dump { text: String, total: u64, shown: usize },
    /// `fs::read`/`metadata` falló.
    Error(String),
}

impl Default for HexPreview {
    fn default() -> Self {
        HexPreview::Empty
    }
}

/// Lee hasta `max_bytes` del inicio del archivo y arma el dump.
pub fn load_hex(path: &Path, max_bytes: usize) -> HexPreview {
    use std::io::Read;
    let total = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => return HexPreview::Error(e.to_string()),
    };
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return HexPreview::Error(e.to_string()),
    };
    let mut buf = vec![0u8; max_bytes];
    let n = match f.read(&mut buf) {
        Ok(n) => n,
        Err(e) => return HexPreview::Error(e.to_string()),
    };
    buf.truncate(n);
    HexPreview::Dump {
        text: dump(&buf),
        total,
        shown: n,
    }
}

/// Formatea `bytes` como `OFFSET  hex(8) hex(8)  |ascii|`, 16 por fila.
fn dump(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4);
    for (row, chunk) in bytes.chunks(COLS).enumerate() {
        if row > 0 {
            out.push('\n');
        }
        // Offset.
        let offset = row * COLS;
        out.push_str(&format!("{offset:08x}  "));
        // Hex, en dos grupos de 8 separados por un espacio extra.
        for i in 0..COLS {
            if i == COLS / 2 {
                out.push(' ');
            }
            match chunk.get(i) {
                Some(b) => out.push_str(&format!("{b:02x} ")),
                None => out.push_str("   "), // relleno para alinear el ascii
            }
        }
        // ASCII.
        out.push_str(" |");
        for &b in chunk {
            let c = if (0x20..0x7f).contains(&b) { b as char } else { '.' };
            out.push(c);
        }
        out.push('|');
    }
    out
}

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct HexViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
}

impl Default for HexViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl HexViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
        }
    }
}

/// Pinta header (nombre · tamaño · "primeros N B" si truncó) + body con
/// el dump en monoespaciada.
pub fn hex_viewer_view<Msg>(
    state: &HexPreview,
    path: Option<&Path>,
    palette: &HexViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let name = match path {
        Some(p) => p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| p.display().to_string()),
        None => "(seleccioná un binario)".to_string(),
    };
    let header_text = match state {
        HexPreview::Dump { total, shown, .. } => {
            if (*shown as u64) < *total {
                format!("hex · {name} · {total} B (primeros {shown})")
            } else {
                format!("hex · {name} · {total} B")
            }
        }
        _ => format!("hex · {name}"),
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
        HexPreview::Empty => ("—".to_string(), palette.fg_muted),
        HexPreview::Dump { text, .. } => (text.clone(), palette.fg_text),
        HexPreview::Error(e) => (format!("(error: {e})"), palette.fg_error),
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
    fn dump_basico_alinea_offset_hex_ascii() {
        let d = dump(b"Hello, world!");
        // Una sola fila: 13 bytes.
        assert!(d.starts_with("00000000  "));
        assert!(d.contains("48 65 6c 6c 6f")); // "Hello"
        assert!(d.ends_with("|Hello, world!|"));
    }

    #[test]
    fn no_imprimibles_son_punto() {
        let d = dump(&[0x00, 0x1f, 0x7f, 0x41]);
        assert!(d.ends_with("|...A|"));
    }

    #[test]
    fn dos_filas_tienen_offset_correcto() {
        let bytes: Vec<u8> = (0u8..20).collect();
        let d = dump(&bytes);
        let mut lines = d.lines();
        assert!(lines.next().unwrap().starts_with("00000000  "));
        assert!(lines.next().unwrap().starts_with("00000010  "));
    }

    #[test]
    fn load_inexistente_es_error() {
        assert!(matches!(
            load_hex(Path::new("/no/existe.bin"), DEFAULT_HEX_BYTES_MAX),
            HexPreview::Error(_)
        ));
    }
}
