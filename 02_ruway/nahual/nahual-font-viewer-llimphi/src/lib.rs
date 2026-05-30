//! `nahual-font-viewer-llimphi` — visor de fuentes TTF/OTF.
//!
//! Undécimo visor del shell meta-app. Un `.ttf`/`.otf` no lo cubría
//! ningún visor rico: caía al text viewer como "(binario — sin preview)".
//! Pero una fuente es para *verla*. Este visor parsea el archivo con
//! `ttf-parser`, muestra sus metadatos (familia, estilo, nº de glifos,
//! unidades por em) y —lo interesante— **renderiza una muestra dibujada
//! con la propia fuente del archivo**: extrae los contornos de cada glifo
//! a un `kurbo::BezPath` y los rellena en la escena vello vía `paint_with`.
//!
//! No pasa por parley (que sólo conoce las fuentes del sistema): los
//! glifos se pintan directo desde los outlines del archivo, así ves
//! exactamente la fuente que estás inspeccionando aunque no esté
//! instalada.
//!
//! Patrón fino de los otros viewers: carga sync en [`load_font`], render
//! en [`font_viewer_view`]. No conoce el AppBus: el caller pasa el path.
//! MVP feo-primero: muestra fija (pangrama + dígitos), sin elegir tamaño
//! ni texto todavía.

#![forbid(unsafe_code)]

use std::path::Path;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Tope de bytes a leer (32 MiB). Una fuente más grande es rara; el
/// caller puede subirlo.
pub const DEFAULT_FONT_BYTES_MAX: u64 = 32 * 1024 * 1024;

/// Líneas de muestra que se renderizan con la fuente del archivo.
const SAMPLE_LINES: &[&str] = &[
    "Aa Bb Cc Dd Ee Ff Gg",
    "The quick brown fox jumps",
    "0123456789  !?.,;:&@#",
];

/// Una línea de muestra ya convertida a contornos, en unidades de fuente
/// (eje Y hacia arriba, origen en la baseline). El render la escala.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleLine {
    pub path: BezPath,
    /// Ancho total avanzado, en unidades de fuente.
    pub width: f64,
}

/// Metadatos + muestras renderizables de una fuente abierta.
#[derive(Debug, Clone, PartialEq)]
pub struct FontInfo {
    pub family: String,
    pub subfamily: String,
    pub num_glyphs: u16,
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub lines: Vec<SampleLine>,
}

/// Estado del visor. Replica la forma de los otros.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum FontPreview {
    /// Sin archivo seleccionado.
    #[default]
    Empty,
    /// Fuente parseada.
    Font(Box<FontInfo>),
    /// Excede el tope de tamaño.
    TooBig(u64),
    /// No se pudo abrir/parsear.
    Error(String),
}

/// Implementa el sink de `ttf-parser` para volcar el contorno de un glifo
/// a un `kurbo::BezPath`.
struct OutlineToPath {
    path: BezPath,
}

impl ttf_parser::OutlineBuilder for OutlineToPath {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to((x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to((x as f64, y as f64));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.path.quad_to((x1 as f64, y1 as f64), (x as f64, y as f64));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path
            .curve_to((x1 as f64, y1 as f64), (x2 as f64, y2 as f64), (x as f64, y as f64));
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

/// Lee y parsea la fuente, construyendo las muestras de contorno.
pub fn load_font(path: &Path, max_bytes: u64) -> FontPreview {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > max_bytes => return FontPreview::TooBig(meta.len()),
        Err(e) => return FontPreview::Error(e.to_string()),
        _ => {}
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return FontPreview::Error(e.to_string()),
    };
    let face = match ttf_parser::Face::parse(&bytes, 0) {
        Ok(f) => f,
        Err(e) => return FontPreview::Error(format!("no parsea como fuente: {e}")),
    };
    FontPreview::Font(Box::new(build_info(&face)))
}

/// Extrae metadatos y arma las líneas de muestra a partir de una `Face`.
fn build_info(face: &ttf_parser::Face<'_>) -> FontInfo {
    let family = pick_name(face, 1).unwrap_or_else(|| "(sin nombre)".to_string());
    let subfamily = pick_name(face, 2).unwrap_or_else(|| "Regular".to_string());
    let em = face.units_per_em();
    let lines = SAMPLE_LINES
        .iter()
        .map(|s| build_line(face, s))
        .collect();
    FontInfo {
        family,
        subfamily,
        num_glyphs: face.number_of_glyphs(),
        units_per_em: em,
        ascender: face.ascender(),
        descender: face.descender(),
        lines,
    }
}

/// Toma el primer `name` legible con el `name_id` pedido (1=familia,
/// 2=subfamilia). `ttf-parser` sólo devuelve string para encodings
/// Unicode/Mac, así que algunos nombres salen `None`.
fn pick_name(face: &ttf_parser::Face<'_>, want_id: u16) -> Option<String> {
    face.names()
        .into_iter()
        .filter(|n| n.name_id == want_id)
        .find_map(|n| n.to_string())
        .filter(|s| !s.is_empty())
}

/// Convierte una cadena en un único `BezPath` (todos los glifos
/// trasladados a su posición de pen) en unidades de fuente.
fn build_line(face: &ttf_parser::Face<'_>, text: &str) -> SampleLine {
    let mut combined = BezPath::new();
    let mut pen: f64 = 0.0;
    let space = face.units_per_em() as f64 / 3.0;
    for ch in text.chars() {
        let gid = match face.glyph_index(ch) {
            Some(g) => g,
            None => {
                pen += space;
                continue;
            }
        };
        let mut sink = OutlineToPath { path: BezPath::new() };
        if face.outline_glyph(gid, &mut sink).is_some() {
            sink.path.apply_affine(Affine::translate((pen, 0.0)));
            combined.extend(sink.path.elements().iter().copied());
        }
        let adv = face.glyph_hor_advance(gid).unwrap_or(0) as f64;
        // Un avance 0 (p.ej. espacio sin glifo) usa el ancho de fallback.
        pen += if adv > 0.0 { adv } else { space };
    }
    SampleLine { path: combined, width: pen }
}

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct FontViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
    pub glyph: Color,
}

impl Default for FontViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl FontViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
            glyph: t.fg_text,
        }
    }
}

/// Pinta header + metadatos + lienzo con la muestra dibujada.
pub fn font_viewer_view<Msg>(
    state: &FontPreview,
    path: Option<&Path>,
    palette: &FontViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let header_text = match path {
        Some(p) => format!(
            "font · {}",
            p.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string())
        ),
        None => "(seleccioná una fuente TTF/OTF)".to_string(),
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: pad(12.0, 0.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let children = match state {
        FontPreview::Empty => vec![header, info_line("—", palette.fg_muted)],
        FontPreview::TooBig(n) => vec![
            header,
            info_line(&format!("(fuente muy grande: {n} bytes)"), palette.fg_muted),
        ],
        FontPreview::Error(e) => {
            vec![header, info_line(&format!("(no se pudo abrir: {e})"), palette.fg_error)]
        }
        FontPreview::Font(info) => {
            let meta = format!(
                "{} · {}\n{} glifos · {} u/em · asc {} / desc {}",
                info.family,
                info.subfamily,
                info.num_glyphs,
                info.units_per_em,
                info.ascender,
                info.descender,
            );
            vec![
                header,
                info_line(&meta, palette.fg_text),
                sample_canvas::<Msg>(info, palette),
            ]
        }
    };

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
    .children(children)
}

/// Lienzo que dibuja las líneas de muestra rellenando los contornos de
/// glifo. Los paths vienen en unidades de fuente; acá los escalamos para
/// que entren a lo ancho y los apilamos verticalmente.
fn sample_canvas<Msg>(info: &FontInfo, palette: &FontViewerPalette) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let lines = info.lines.clone();
    let em = info.units_per_em.max(1) as f64;
    let ascender = info.ascender as f64;
    let descender = info.descender as f64;
    let glyph_color = palette.glyph;

    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: pad(16.0, 10.0),
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 8.0 || rect.h <= 8.0 || lines.is_empty() {
            return;
        }
        let pad_x = 4.0_f64;
        let avail_w = (rect.w as f64 - 2.0 * pad_x).max(1.0);
        // Altura de cada renglón = alto del lienzo repartido (con tope para
        // que un panel alto no infle los glifos hasta deformarlos).
        let slot_h = ((rect.h as f64) / lines.len() as f64).min(96.0);
        // El glifo ocupa la caja ascender..descender → escala por alto.
        let line_units = (ascender - descender).max(em);
        for (i, line) in lines.iter().enumerate() {
            if line.path.elements().is_empty() {
                continue;
            }
            // Escala que respeta tanto el alto del renglón como el ancho.
            let scale_h = (slot_h * 0.72) / line_units;
            let scale_w = if line.width > 0.0 {
                avail_w / line.width
            } else {
                scale_h
            };
            let scale = scale_h.min(scale_w);
            let baseline = rect.y as f64 + i as f64 * slot_h + slot_h * 0.5 + (ascender * scale) * 0.5;
            let x0 = rect.x as f64 + pad_x;
            // Font: Y arriba; pantalla: Y abajo → escala Y negativa.
            let affine = Affine::new([scale, 0.0, 0.0, -scale, x0, baseline]);
            scene.fill(Fill::NonZero, affine, glyph_color, None, &line.path);
        }
    })
}

/// Bloque de texto de una línea (metadatos / estados).
fn info_line<Msg>(text: &str, color: Color) -> View<Msg>
where
    Msg: Clone + 'static,
{
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        padding: pad(12.0, 4.0),
        ..Default::default()
    })
    .text_aligned(text.to_string(), 12.0, color, Alignment::Start)
}

/// Padding horizontal `h` + vertical `v`.
fn pad(h: f32, v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inexistente_es_error() {
        assert!(matches!(
            load_font(Path::new("/no/existe.ttf"), DEFAULT_FONT_BYTES_MAX),
            FontPreview::Error(_)
        ));
    }

    #[test]
    fn basura_no_es_fuente() {
        let tmp = std::env::temp_dir().join("nahual-font-viewer-test-bad.ttf");
        std::fs::write(&tmp, b"no soy una fuente, soy texto cualquiera").unwrap();
        assert!(matches!(
            load_font(&tmp, DEFAULT_FONT_BYTES_MAX),
            FontPreview::Error(_)
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn outline_builder_construye_path() {
        // Verifica que el sink traduce los comandos a elementos de BezPath.
        let mut sink = OutlineToPath { path: BezPath::new() };
        use ttf_parser::OutlineBuilder;
        sink.move_to(0.0, 0.0);
        sink.line_to(10.0, 0.0);
        sink.quad_to(10.0, 10.0, 0.0, 10.0);
        sink.close();
        assert_eq!(sink.path.elements().len(), 4);
    }
}
