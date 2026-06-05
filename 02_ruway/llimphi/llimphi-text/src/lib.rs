//! llimphi-text — Texto sobre vello vía parley.
//!
//! parley hace shaping completo (bidi, ligatures, kerning), line break y
//! alineación; fontique resuelve fuentes del sistema con fallback CJK/emoji.
//! Aquí lo envolvemos en una API mínima centrada en el caso común: un
//! bloque de texto con color uniforme, ancho máximo opcional y alineación.

use vello::peniko::{Brush, Color};

pub use parley;
pub use vello;
pub use vello::peniko;

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

/// DejaVu Sans embebida como **fallback universal de símbolos**. El motor
/// confía en las fuentes del sistema vía fontique, pero muchas instalaciones
/// (p. ej. solo Liberation/Adwaita) carecen de glyphs para flechas (`→`),
/// formas geométricas (`● ▶`), dingbats (`✓ ✗ ✎`), avisos (`⚠`) o astro
/// (`♈ ☉ ☽`) — y entonces parley pinta el "tofu" (□). DejaVu cubre todo ese
/// rango; la registramos y la enganchamos al fallback del script `Common`
/// (`Zyyy`), que es donde Unicode clasifica esos símbolos. Así cualquier app
/// Llimphi deja de mostrar cuadrados sin tocar una línea de su código.
/// Licencia: Bitstream Vera + Arev (libre, redistribuible).
const DEJAVU_SANS: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

impl Typesetter {
    pub fn new() -> Self {
        let mut font_cx = parley::FontContext::new();
        Self::install_symbol_fallback(&mut font_cx);
        Self {
            font_cx,
            layout_cx: parley::LayoutContext::new(),
            runs_cx: parley::LayoutContext::new(),
        }
    }

    /// Registra DejaVu Sans y la apila como último recurso para los símbolos
    /// del script `Common` (flechas, geométricos, dingbats, astro…). Ver la
    /// nota de [`DEJAVU_SANS`]. Best-effort: si algo falla, el texto sigue
    /// funcionando con las fuentes del sistema (solo reaparecería el tofu).
    fn install_symbol_fallback(font_cx: &mut parley::FontContext) {
        use parley::fontique::Blob;
        let blob = Blob::new(std::sync::Arc::new(DEJAVU_SANS));
        let registered = font_cx.collection.register_fonts(blob, None);
        if let Some((family_id, _)) = registered.first() {
            // `Zyyy` (Common) es el script de la inmensa mayoría de los
            // símbolos que daban tofu; lo apilamos al final del fallback.
            font_cx
                .collection
                .append_fallbacks("Zyyy", std::iter::once(*family_id));
        }
    }

    /// Acceso al `FontContext` por si se necesita registrar fuentes extra
    /// o cambiar la stack de fallback.
    pub fn font_context_mut(&mut self) -> &mut parley::FontContext {
        &mut self.font_cx
    }

    /// Construye y resuelve un `parley::Layout`. Aplica `font_size`,
    /// `line_height` (multiplicador del font_size), `max_width` (line
    /// break), `alignment` y `weight` (peso de fuente CSS: 400 normal,
    /// 700 bold). `italic`=true selecciona la variante italic/oblique de
    /// la fuente activa (vía `parley::FontStyle`).
    #[allow(clippy::too_many_arguments)]
    pub fn layout(
        &mut self,
        text: &str,
        size_px: f32,
        max_width: Option<f32>,
        alignment: Alignment,
        line_height: f32,
        italic: bool,
        font_family: Option<&str>,
        weight: f32,
    ) -> parley::Layout<()> {
        let mut builder =
            self.layout_cx
                .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontSize(size_px));
        builder.push_default(parley::StyleProperty::LineHeight(line_height));
        if weight != 400.0 {
            builder.push_default(parley::StyleProperty::FontWeight(
                parley::FontWeight::new(weight),
            ));
        }
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

    /// Como [`Self::layout`] pero **clampado** a `max_lines` líneas (CSS
    /// `-webkit-line-clamp` / Flutter `maxLines`). Si el texto envuelto cabe en
    /// `max_lines` o menos, devuelve el layout completo. Si excede:
    /// - `ellipsis = true` → la última línea visible termina en `…` (se
    ///   recortan graphemes del final hasta que el bloque vuelve a caber en
    ///   `max_lines`).
    /// - `ellipsis = false` → se corta sin glifo (queda el prefijo que cupo).
    ///
    /// `max_lines = None` o `Some(0)` ⇒ sin límite (idéntico a `layout`). El
    /// clamp sólo recorta cuando hay envoltura, así que requiere un `max_width`
    /// definido para tener efecto (un label en una caja dimensionada — el caso
    /// típico). Reusa `layout` internamente: 0 costo extra cuando no trunca.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_clamped(
        &mut self,
        text: &str,
        size_px: f32,
        max_width: Option<f32>,
        alignment: Alignment,
        line_height: f32,
        italic: bool,
        font_family: Option<&str>,
        weight: f32,
        max_lines: Option<usize>,
        ellipsis: bool,
    ) -> parley::Layout<()> {
        let full = self.layout(
            text, size_px, max_width, alignment, line_height, italic, font_family, weight,
        );
        let limit = match max_lines {
            Some(n) if n >= 1 => n,
            _ => return full,
        };
        if full.lines().count() <= limit {
            return full;
        }
        // Byte de fin de la última línea visible (rango sobre `text` original).
        let mut cutoff = full
            .lines()
            .nth(limit - 1)
            .map(|l| l.text_range().end)
            .unwrap_or(text.len())
            .min(text.len());
        while cutoff > 0 && !text.is_char_boundary(cutoff) {
            cutoff -= 1;
        }
        let base = text[..cutoff].trim_end();
        if !ellipsis {
            return self.layout(
                base, size_px, max_width, alignment, line_height, italic, font_family, weight,
            );
        }
        // Recortá graphemes del final hasta que `base…` vuelva a caber en
        // `limit` líneas (apilar el `…` puede empujar una palabra a una línea
        // extra). Acotado: cada vuelta quita ≥1 char.
        let mut s = base.to_string();
        loop {
            let candidate = format!("{s}…");
            let lay = self.layout(
                &candidate, size_px, max_width, alignment, line_height, italic, font_family,
                weight,
            );
            if s.is_empty() || lay.lines().count() <= limit {
                return lay;
            }
            s.pop();
            while s.ends_with(char::is_whitespace) {
                s.pop();
            }
        }
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
        weight: f32,
    ) -> parley::Layout<RunBrush> {
        let mut builder = self
            .runs_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontSize(size_px));
        builder.push_default(parley::StyleProperty::LineHeight(line_height));
        if weight != 400.0 {
            builder.push_default(parley::StyleProperty::FontWeight(
                parley::FontWeight::new(weight),
            ));
        }
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
        // `TextBlock` no transporta peso (su API queda en normal); el peso de
        // fuente fluye por el camino del compositor, que llama a `layout`
        // directamente con el `weight` del `TextSpec`/`TextMeasure`.
        400.0,
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
    draw_layout_brush_xf(scene, layout, &Brush::Solid(color), transform);
}

/// Igual que [`draw_layout_xf`] pero con un [`Brush`] arbitrario en vez de un
/// color sólido: permite rellenar los glifos con un gradiente o una imagen
/// (p. ej. CSS `background-clip: text`). El brush se interpreta en el espacio
/// **local** del layout (origen 0,0) y `transform` lo lleva al lugar final —
/// así un gradiente construido en coords (0,0)-(w,h) queda alineado con los
/// glifos. Para texto normal usá [`draw_layout_xf`] (solid = máxima compat).
pub fn draw_layout_brush_xf(
    scene: &mut vello::Scene,
    layout: &parley::Layout<()>,
    brush: &Brush,
    transform: vello::kurbo::Affine,
) {
    for line in layout.lines() {
        for item in line.items() {
            if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                let run = glyph_run.run();
                let font = run.font().clone();
                let font_size = run.font_size();
                scene
                    .draw_glyphs(&font)
                    .font_size(font_size)
                    .brush(brush)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Texto que envuelve a muchas líneas en un ancho angosto.
    const LARGO: &str =
        "palabras varias que envuelven en bastantes renglones cuando el ancho \
         disponible es realmente angosto y no caben de un solo tirón";

    fn n_lineas(ts: &mut Typesetter, max_lines: Option<usize>, ellipsis: bool) -> usize {
        ts.layout_clamped(
            LARGO,
            14.0,
            Some(120.0),
            Alignment::Start,
            1.2,
            false,
            None,
            400.0,
            max_lines,
            ellipsis,
        )
        .lines()
        .count()
    }

    #[test]
    fn clamp_limita_el_numero_de_lineas() {
        let mut ts = Typesetter::new();
        let libre = n_lineas(&mut ts, None, false);
        assert!(libre > 2, "el fixture debe envolver a >2 líneas (dio {libre})");
        // Con clamp, nunca más que el límite — con o sin ellipsis.
        assert_eq!(n_lineas(&mut ts, Some(1), false), 1);
        assert_eq!(n_lineas(&mut ts, Some(1), true), 1);
        assert!(n_lineas(&mut ts, Some(2), true) <= 2);
        // max_lines None ⇒ sin límite (idéntico a layout).
        assert_eq!(n_lineas(&mut ts, None, true), libre);
    }

    #[test]
    fn clamp_no_trunca_si_ya_cabe() {
        let mut ts = Typesetter::new();
        // "Hola" cabe en una línea: pedir 3 no debe inventar truncado.
        let lay = ts.layout_clamped(
            "Hola", 14.0, Some(200.0), Alignment::Start, 1.2, false, None, 400.0, Some(3), true,
        );
        assert_eq!(lay.lines().count(), 1);
    }
}
