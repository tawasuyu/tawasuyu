//! Painters de barras agnósticos: categorías → `fill_rect` contra un
//! `Canvas`. Una unidad geométrica común ([`Axis`]) unifica orientación
//! vertical/horizontal y los tres modos (simple, agrupado, apilado).

use pineal_render::{Canvas, Color, Rect};

/// Una barra: su valor (puede ser negativo) y su color.
#[derive(Debug, Clone, Copy)]
pub struct Bar {
    pub value: f64,
    pub color: Color,
}

impl Bar {
    pub fn new(value: f64, color: Color) -> Self {
        Self { value, color }
    }
}

/// Hacia dónde crecen las barras.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Columnas: el eje de valor es vertical, crecen hacia arriba.
    Vertical,
    /// Barras: el eje de valor es horizontal, crecen hacia la derecha.
    Horizontal,
}

/// Estilo del gráfico de barras.
#[derive(Debug, Clone, Copy)]
pub struct BarStyle {
    pub orientation: Orientation,
    /// Fracción del slot de cada categoría que va a separación, en
    /// `[0, 1)`. `0.0` = barras pegadas; `0.2` = 20 % de aire.
    pub gap_ratio: f32,
    /// Valor del cero. Las barras nacen acá; valores por debajo crecen
    /// en sentido contrario. Normalmente `0.0`.
    pub baseline: f64,
    /// Override del rango de valor. `None` = automático a partir de los
    /// datos (incluyendo siempre el baseline).
    pub range: Option<(f64, f64)>,
}

impl Default for BarStyle {
    fn default() -> Self {
        Self {
            orientation: Orientation::Vertical,
            gap_ratio: 0.18,
            baseline: 0.0,
            range: None,
        }
    }
}

impl BarStyle {
    pub fn vertical() -> Self {
        Self::default()
    }
    pub fn horizontal() -> Self {
        Self {
            orientation: Orientation::Horizontal,
            ..Self::default()
        }
    }
    pub fn with_gap(mut self, gap_ratio: f32) -> Self {
        self.gap_ratio = gap_ratio.clamp(0.0, 0.95);
        self
    }
    pub fn with_baseline(mut self, baseline: f64) -> Self {
        self.baseline = baseline;
        self
    }
    pub fn with_range(mut self, lo: f64, hi: f64) -> Self {
        self.range = Some((lo, hi));
        self
    }
}

/// Geometría compartida: mapea valor → pixel sobre el eje de valor y
/// arma el rect de una barra que ocupa el segmento `[cat_lo, cat_hi]`
/// del eje de categoría y va de `v_from` a `v_to` en el eje de valor.
struct Axis {
    area: Rect,
    vmin: f64,
    vmax: f64,
    orientation: Orientation,
}

impl Axis {
    fn new(area: Rect, vmin: f64, vmax: f64, orientation: Orientation) -> Self {
        // Evita división por cero cuando todos los valores coinciden.
        let (vmin, vmax) = if (vmax - vmin).abs() < f64::EPSILON {
            (vmin - 0.5, vmax + 0.5)
        } else {
            (vmin, vmax)
        };
        Self { area, vmin, vmax, orientation }
    }

    /// Extensión del eje de categoría (donde se reparten los slots).
    fn cat_span(&self) -> (f32, f32) {
        match self.orientation {
            Orientation::Vertical => (self.area.x, self.area.x + self.area.w),
            Orientation::Horizontal => (self.area.y, self.area.y + self.area.h),
        }
    }

    /// Pixel del eje de valor para `v`. En vertical, +valor = arriba
    /// (y menor); en horizontal, +valor = derecha (x mayor).
    fn value_px(&self, v: f64) -> f32 {
        let t = ((v - self.vmin) / (self.vmax - self.vmin)) as f32;
        match self.orientation {
            Orientation::Vertical => self.area.y + self.area.h * (1.0 - t),
            Orientation::Horizontal => self.area.x + self.area.w * t,
        }
    }

    /// Rect de una barra: ocupa `[cat_lo, cat_hi]` en categoría y el
    /// tramo de valor `[v_from, v_to]` (orden indistinto).
    fn bar_rect(&self, cat_lo: f32, cat_hi: f32, v_from: f64, v_to: f64) -> Rect {
        let p0 = self.value_px(v_from);
        let p1 = self.value_px(v_to);
        let (lo, hi) = (p0.min(p1), p0.max(p1));
        match self.orientation {
            Orientation::Vertical => Rect::new(cat_lo, lo, cat_hi - cat_lo, hi - lo),
            Orientation::Horizontal => Rect::new(lo, cat_lo, hi - lo, cat_hi - cat_lo),
        }
    }
}

/// Reparte `n` slots iguales sobre `[span0, span1]` y devuelve el
/// sub-rango `[lo, hi]` del slot `i` ya descontado el gap.
fn slot(span0: f32, span1: f32, n: usize, i: usize, gap_ratio: f32) -> (f32, f32) {
    let total = span1 - span0;
    let w = total / n as f32;
    let pad = w * gap_ratio * 0.5;
    let lo = span0 + w * i as f32 + pad;
    let hi = span0 + w * (i + 1) as f32 - pad;
    (lo, hi)
}

fn auto_range(values: impl Iterator<Item = f64>, baseline: f64) -> (f64, f64) {
    let mut lo = baseline;
    let mut hi = baseline;
    for v in values {
        if v < lo {
            lo = v;
        }
        if v > hi {
            hi = v;
        }
    }
    (lo, hi)
}

/// Dibuja una serie de barras dentro de `area`. Una `fill_rect` por
/// barra; valores negativos crecen al otro lado del baseline.
pub fn paint_bars(bars: &[Bar], area: Rect, style: &BarStyle, canvas: &mut dyn Canvas) {
    if bars.is_empty() {
        return;
    }
    let (vmin, vmax) = style
        .range
        .unwrap_or_else(|| auto_range(bars.iter().map(|b| b.value), style.baseline));
    let axis = Axis::new(area, vmin, vmax, style.orientation);
    let (s0, s1) = axis.cat_span();
    for (i, bar) in bars.iter().enumerate() {
        let (lo, hi) = slot(s0, s1, bars.len(), i, style.gap_ratio);
        let r = axis.bar_rect(lo, hi, style.baseline, bar.value);
        if r.w > 0.0 && r.h > 0.0 {
            canvas.fill_rect(r, bar.color);
        }
    }
}

/// Dibuja varias series agrupadas (clustered): cada categoría es un
/// slot que se subdivide entre las `series.len()` series. `series[k]`
/// debe tener un valor por categoría; series más cortas se rellenan
/// hasta la categoría que tengan.
pub fn paint_grouped(series: &[&[Bar]], area: Rect, style: &BarStyle, canvas: &mut dyn Canvas) {
    let n_series = series.len();
    if n_series == 0 {
        return;
    }
    let n_cats = series.iter().map(|s| s.len()).max().unwrap_or(0);
    if n_cats == 0 {
        return;
    }
    let all = series.iter().flat_map(|s| s.iter().map(|b| b.value));
    let (vmin, vmax) = style.range.unwrap_or_else(|| auto_range(all, style.baseline));
    let axis = Axis::new(area, vmin, vmax, style.orientation);
    let (s0, s1) = axis.cat_span();
    for cat in 0..n_cats {
        // Slot de la categoría (sin gap: el gap se aplica adentro, entre
        // las barras del cluster).
        let (clo, chi) = slot(s0, s1, n_cats, cat, 0.0);
        for (k, serie) in series.iter().enumerate() {
            let Some(bar) = serie.get(cat) else { continue };
            let (lo, hi) = slot(clo, chi, n_series, k, style.gap_ratio);
            let r = axis.bar_rect(lo, hi, style.baseline, bar.value);
            if r.w > 0.0 && r.h > 0.0 {
                canvas.fill_rect(r, bar.color);
            }
        }
    }
}

/// Dibuja barras apiladas: `stacks[c]` son los segmentos de la
/// categoría `c`, acumulados desde el baseline. Pensado para segmentos
/// del mismo signo (lo habitual en stacked bars).
pub fn paint_stacked(stacks: &[&[Bar]], area: Rect, style: &BarStyle, canvas: &mut dyn Canvas) {
    if stacks.is_empty() {
        return;
    }
    // Rango: del baseline al mínimo/máximo acumulado de cada pila.
    let mut lo = style.baseline;
    let mut hi = style.baseline;
    for stack in stacks {
        let mut acc = style.baseline;
        for seg in stack.iter() {
            acc += seg.value;
            lo = lo.min(acc);
            hi = hi.max(acc);
        }
    }
    let (vmin, vmax) = style.range.unwrap_or((lo, hi));
    let axis = Axis::new(area, vmin, vmax, style.orientation);
    let (s0, s1) = axis.cat_span();
    for (c, stack) in stacks.iter().enumerate() {
        let (clo, chi) = slot(s0, s1, stacks.len(), c, style.gap_ratio);
        let mut acc = style.baseline;
        for seg in stack.iter() {
            let from = acc;
            acc += seg.value;
            let r = axis.bar_rect(clo, chi, from, acc);
            if r.w > 0.0 && r.h > 0.0 {
                canvas.fill_rect(r, seg.color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pineal_render::{PlanRecorder, RenderCmd};

    fn fill_rects(rec: PlanRecorder) -> Vec<Rect> {
        rec.into_plan()
            .cmds
            .into_iter()
            .filter_map(|c| match c {
                RenderCmd::FillRect { rect, .. } => Some(rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn one_rect_per_bar() {
        let bars = [
            Bar::new(3.0, Color::WHITE),
            Bar::new(5.0, Color::BLACK),
            Bar::new(1.0, Color::from_hex(0x00ff00)),
        ];
        let mut rec = PlanRecorder::new();
        paint_bars(&bars, Rect::new(0.0, 0.0, 300.0, 200.0), &BarStyle::vertical(), &mut rec);
        assert_eq!(fill_rects(rec).len(), 3);
    }

    #[test]
    fn taller_value_taller_bar() {
        let bars = [Bar::new(1.0, Color::WHITE), Bar::new(4.0, Color::WHITE)];
        let mut rec = PlanRecorder::new();
        paint_bars(&bars, Rect::new(0.0, 0.0, 200.0, 100.0), &BarStyle::vertical(), &mut rec);
        let rects = fill_rects(rec);
        assert!(rects[1].h > rects[0].h, "la barra de mayor valor debe ser más alta");
    }

    #[test]
    fn negative_grows_below_baseline() {
        // Con baseline 0 y rango simétrico, un valor negativo debe quedar
        // por debajo (y mayor) que uno positivo.
        let bars = [Bar::new(2.0, Color::WHITE), Bar::new(-2.0, Color::WHITE)];
        let style = BarStyle::vertical().with_range(-3.0, 3.0);
        let mut rec = PlanRecorder::new();
        paint_bars(&bars, Rect::new(0.0, 0.0, 200.0, 100.0), &style, &mut rec);
        let rects = fill_rects(rec);
        // baseline (v=0) está en el medio (y=50). El positivo arranca
        // arriba del baseline; el negativo abajo.
        assert!(rects[0].y < 50.0, "positivo arriba del baseline");
        assert!(rects[1].y >= 50.0 - f32::EPSILON, "negativo en/abajo del baseline");
    }

    #[test]
    fn horizontal_swaps_axes() {
        let bars = [Bar::new(1.0, Color::WHITE), Bar::new(4.0, Color::WHITE)];
        let mut rec = PlanRecorder::new();
        paint_bars(&bars, Rect::new(0.0, 0.0, 200.0, 100.0), &BarStyle::horizontal(), &mut rec);
        let rects = fill_rects(rec);
        // En horizontal el largo es el ancho (w), no la altura.
        assert!(rects[1].w > rects[0].w, "mayor valor = barra más larga (w)");
    }

    #[test]
    fn grouped_emits_all_bars() {
        let a = [Bar::new(1.0, Color::WHITE), Bar::new(2.0, Color::WHITE)];
        let b = [Bar::new(3.0, Color::BLACK), Bar::new(4.0, Color::BLACK)];
        let series: [&[Bar]; 2] = [&a, &b];
        let mut rec = PlanRecorder::new();
        paint_grouped(&series, Rect::new(0.0, 0.0, 400.0, 200.0), &BarStyle::vertical(), &mut rec);
        assert_eq!(fill_rects(rec).len(), 4);
    }

    #[test]
    fn stacked_segments_dont_overlap() {
        let s0 = [Bar::new(2.0, Color::WHITE), Bar::new(3.0, Color::BLACK)];
        let stacks: [&[Bar]; 1] = [&s0];
        let mut rec = PlanRecorder::new();
        paint_stacked(&stacks, Rect::new(0.0, 0.0, 100.0, 100.0), &BarStyle::vertical(), &mut rec);
        let rects = fill_rects(rec);
        assert_eq!(rects.len(), 2);
        // Apilados verticalmente: el primer segmento (baseline→2) queda
        // debajo del segundo (2→5). Sin solape ⇒ el de abajo empieza
        // donde termina el de arriba (con tolerancia de borde).
        let bottom0 = rects[0].y + rects[0].h;
        let bottom1 = rects[1].y + rects[1].h;
        assert!(bottom1 <= bottom0 + 0.01 && rects[1].y < rects[0].y);
    }
}
