//! llimphi-text — Texto sobre vello vía parley.
//!
//! parley hace shaping completo (bidi, ligatures, kerning), line break y
//! alineación; fontique resuelve fuentes del sistema con fallback CJK/emoji.
//! Aquí lo envolvemos en una API mínima centrada en el caso común: un
//! bloque de texto con color uniforme, ancho máximo opcional y alineación.

use llimphi_raster::peniko::{Brush, Color};
use llimphi_raster::vello;

pub use llimphi_raster::peniko;
pub use parley;

/// Estado compartido del motor de texto. Una instancia por proceso es lo
/// recomendado: `FontContext` cachea la base de fuentes y `LayoutContext`
/// reutiliza allocaciones entre layouts.
pub struct Typesetter {
    font_cx: parley::FontContext,
    layout_cx: parley::LayoutContext<()>,
    /// Contexto separado para layouts multicolor (`Brush` por rango). El
    /// brush genérico de parley no puede ser `()` y `RunBrush` a la vez en
    /// el mismo `LayoutContext`, así que mantenemos uno por sabor.
    runs_cx: parley::LayoutContext<RunBrush>,
}

impl Default for Typesetter {
    fn default() -> Self {
        Self::new()
    }
}

impl Typesetter {
    pub fn new() -> Self {
        Self {
            font_cx: parley::FontContext::new(),
            layout_cx: parley::LayoutContext::new(),
            runs_cx: parley::LayoutContext::new(),
        }
    }

    /// Acceso al `FontContext` por si se necesita registrar fuentes extra
    /// o cambiar la stack de fallback.
    pub fn font_context_mut(&mut self) -> &mut parley::FontContext {
        &mut self.font_cx
    }

    /// Construye y resuelve un `parley::Layout`. Aplica `font_size`,
    /// `line_height` (multiplicador del font_size), `max_width` (line
    /// break), y `alignment`. `italic`=true selecciona la variante
    /// italic/oblique de la fuente activa (vía `parley::FontStyle`).
    pub fn layout(
        &mut self,
        text: &str,
        size_px: f32,
        max_width: Option<f32>,
        alignment: Alignment,
        line_height: f32,
        italic: bool,
        font_family: Option<&str>,
    ) -> parley::Layout<()> {
        let mut builder =
            self.layout_cx
                .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontSize(size_px));
        builder.push_default(parley::StyleProperty::LineHeight(line_height));
        if italic {
            builder.push_default(parley::StyleProperty::FontStyle(
                parley::FontStyle::Italic,
            ));
        }
        if let Some(ff) = font_family {
            // parley::FontStack::Source acepta CSS-like syntax
            // (`"Helvetica", sans-serif`).
            builder.push_default(parley::StyleProperty::FontStack(
                parley::FontStack::Source(std::borrow::Cow::Borrowed(ff)),
            ));
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);
        layout.align(
            max_width,
            alignment.into(),
            parley::AlignmentOptions::default(),
        );
        layout
    }

    /// Construye un layout **multicolor** en una sola pasada de shaping:
    /// `default_color` cubre todo el texto y cada `(start_byte, end_byte,
    /// color)` lo sobreescribe en su rango (offsets en **bytes**, no chars —
    /// la convención de parley). Pensado para syntax highlighting: shapear
    /// la línea entera una vez con un color por token, en vez de un layout
    /// por token. Sin wrap (`max_width = None`); el caller posiciona la línea.
    pub fn layout_runs(
        &mut self,
        text: &str,
        size_px: f32,
        default_color: Color,
        runs: &[(usize, usize, Color)],
        alignment: Alignment,
        line_height: f32,
    ) -> parley::Layout<RunBrush> {
        let mut builder = self
            .runs_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontSize(size_px));
        builder.push_default(parley::StyleProperty::LineHeight(line_height));
        builder.push_default(parley::StyleProperty::Brush(RunBrush(default_color)));
        let len = text.len();
        for &(start, end, color) in runs {
            if start < end && end <= len {
                builder.push(parley::StyleProperty::Brush(RunBrush(color)), start..end);
            }
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(None, alignment.into(), parley::AlignmentOptions::default());
        layout
    }
}

/// Brush por-run para texto multicolor. Newtype sobre [`Color`] porque
/// parley exige que el brush genérico implemente `Default` (que `Color` no
/// garantiza); aquí proveemos uno explícito (negro opaco) que nunca se ve
/// en la práctica: todo run lleva su color o el `default_color` del bloque.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RunBrush(pub Color);

impl Default for RunBrush {
    fn default() -> Self {
        RunBrush(Color::from_rgba8(0, 0, 0, 255))
    }
}

/// Alineación horizontal del bloque dentro de su ancho máximo.
#[derive(Debug, Clone, Copy)]
pub enum Alignment {
    Start,
    Center,
    End,
    Justify,
}

impl From<Alignment> for parley::Alignment {
    fn from(a: Alignment) -> Self {
        match a {
            Alignment::Start => parley::Alignment::Start,
            Alignment::Center => parley::Alignment::Middle,
            Alignment::End => parley::Alignment::End,
            Alignment::Justify => parley::Alignment::Justified,
        }
    }
}

/// Especificación de un bloque de texto a rasterizar.
pub struct TextBlock<'a> {
    pub text: &'a str,
    pub size_px: f32,
    pub color: Color,
    /// Esquina superior-izquierda del bloque (no el baseline — parley se
    /// encarga del baseline internamente).
    pub origin: (f64, f64),
    pub max_width: Option<f32>,
    pub alignment: Alignment,
    /// Múltiplo del font_size (1.0 = compacto, 1.3 = cómodo).
    pub line_height: f32,
    /// `true` → fuerza variante italic/oblique en la fuente activa.
    pub italic: bool,
    /// CSS-style `font-family` string. `None` = sans-serif default.
    pub font_family: Option<String>,
}

impl<'a> TextBlock<'a> {
    /// Constructor simple para una línea sin wrap.
    pub fn simple(text: &'a str, size_px: f32, color: Color, origin: (f64, f64)) -> Self {
        Self {
            text,
            size_px,
            color,
            origin,
            max_width: None,
            alignment: Alignment::Start,
            line_height: 1.0,
            italic: false,
            font_family: None,
        }
    }
}

/// Medidas resultantes de un layout.
#[derive(Debug, Clone, Copy)]
pub struct Measurement {
    pub width: f32,
    pub height: f32,
}

/// Construye el layout (shaping + line break + alineación) listo para medir
/// y/o pintar. Usá esta API cuando necesitás el alto **antes** de elegir el
/// origen (p. ej. centrado vertical) y no querés repetir el shaping en el
/// `draw`: medís sobre el layout retornado y luego lo pasás a
/// [`draw_layout`].
pub fn layout_block(ts: &mut Typesetter, block: &TextBlock<'_>) -> parley::Layout<()> {
    ts.layout(
        block.text,
        block.size_px,
        block.max_width,
        block.alignment,
        block.line_height,
        block.italic,
        block.font_family.as_deref(),
    )
}

/// Devuelve las medidas de un layout ya resuelto. Equivalente conceptual a
/// `(layout.width(), layout.height())` pero envuelto en [`Measurement`].
pub fn measurement(layout: &parley::Layout<()>) -> Measurement {
    Measurement {
        width: layout.width(),
        height: layout.height(),
    }
}

/// Pinta un layout ya resuelto en `scene` con `color` y un offset `origin`
/// (esquina superior-izquierda del bloque). No alloca: los glifos van
/// directo del iterador de parley al builder de vello.
pub fn draw_layout(
    scene: &mut vello::Scene,
    layout: &parley::Layout<()>,
    color: Color,
    origin: (f64, f64),
) {
    draw_layout_xf(scene, layout, color, vello::kurbo::Affine::translate(origin));
}

/// Igual que [`draw_layout`] pero con una **afín completa** en vez de sólo un
/// desplazamiento: permite pintar texto girado/escalado (p. ej. dentro de un
/// marco rotado en una presentación espacial). El origen del layout (0,0) es el
/// que mapea `transform`; las posiciones de glifo se aplican en ese espacio.
pub fn draw_layout_xf(
    scene: &mut vello::Scene,
    layout: &parley::Layout<()>,
    color: Color,
    transform: vello::kurbo::Affine,
) {
    let brush = Brush::Solid(color);
    for line in layout.lines() {
        for item in line.items() {
            if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                let run = glyph_run.run();
                let font = run.font().clone();
                let font_size = run.font_size();
                scene
                    .draw_glyphs(&font)
                    .font_size(font_size)
                    .brush(&brush)
                    .transform(transform)
                    .draw(
                        peniko::Fill::NonZero,
                        glyph_run.positioned_glyphs().map(|g| vello::Glyph {
                            id: g.id as u32,
                            x: g.x,
                            y: g.y,
                        }),
                    );
            }
        }
    }
}

/// Pinta un layout **multicolor** ([`Typesetter::layout_runs`]): cada
/// `glyph_run` usa el color de su propio brush ([`RunBrush`]) en vez de un
/// color uniforme. `origin` es la esquina superior-izquierda del bloque.
pub fn draw_layout_runs(
    scene: &mut vello::Scene,
    layout: &parley::Layout<RunBrush>,
    origin: (f64, f64),
) {
    let transform = vello::kurbo::Affine::translate(origin);
    for line in layout.lines() {
        for item in line.items() {
            if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                let brush = Brush::Solid(glyph_run.style().brush.0);
                let run = glyph_run.run();
                let font = run.font().clone();
                let font_size = run.font_size();
                scene
                    .draw_glyphs(&font)
                    .font_size(font_size)
                    .brush(&brush)
                    .transform(transform)
                    .draw(
                        peniko::Fill::NonZero,
                        glyph_run.positioned_glyphs().map(|g| vello::Glyph {
                            id: g.id as u32,
                            x: g.x,
                            y: g.y,
                        }),
                    );
            }
        }
    }
}

/// Mide sin pintar. Atajo de [`layout_block`] + [`measurement`] para
/// llamadores que sólo necesitan el bounding box.
pub fn measure(ts: &mut Typesetter, block: &TextBlock<'_>) -> Measurement {
    measurement(&layout_block(ts, block))
}

/// Rasteriza el bloque en `scene` haciendo shaping una sola vez. Equivale a
/// `layout_block` + `draw_layout` con `block.origin`.
pub fn draw_block(scene: &mut vello::Scene, ts: &mut Typesetter, block: &TextBlock<'_>) {
    let layout = layout_block(ts, block);
    draw_layout(scene, &layout, block.color, block.origin);
}
