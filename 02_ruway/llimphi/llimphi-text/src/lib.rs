//! llimphi-text — Glifos sobre vello.
//!
//! Carga una fuente TTF/OTF (path o bytes) y emite runs de glifos sobre
//! una `vello::Scene`. Shaping mínimo letra-a-letra (sin kerning ni
//! ligatures); suficiente para texto Latin/dígitos. Para shaping completo
//! (Arabic, Indic, color emoji) integraremos `parley` cuando se necesite.

use std::path::Path;
use std::sync::Arc;

use llimphi_raster::peniko::{Blob, Brush, Color, Font};
use llimphi_raster::vello;
use skrifa::instance::{LocationRef, Size};
use skrifa::metrics::{GlyphMetrics, Metrics};
use skrifa::{FontRef, MetadataProvider};

pub use llimphi_raster::peniko;
pub use skrifa;

/// Errores al cargar o usar una fuente.
#[derive(Debug)]
pub enum TextError {
    Io(std::io::Error),
    Parse(String),
}

impl std::fmt::Display for TextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Parse(s) => write!(f, "font parse: {s}"),
        }
    }
}

impl std::error::Error for TextError {}

impl From<std::io::Error> for TextError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Una fuente lista para rasterizar. Se mantiene `data: Arc<Vec<u8>>`
/// porque tanto `peniko::Font` (para vello) como `skrifa::FontRef`
/// (para metrics) la quieren simultáneamente.
#[derive(Clone)]
pub struct Typeface {
    data: Arc<Vec<u8>>,
    index: u32,
    font: Font,
}

impl Typeface {
    pub fn from_bytes(bytes: Vec<u8>, index: u32) -> Result<Self, TextError> {
        let data = Arc::new(bytes);
        // Validar parse: si skrifa no puede leer la fuente, fallamos temprano.
        FontRef::from_index(&data, index)
            .map_err(|e| TextError::Parse(format!("{e:?}")))?;
        let font = Font::new(Blob::new(data.clone()), index);
        Ok(Self { data, index, font })
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, TextError> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(bytes, 0)
    }

    /// Busca la primera fuente disponible de la lista de candidatos. Usar para
    /// fallback típico Linux: AdwaitaSans → Inter → DejaVuSans → cualquiera.
    pub fn first_available<P: AsRef<Path>>(paths: &[P]) -> Result<Self, TextError> {
        let mut last_err: Option<TextError> = None;
        for p in paths {
            match Self::from_path(p) {
                Ok(t) => return Ok(t),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            TextError::Parse("no candidate paths supplied".to_string())
        }))
    }

    pub fn font(&self) -> &Font {
        &self.font
    }

    fn font_ref(&self) -> FontRef<'_> {
        // Ya validado en from_bytes.
        FontRef::from_index(&self.data, self.index)
            .expect("font data validated at construction")
    }

    /// Mide un fragmento de texto a `size_px`. Devuelve `(width, ascent, descent)`
    /// en píxeles. Útil para centrar/alinear sin shaping (Latin básico).
    pub fn measure(&self, text: &str, size_px: f32) -> Measurement {
        let font_ref = self.font_ref();
        let size = Size::new(size_px);
        let charmap = font_ref.charmap();
        let glyph_metrics: GlyphMetrics = font_ref.glyph_metrics(size, LocationRef::default());
        let metrics: Metrics = font_ref.metrics(size, LocationRef::default());
        let width = text
            .chars()
            .filter_map(|c| charmap.map(c))
            .map(|id| glyph_metrics.advance_width(id).unwrap_or(0.0))
            .sum();
        Measurement {
            width,
            ascent: metrics.ascent,
            descent: metrics.descent,
            line_gap: metrics.leading,
        }
    }
}

/// Métricas en píxeles para un fragmento de texto a un tamaño dado.
#[derive(Debug, Clone, Copy)]
pub struct Measurement {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

impl Measurement {
    /// Altura visual (ascent + |descent|).
    pub fn height(&self) -> f32 {
        self.ascent + self.descent.abs()
    }
}

/// Especificación de un fragmento de texto a rasterizar.
pub struct TextBlock<'a> {
    pub text: &'a str,
    pub size_px: f32,
    pub color: Color,
    /// Origen del baseline (esquina inferior-izquierda del primer glifo).
    pub origin: (f64, f64),
}

/// Dibuja un `TextBlock` sobre `scene`. Layout simple: cada char produce un
/// glifo, el cursor X avanza por el `advance_width` del glifo. Sin shaping
/// (no kerning, no ligaduras, no bidi).
pub fn draw_block(scene: &mut vello::Scene, face: &Typeface, block: &TextBlock<'_>) {
    let font_ref = face.font_ref();
    let size = Size::new(block.size_px);
    let charmap = font_ref.charmap();
    let metrics: GlyphMetrics = font_ref.glyph_metrics(size, LocationRef::default());

    let mut x: f32 = 0.0;
    let glyphs = block.text.chars().filter_map(|c| {
        let id = charmap.map(c)?;
        let advance = metrics.advance_width(id).unwrap_or(0.0);
        let glyph = vello::Glyph {
            id: id.to_u32(),
            x,
            y: 0.0,
        };
        x += advance;
        Some(glyph)
    });

    scene
        .draw_glyphs(face.font())
        .font_size(block.size_px)
        .transform(vello::kurbo::Affine::translate((block.origin.0, block.origin.1)))
        .brush(&Brush::Solid(block.color))
        .draw(llimphi_raster::peniko::Fill::NonZero, glyphs);
}
