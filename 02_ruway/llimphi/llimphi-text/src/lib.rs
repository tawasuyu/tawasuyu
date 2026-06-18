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
    /// Caché de shaping: `[`Self::layout`]` es el único chokepoint por el que
    /// pasan medición y pintado (vía `layout_clamped`), y se invoca por cada
    /// nodo de texto en **cada** redraw — dos veces (medir + pintar). Shapear
    /// con parley (font matching, bidi, clusters, line break) es lo caro; el
    /// `parley::Layout` resultante es `Clone`. Cacheamos por los parámetros
    /// que lo determinan y clonamos en el hit: durante scroll/tipeo, el texto
    /// que no cambió no se re-shapea.
    cache: ShapeCache,
    cache_hits: u64,
    cache_misses: u64,
}

/// Estadísticas del caché de shaping (evidencia/benchmark). `entries` es el
/// total vivo entre las dos generaciones.
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
}

/// Clave de caché: todos los parámetros que determinan un `layout`. Los `f32`
/// van por `to_bits` para ser `Hash + Eq` exactos (sin problemas de NaN/−0.0:
/// comparamos los bits crudos, no el valor numérico). `Alignment` se mapea a
/// un tag `u8` porque su enum no deriva `Hash`.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ShapeKey {
    text: String,
    size_bits: u32,
    max_width_bits: Option<u32>,
    align: u8,
    line_height_bits: u32,
    italic: bool,
    font_family: Option<String>,
    weight_bits: u32,
    /// Underline activo. parley emite `Decoration` por run cuando este flag
    /// está, así que el layout difiere y el caché tiene que separarlos.
    underline: bool,
    /// Strikethrough activo. Idem `underline`.
    strikethrough: bool,
    /// `letter-spacing` (px extra entre letras). 0 = sin override. Cambia el
    /// shaping/ancho, así que entra en la clave.
    letter_bits: u32,
    /// `word-spacing` (px extra entre palabras). Idem `letter_bits`.
    word_bits: u32,
    /// `overflow-wrap: break-word`/`anywhere`: si está, parley puede partir
    /// dentro de una palabra para que entre en la caja. Cambia el line-break,
    /// así que separa la entrada del caché.
    overflow_wrap: bool,
}

fn align_tag(a: Alignment) -> u8 {
    match a {
        Alignment::Start => 0,
        Alignment::Center => 1,
        Alignment::End => 2,
        Alignment::Justify => 3,
    }
}

/// Caché generacional (LRU aproximado, sin dependencias). Dos mapas: `hot`
/// recibe inserciones y promociones; cuando `hot` llega a `cap`, rota
/// (`cold = hot`, `hot = ∅`) y la generación vieja se descarta. Un hit en
/// `cold` se promueve a `hot`, así lo accedido en la última época sobrevive a
/// la rotación — el texto visible, re-consultado cada frame, queda siempre
/// caliente; lo transitorio (candidatos de elipsis, tooltips) cae solo. Es el
/// patrón de los cachés de glyph/shape de swash/cosmic-text: O(1), sin orden
/// enlazado.
struct ShapeCache {
    hot: std::collections::HashMap<ShapeKey, parley::Layout<()>>,
    cold: std::collections::HashMap<ShapeKey, parley::Layout<()>>,
    cap: usize,
}

impl ShapeCache {
    fn new(cap: usize) -> Self {
        Self {
            hot: std::collections::HashMap::new(),
            cold: std::collections::HashMap::new(),
            cap,
        }
    }

    /// Devuelve un clon del layout cacheado si existe, promoviendo desde
    /// `cold` a `hot` en el camino.
    fn get(&mut self, key: &ShapeKey) -> Option<parley::Layout<()>> {
        if let Some(v) = self.hot.get(key) {
            return Some(v.clone());
        }
        // Hit frío: sacalo de cold y reinsertalo en hot (promoción). Una sola
        // clonación: el clon queda en hot, el original se devuelve al caller.
        if let Some(v) = self.cold.remove(key) {
            self.hot.insert(key.clone(), v.clone());
            return Some(v);
        }
        None
    }

    fn put(&mut self, key: ShapeKey, layout: parley::Layout<()>) {
        if self.hot.len() >= self.cap {
            // Rotá la generación: lo no reaccedido desde la última rotación
            // (quedó sólo en cold) se libera acá.
            self.cold = std::mem::take(&mut self.hot);
        }
        self.hot.insert(key, layout);
    }

    fn clear(&mut self) {
        self.hot.clear();
        self.cold.clear();
    }

    fn entries(&self) -> usize {
        self.hot.len() + self.cold.len()
    }
}

/// Capacidad de la generación caliente antes de rotar. 512 layouts cubre con
/// holgura el texto visible de una UI densa (un editor de ~50 líneas + chrome)
/// sin retener de más. La memoria real es ~2× (dos generaciones).
const SHAPE_CACHE_CAP: usize = 512;

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

/// **Inter** embebida como **fuente de UI por defecto** (SIL OFL 1.1, libre y
/// redistribuible — ver `assets/Inter-LICENSE.txt`). Inter es una grotesca
/// neo-humanista diseñada específicamente para interfaces a tamaños chicos:
/// caja alta de la x, aperturas amplias y espaciado parejo. Es el look 2026
/// que queremos de fábrica, sin depender de que el sistema tenga una sans
/// linda instalada (en una instalación pelada el default de fontique podía
/// caer en Liberation/Adwaita, que envejecen mal). La enganchamos como
/// primera familia del genérico `sans-serif` (ver [`Typesetter::install_ui_font`]),
/// que es lo que parley resuelve cuando el bloque no pide `font_family`. El
/// fallback por-script sigue intacto: símbolos via DejaVu, CJK/árabe/etc. via
/// las fuentes del sistema.
const INTER_SANS: &[u8] = include_bytes!("../assets/Inter-Regular.ttf");

/// Fuente monoespaciada embebida (Liberation Mono, SIL OFL — metric-
/// compatible con Courier). Va embebida para que *cualquier* app Llimphi
/// pueda pedir ancho fijo (output de terminal, IDE-text, tablas que
/// columnean) sin depender de que el sistema tenga una mono instalada.
/// Se referencia por su nombre de familia con [`MONOSPACE`].
const LIBERATION_MONO: &[u8] = include_bytes!("../assets/LiberationMono.ttf");

/// Bytes de la fuente **monospace embebida** (Liberation Mono TTF). Pública
/// para que otros crates (p. ej. `llimphi-widget-terminal`, que necesita
/// rasterizar glifos para su atlas GPU) usen exactamente la misma fuente
/// que el render normal, sin volver a embeber el archivo.
pub const MONO_FONT_BYTES: &[u8] = LIBERATION_MONO;

/// Nombre de familia de la fuente monoespaciada embebida. Pasalo como
/// `font_family: Some(llimphi_text::MONOSPACE)` en un [`TextBlock`] (o el
/// `font_family` de `layout`) para render de ancho fijo garantizado.
pub const MONOSPACE: &str = "Liberation Mono";

/// Nombre de familia de la fuente de UI embebida ([Inter](https://rsms.me/inter/)).
/// Es el default proporcional cuando un bloque **no** especifica `font_family`
/// (la enganchamos como primera familia del genérico `sans-serif`). Exponemos
/// el nombre por si un caller quiere pedirla explícitamente.
pub const UI_SANS: &str = "Inter";

impl Typesetter {
    pub fn new() -> Self {
        let mut font_cx = parley::FontContext::new();
        Self::install_ui_font(&mut font_cx);
        Self::install_symbol_fallback(&mut font_cx);
        Self::install_monospace(&mut font_cx);
        Self {
            font_cx,
            layout_cx: parley::LayoutContext::new(),
            runs_cx: parley::LayoutContext::new(),
            cache: ShapeCache::new(SHAPE_CACHE_CAP),
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    /// Registra **Inter** y la pone como **primera familia del genérico
    /// `sans-serif`**. Ese genérico es lo que parley resuelve cuando un bloque
    /// no especifica `font_family` (su default es `FontStack::Source("sans-serif")`),
    /// así que con esto toda app Llimphi tipografía en Inter de fábrica sin
    /// tocar una línea de su código, y sin depender de la sans del sistema.
    /// Usamos `append_*` (no `set_*`) para no borrar las familias que el SO ya
    /// asociaba al genérico: Inter va primero, el resto queda detrás como
    /// respaldo. La cobertura de scripts no-latinos / símbolos sigue saliendo
    /// del fallback por-script (CJK del sistema, símbolos de DejaVu). Si una
    /// app pide otra familia explícita, gana esa. Best-effort: si el registro
    /// falla, el texto sigue con la sans del sistema.
    fn install_ui_font(font_cx: &mut parley::FontContext) {
        use parley::fontique::{Blob, GenericFamily};
        let blob = Blob::new(std::sync::Arc::new(INTER_SANS));
        let registered = font_cx.collection.register_fonts(blob, None);
        if let Some((family_id, _)) = registered.first() {
            // Las familias actuales del genérico (las del sistema) van detrás:
            // Inter primero, luego el respaldo previo.
            let existing: Vec<_> = font_cx
                .collection
                .generic_families(GenericFamily::SansSerif)
                .collect();
            font_cx.collection.set_generic_families(
                GenericFamily::SansSerif,
                std::iter::once(*family_id).chain(existing),
            );
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

    /// Registra la fuente monoespaciada embebida (Liberation Mono) bajo su
    /// nombre de familia [`MONOSPACE`], para que `FontStack::Source`
    /// (`font_family: Some(MONOSPACE)`) la resuelva aunque el sistema no
    /// tenga ninguna mono instalada. Best-effort: si falla, los callers que
    /// pidan monospace caen al fallback de fontique (mono del sistema, o la
    /// proporcional si no hay) — el texto sigue, sólo pierde el ancho fijo.
    fn install_monospace(font_cx: &mut parley::FontContext) {
        use parley::fontique::Blob;
        let blob = Blob::new(std::sync::Arc::new(LIBERATION_MONO));
        font_cx.collection.register_fonts(blob, None);
    }

    /// Acceso al `FontContext` por si se necesita registrar fuentes extra
    /// o cambiar la stack de fallback. **Invalida el caché de shaping**: tocar
    /// el set de fuentes o el fallback puede cambiar el resultado de cualquier
    /// layout, así que descartamos lo cacheado (operación rara, de setup).
    pub fn font_context_mut(&mut self) -> &mut parley::FontContext {
        self.cache.clear();
        &mut self.font_cx
    }

    /// Estadísticas del caché de shaping (hits/misses acumulados + entradas
    /// vivas). Para benchmark/evidencia; no afecta el render.
    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            hits: self.cache_hits,
            misses: self.cache_misses,
            entries: self.cache.entries(),
        }
    }

    /// Construye y resuelve un `parley::Layout`. Aplica `font_size`,
    /// `line_height` (multiplicador del font_size), `max_width` (line
    /// break), `alignment` y `weight` (peso de fuente CSS: 400 normal,
    /// 700 bold). `italic`=true selecciona la variante italic/oblique de
    /// la fuente activa (vía `parley::FontStyle`). `underline`/`strikethrough`
    /// activan la decoración global del bloque — parley deja la metadata
    /// (offset + grosor) en cada `Run` y el pintado (`draw_layout_*`) emite
    /// el rect correspondiente sobre la línea base.
    /// API pública 12-arg (sin `overflow-wrap`): la usan showreels, canvas,
    /// hit-testing de selección, etc. Delega en [`Self::layout_inner`] con
    /// `overflow_wrap = false` (la palabra larga desborda, comportamiento
    /// histórico). El quiebre dentro de palabra entra sólo por `layout_clamped`
    /// (camino del compositor), para no propagar el flag a todos los callers.
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
        underline: bool,
        strikethrough: bool,
        letter_spacing: f32,
        word_spacing: f32,
    ) -> parley::Layout<()> {
        self.layout_inner(
            text, size_px, max_width, alignment, line_height, italic, font_family, weight,
            underline, strikethrough, letter_spacing, word_spacing, false,
        )
    }

    /// Impl real del shaping con el flag `overflow_wrap` (CSS
    /// `overflow-wrap: break-word`/`anywhere`). Privado: sólo lo invocan
    /// [`Self::layout`] (con `false`) y [`Self::layout_clamped`] (con el valor
    /// del estilo). Así la firma pública 12-arg no cambia y los ~20 callers de
    /// showreels/canvas siguen compilando sin tocar.
    #[allow(clippy::too_many_arguments)]
    fn layout_inner(
        &mut self,
        text: &str,
        size_px: f32,
        max_width: Option<f32>,
        alignment: Alignment,
        line_height: f32,
        italic: bool,
        font_family: Option<&str>,
        weight: f32,
        underline: bool,
        strikethrough: bool,
        letter_spacing: f32,
        word_spacing: f32,
        overflow_wrap: bool,
    ) -> parley::Layout<()> {
        // Caché de shaping: clave por todos los parámetros que determinan el
        // layout. En el hit clonamos el `parley::Layout` (memcpy de vectores,
        // ~órdenes de magnitud más barato que re-shapear). El `String`/clave
        // que se aloca para consultar es un costo menor frente al shaping que
        // evita; mantener la firma `&str` no fuerza alloc en el caller.
        let key = ShapeKey {
            text: text.to_string(),
            size_bits: size_px.to_bits(),
            max_width_bits: max_width.map(f32::to_bits),
            align: align_tag(alignment),
            line_height_bits: line_height.to_bits(),
            italic,
            font_family: font_family.map(str::to_string),
            weight_bits: weight.to_bits(),
            underline,
            strikethrough,
            letter_bits: letter_spacing.to_bits(),
            word_bits: word_spacing.to_bits(),
            overflow_wrap,
        };
        if let Some(hit) = self.cache.get(&key) {
            self.cache_hits += 1;
            return hit;
        }
        self.cache_misses += 1;
        let mut builder =
            self.layout_cx
                .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontSize(size_px));
        builder.push_default(parley::StyleProperty::LineHeight(
            parley::LineHeight::FontSizeRelative(line_height),
        ));
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
        if underline {
            builder.push_default(parley::StyleProperty::Underline(true));
        }
        if strikethrough {
            builder.push_default(parley::StyleProperty::Strikethrough(true));
        }
        // `letter-spacing`/`word-spacing` (px extra). 0 = sin override (normal).
        if letter_spacing != 0.0 {
            builder.push_default(parley::StyleProperty::LetterSpacing(letter_spacing));
        }
        if word_spacing != 0.0 {
            builder.push_default(parley::StyleProperty::WordSpacing(word_spacing));
        }
        // `overflow-wrap: break-word`/`anywhere`: habilita la partición dentro
        // de una palabra cuando no hay otra oportunidad de quiebre en la línea
        // (un token más ancho que la caja). `Anywhere` cubre ambos valores CSS
        // — su única diferencia con `BreakWord` es el min-content sizing, sin
        // efecto visible en el wrap del bloque. Sin el flag (normal) parley deja
        // desbordar la palabra larga (comportamiento previo).
        if overflow_wrap {
            builder.push_default(parley::StyleProperty::OverflowWrap(
                parley::OverflowWrap::Anywhere,
            ));
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);
        layout.align(
            max_width,
            alignment.into(),
            parley::AlignmentOptions::default(),
        );
        self.cache.put(key, layout.clone());
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
        underline: bool,
        strikethrough: bool,
        letter_spacing: f32,
        word_spacing: f32,
        overflow_wrap: bool,
    ) -> parley::Layout<()> {
        let full = self.layout_inner(
            text, size_px, max_width, alignment, line_height, italic, font_family, weight,
            underline, strikethrough, letter_spacing, word_spacing, overflow_wrap,
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
            return self.layout_inner(
                base, size_px, max_width, alignment, line_height, italic, font_family, weight,
                underline, strikethrough, letter_spacing, word_spacing, overflow_wrap,
            );
        }
        // Recortá graphemes del final hasta que `base…` vuelva a caber en
        // `limit` líneas (apilar el `…` puede empujar una palabra a una línea
        // extra). Acotado: cada vuelta quita ≥1 char.
        let mut s = base.to_string();
        loop {
            let candidate = format!("{s}…");
            let lay = self.layout_inner(
                &candidate, size_px, max_width, alignment, line_height, italic, font_family,
                weight, underline, strikethrough, letter_spacing, word_spacing, overflow_wrap,
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
    #[allow(clippy::too_many_arguments)]
    pub fn layout_runs(
        &mut self,
        text: &str,
        size_px: f32,
        default_color: Color,
        runs: &[(usize, usize, Color)],
        alignment: Alignment,
        line_height: f32,
        weight: f32,
        underline: bool,
        strikethrough: bool,
    ) -> parley::Layout<RunBrush> {
        let mut builder = self
            .runs_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontSize(size_px));
        builder.push_default(parley::StyleProperty::LineHeight(
            parley::LineHeight::FontSizeRelative(line_height),
        ));
        if weight != 400.0 {
            builder.push_default(parley::StyleProperty::FontWeight(
                parley::FontWeight::new(weight),
            ));
        }
        builder.push_default(parley::StyleProperty::Brush(RunBrush(default_color)));
        if underline {
            builder.push_default(parley::StyleProperty::Underline(true));
        }
        if strikethrough {
            builder.push_default(parley::StyleProperty::Strikethrough(true));
        }
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

    /// Construye un layout **RichText**: defaults a nivel bloque + un
    /// arreglo de [`TextSpan`] que sobreescriben tamaño/peso/italic/familia/
    /// color/decoración **por rango de bytes**. A diferencia de
    /// [`Self::layout_runs`] (sólo color, sin wrap), este camino:
    ///
    /// - permite `max_width` (envuelve a párrafo);
    /// - aplica los siete `StyleProperty` por rango;
    /// - usa el mismo `runs_cx` (`RunBrush`), así puede convivir con el
    ///   pintado multicolor.
    ///
    /// **Sin caché** en v1 (a diferencia de `layout`/`layout_clamped`): el
    /// RichText típico cambia frame-a-frame (cursor de editor, hover de
    /// link), y la clave de caché de un span-set arbitrario es pesada.
    /// Reusa todo el shaping interno de parley, que ya es rápido para
    /// párrafos de la magnitud de una UI.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_spans(
        &mut self,
        text: &str,
        size_px: f32,
        default_color: Color,
        weight: f32,
        line_height: f32,
        italic: bool,
        font_family: Option<&str>,
        underline: bool,
        strikethrough: bool,
        spans: &[TextSpan],
        max_width: Option<f32>,
        alignment: Alignment,
    ) -> parley::Layout<RunBrush> {
        let mut builder = self
            .runs_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontSize(size_px));
        builder.push_default(parley::StyleProperty::LineHeight(
            parley::LineHeight::FontSizeRelative(line_height),
        ));
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
            builder.push_default(parley::StyleProperty::FontStack(
                parley::FontStack::Source(std::borrow::Cow::Borrowed(ff)),
            ));
        }
        builder.push_default(parley::StyleProperty::Brush(RunBrush(default_color)));
        if underline {
            builder.push_default(parley::StyleProperty::Underline(true));
        }
        if strikethrough {
            builder.push_default(parley::StyleProperty::Strikethrough(true));
        }
        let len = text.len();
        for span in spans {
            if span.start >= span.end || span.end > len {
                continue;
            }
            let range = span.start..span.end;
            let s = &span.style;
            if let Some(v) = s.size_px {
                builder.push(parley::StyleProperty::FontSize(v), range.clone());
            }
            if let Some(v) = s.weight {
                builder.push(
                    parley::StyleProperty::FontWeight(parley::FontWeight::new(v)),
                    range.clone(),
                );
            }
            if let Some(v) = s.italic {
                let style = if v {
                    parley::FontStyle::Italic
                } else {
                    parley::FontStyle::Normal
                };
                builder.push(parley::StyleProperty::FontStyle(style), range.clone());
            }
            if let Some(ff) = s.font_family.as_deref() {
                builder.push(
                    parley::StyleProperty::FontStack(parley::FontStack::Source(
                        std::borrow::Cow::Owned(ff.to_string()),
                    )),
                    range.clone(),
                );
            }
            if let Some(c) = s.color {
                builder.push(parley::StyleProperty::Brush(RunBrush(c)), range.clone());
            }
            if let Some(v) = s.underline {
                builder.push(parley::StyleProperty::Underline(v), range.clone());
            }
            if let Some(v) = s.strikethrough {
                builder.push(parley::StyleProperty::Strikethrough(v), range.clone());
            }
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

/// Overrides de estilo aplicables a un **rango de bytes** dentro de un
/// bloque de texto, para `Typesetter::layout_spans` (RichText). Cada
/// campo es opcional: `None` hereda del default del bloque. La granularidad
/// es por bytes (convención de parley), igual que el `runs` multicolor.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct TextSpanStyle {
    /// Tamaño de fuente (CSS `font-size`). El reshape recalcula el alto
    /// de la línea afectada.
    pub size_px: Option<f32>,
    /// Peso de fuente (400 = normal, 700 = bold).
    pub weight: Option<f32>,
    /// Italic on/off.
    pub italic: Option<bool>,
    /// Family CSS-like ("Helvetica, sans-serif"). Útil para `code` inline
    /// (forzar monospace en una palabra).
    pub font_family: Option<String>,
    /// Color del texto (gana sobre el `default_color` del bloque).
    pub color: Option<Color>,
    /// Subrayado on/off.
    pub underline: Option<bool>,
    /// Tachado on/off.
    pub strikethrough: Option<bool>,
}

/// Un span de RichText: rango de bytes `[start, end)` + overrides de
/// estilo (`style`). Los rangos pueden superponerse — parley aplica los
/// `StyleProperty` en orden de inserción, así el caller debería pushar de
/// menor a mayor especificidad.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
    pub style: TextSpanStyle,
}

impl TextSpan {
    pub fn new(start: usize, end: usize, style: TextSpanStyle) -> Self {
        Self { start, end, style }
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
            Alignment::Center => parley::Alignment::Center,
            Alignment::End => parley::Alignment::End,
            Alignment::Justify => parley::Alignment::Justify,
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
        // Decoración tampoco viaja por `TextBlock`: la activa el compositor
        // por nodo según `TextSpec::{underline,strikethrough}`.
        false,
        false,
        // `letter-spacing`/`word-spacing` tampoco viajan por `TextBlock`; el
        // compositor los pasa por su camino directo (`layout_clamped`).
        0.0,
        0.0,
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
                paint_decoration(scene, &glyph_run, brush, transform);
            }
        }
    }
}

/// Pinta las decoraciones (`underline`/`strikethrough`) del run si las trae
/// del shaping. El offset que devuelve parley sigue la convención OpenType
/// (positivo = sobre la línea base en font-space, eje Y arriba); en
/// coordenadas de pantalla (Y abajo) el rect va a `baseline - offset`. El
/// `transform` es el mismo que se usa para los glifos, así la decoración
/// hereda el scroll/rotación/zoom del subárbol.
fn paint_decoration<B: parley::Brush>(
    scene: &mut vello::Scene,
    glyph_run: &parley::GlyphRun<'_, B>,
    brush: &Brush,
    transform: vello::kurbo::Affine,
) {
    let style = glyph_run.style();
    let run = glyph_run.run();
    let metrics = run.metrics();
    let x = glyph_run.offset() as f64;
    let baseline = glyph_run.baseline() as f64;
    let advance = glyph_run.advance() as f64;
    if let Some(dec) = &style.underline {
        let offset = dec.offset.unwrap_or(metrics.underline_offset) as f64;
        let size = dec.size.unwrap_or(metrics.underline_size) as f64;
        let y0 = baseline - offset;
        let rect = vello::kurbo::Rect::new(x, y0, x + advance, y0 + size);
        scene.fill(peniko::Fill::NonZero, transform, brush, None, &rect);
    }
    if let Some(dec) = &style.strikethrough {
        let offset = dec.offset.unwrap_or(metrics.strikethrough_offset) as f64;
        let size = dec.size.unwrap_or(metrics.strikethrough_size) as f64;
        let y0 = baseline - offset;
        let rect = vello::kurbo::Rect::new(x, y0, x + advance, y0 + size);
        scene.fill(peniko::Fill::NonZero, transform, brush, None, &rect);
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
    draw_layout_runs_xf(scene, layout, vello::kurbo::Affine::translate(origin));
}

/// Igual que [`draw_layout_runs`] pero con una **afín completa** en vez de sólo
/// un desplazamiento — el equivalente multicolor de [`draw_layout_xf`]. Lo
/// necesita el compositor para que el texto multicolor herede la
/// transformación acumulada del subárbol (scroll/rotación del padre): sin esto,
/// el texto con `runs` se pintaba en coords de layout crudas, **ignorando** el
/// transform, y se desalineaba del resto (p. ej. el cuerpo coloreado del shell
/// no seguía el scroll del panel). El origen del layout (0,0) lo mapea
/// `transform`; las posiciones de glifo se aplican en ese espacio.
pub fn draw_layout_runs_xf(
    scene: &mut vello::Scene,
    layout: &parley::Layout<RunBrush>,
    transform: vello::kurbo::Affine,
) {
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
                paint_decoration(scene, &glyph_run, &brush, transform);
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
            false,
            false,
            0.0,
            0.0,
            false,
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
    fn letter_y_word_spacing_ensanchan_la_medida() {
        // letter-spacing y word-spacing agregan px al ancho del shaping; 0 es
        // el baseline (normal). Prueba directa del feature (Fase 7.1252).
        let mut ts = Typesetter::new();
        let w = |ts: &mut Typesetter, ls: f32, ws: f32| {
            measurement(&ts.layout(
                "hola mundo cruel", 14.0, None, Alignment::Start, 1.2, false, None, 400.0, false,
                false, ls, ws,
            ))
            .width
        };
        let base = w(&mut ts, 0.0, 0.0);
        let con_letter = w(&mut ts, 4.0, 0.0);
        let con_word = w(&mut ts, 0.0, 10.0);
        assert!(con_letter > base, "letter-spacing ensancha ({con_letter} > {base})");
        assert!(con_word > base, "word-spacing ensancha ({con_word} > {base})");
    }

    #[test]
    fn clamp_no_trunca_si_ya_cabe() {
        let mut ts = Typesetter::new();
        // "Hola" cabe en una línea: pedir 3 no debe inventar truncado.
        let lay = ts.layout_clamped(
            "Hola", 14.0, Some(200.0), Alignment::Start, 1.2, false, None, 400.0, Some(3), true,
            false, false, 0.0, 0.0, false,
        );
        assert_eq!(lay.lines().count(), 1);
    }

    /// El caché no debe cambiar el resultado: misma medida con o sin hit, y la
    /// segunda llamada idéntica tiene que pegar en el caché (hit), no re-shapear.
    #[test]
    fn cache_es_transparente_y_pega() {
        let mut ts = Typesetter::new();
        let m1 = {
            let l = ts.layout(LARGO, 14.0, Some(120.0), Alignment::Start, 1.2, false, None, 400.0, false, false, 0.0, 0.0);
            (l.width(), l.height(), l.lines().count())
        };
        let s1 = ts.cache_stats();
        assert_eq!(s1.misses, 1, "primera vez = miss");
        assert_eq!(s1.hits, 0);
        // Misma llamada exacta: debe ser hit y dar la misma geometría.
        let m2 = {
            let l = ts.layout(LARGO, 14.0, Some(120.0), Alignment::Start, 1.2, false, None, 400.0, false, false, 0.0, 0.0);
            (l.width(), l.height(), l.lines().count())
        };
        let s2 = ts.cache_stats();
        assert_eq!(s2.hits, 1, "segunda vez idéntica = hit");
        assert_eq!(s2.misses, 1, "no hubo nuevo miss");
        assert_eq!(m1, m2, "el layout cacheado es idéntico al fresco");
        // Cambiar un parámetro (ancho) es una clave distinta: miss nuevo.
        let _ = ts.layout(LARGO, 14.0, Some(80.0), Alignment::Start, 1.2, false, None, 400.0, false, false, 0.0, 0.0);
        assert_eq!(ts.cache_stats().misses, 2, "otro ancho = otra clave");
    }

    /// `font_context_mut` invalida el caché (cambiar fuentes puede alterar el
    /// shaping): la siguiente llamada idéntica vuelve a ser miss.
    #[test]
    fn font_context_mut_invalida_el_cache() {
        let mut ts = Typesetter::new();
        let _ = ts.layout("hola", 14.0, None, Alignment::Start, 1.2, false, None, 400.0, false, false, 0.0, 0.0);
        assert_eq!(ts.cache_stats().entries, 1);
        let _ = ts.font_context_mut();
        assert_eq!(ts.cache_stats().entries, 0, "el caché quedó vacío");
        let _ = ts.layout("hola", 14.0, None, Alignment::Start, 1.2, false, None, 400.0, false, false, 0.0, 0.0);
        assert_eq!(ts.cache_stats().misses, 2, "post-invalidación = miss");
    }

    /// Decoración (underline / strikethrough): el flag de entrada debe
    /// llegar al `parley::Layout` como `style.underline`/`style.strikethrough`
    /// presentes en cada run, y el caché debe distinguir su clave (mismo
    /// texto con vs sin decoración = entradas separadas).
    #[test]
    fn underline_y_strikethrough_se_propagan_al_layout() {
        let mut ts = Typesetter::new();
        let with_dec = ts.layout(
            "Hola", 14.0, None, Alignment::Start, 1.2, false, None, 400.0, true, true, 0.0, 0.0,
        );
        // Caminamos los runs del layout y verificamos que cada GlyphRun trae
        // ambas decoraciones marcadas (no usamos `is_some` directo porque
        // `Layout::lines/items` exige iterar para llegar al Style).
        let mut visto_u = false;
        let mut visto_s = false;
        for line in with_dec.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(gr) = item {
                    if gr.style().underline.is_some() {
                        visto_u = true;
                    }
                    if gr.style().strikethrough.is_some() {
                        visto_s = true;
                    }
                }
            }
        }
        assert!(visto_u, "underline=true ⇒ Decoration en al menos un run");
        assert!(visto_s, "strikethrough=true ⇒ Decoration en al menos un run");

        // Sin decoración el layout no las trae.
        let plain = ts.layout(
            "Hola", 14.0, None, Alignment::Start, 1.2, false, None, 400.0, false, false, 0.0, 0.0,
        );
        for line in plain.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(gr) = item {
                    assert!(gr.style().underline.is_none(), "sin underline=true ⇒ None");
                    assert!(gr.style().strikethrough.is_none(), "sin strikethrough=true ⇒ None");
                }
            }
        }

        // Caché: dos misses (uno por cada variante), no se pisan.
        let s = ts.cache_stats();
        assert!(s.misses >= 2, "claves distintas por decoración ⇒ misses separados");
    }

    /// Mecánica generacional: al pasar `cap`, `hot` rota a `cold`; un ítem
    /// reaccedido se promueve y sobrevive a la siguiente rotación.
    #[test]
    fn cache_generacional_promueve_y_rota() {
        let mut c = ShapeCache::new(2);
        let mk = |s: &str| ShapeKey {
            text: s.to_string(),
            size_bits: 0,
            max_width_bits: None,
            align: 0,
            line_height_bits: 0,
            italic: false,
            font_family: None,
            weight_bits: 0,
            underline: false,
            strikethrough: false,
            letter_bits: 0,
            word_bits: 0,
            overflow_wrap: false,
        };
        // Layouts vacíos como valores (sólo nos importa la presencia de claves).
        let dummy = parley::Layout::<()>::default;
        c.put(mk("a"), dummy());
        c.put(mk("b"), dummy());
        // "a" sigue caliente; lo accedemos para que se quede al rotar.
        assert!(c.get(&mk("a")).is_some());
        // Tercer insert: hot llegó a cap(2) → rota (a,b→cold), c entra a hot.
        c.put(mk("c"), dummy());
        // "a" estaba en cold; get lo encuentra y lo promueve a hot.
        assert!(c.get(&mk("a")).is_some(), "ítem reaccedido sobrevive la rotación");
        // "b" no se reaccedió: cae en la siguiente rotación.
        c.put(mk("d"), dummy()); // hot = {c, a-promovido}? -> al llegar a cap rota
        // Tras suficientes rotaciones sin tocar "b", desaparece.
        c.put(mk("e"), dummy());
        c.put(mk("f"), dummy());
        assert!(c.get(&mk("b")).is_none(), "ítem nunca reaccedido se libera");
    }
}
