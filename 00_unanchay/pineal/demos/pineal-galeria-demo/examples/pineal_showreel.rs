//! **Showreel** de `pineal` — montaje de tipos de gráfico para el README
//! del standalone (audiencia r/rust). NO es eye-candy abstracto: cada beat
//! invoca un **painter REAL** del catálogo de pineal (`paint_bars`,
//! `paint_heatmap`, `paint_contours`, `paint_pie`, `paint_radar`,
//! `paint_treemap`, `paint_hexbin`, `paint_sankey`, el grafo force-directed
//! de `pineal-mesh` y los candlesticks de `pineal-financial`) sobre el
//! `Canvas` de `pineal-render` (backend vello/wgpu de Llimphi).
//!
//! El render es **headless y determinista** (sin reloj, sin runtime, sin
//! winit): frame `i` de `N` → `t = i/(N-1)` → `vello::Scene` → wgpu → PNG.
//! El stage central cruza de un gráfico al siguiente con **cross-fade**
//! (capas vello con alpha), y los datos de cada gráfico se derivan de su
//! progreso local para que *animen* mientras están en pantalla. Cold-open
//! sobrio (trazo bezier draw-on) + wordmark final **"pineal"** con
//! subtítulo "data visualization, in Rust".
//!
//! ```text
//! cargo run -p pineal-galeria-demo --example pineal_showreel --release -- \
//!     [out_dir] [n_frames] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames_pineal`, `n_frames=300`, `W=1600`, `H=900`.
#![allow(clippy::too_many_arguments)]

use std::fs::{create_dir_all, File};
use std::io::BufWriter;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Point as KPoint, Stroke};
use llimphi_ui::llimphi_raster::peniko::{self, Color as PColor, Fill, Gradient};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{
    draw_layout_brush_xf, measurement, Alignment, Typesetter,
};

use pineal_bars::{paint_bars, Bar, BarStyle, Histogram};
use pineal_cartesian::{ChartViewport, CoordinateSystem};
use pineal_contour::paint_contours;
use pineal_financial::{paint_candlesticks, CandlestickStyle, OhlcBuffer};
use pineal_flow::{compute_layout, paint_sankey, SankeyLink, SankeyNode};
use pineal_heatmap::{paint as paint_heatmap, HeatmapMatrix, Ramp};
use pineal_mesh::{EdgeBuffer, ForceLayout, ForceParams, NodeBuffer};
use pineal_polar::{paint_pie, paint_radar, Slice};
use pineal_render::{Canvas as _, Color, Point, Rect, SceneCanvas, StrokeStyle};
use pineal_treemap::{paint_treemap, Tile};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

// ───────────────────────── utilidades de tiempo / color ─────────────────────────

/// Reescala `t` desde el subintervalo `[lo,hi]` a `[0,1]`, clampado.
fn seg(t: f32, lo: f32, hi: f32) -> f32 {
    ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let f = 2.0 * t - 2.0;
        0.5 * f * f * f + 1.0
    }
}

fn ease_out_cubic(t: f32) -> f32 {
    let f = t - 1.0;
    f * f * f + 1.0
}

fn ease_out_back(t: f32) -> f32 {
    let c1 = 1.70158_f32;
    let c3 = c1 + 1.0;
    let f = t - 1.0;
    1.0 + c3 * f * f * f + c1 * f * f
}

/// `PColor` (peniko) con alpha sobreescrito.
fn palpha(c: PColor, a: f32) -> PColor {
    let [r, g, b, _] = c.components;
    PColor::new([r, g, b, a.clamp(0.0, 1.0)])
}

// ───────────────────────── paleta del showreel ─────────────────────────

struct Skin {
    bg: PColor,
    panel_hi: PColor,
    panel_lo: PColor,
    border: PColor,
    accent: PColor,
    fg: PColor,
    fg_muted: PColor,
    /// Fondo del área de plot dentro del stage (oscuro como en la galería).
    plot_bg: Color,
}

fn skin() -> Skin {
    Skin {
        bg: PColor::from_rgba8(0x0C, 0x0E, 0x12, 0xFF),
        panel_hi: PColor::from_rgba8(0x18, 0x1D, 0x26, 0xFF),
        panel_lo: PColor::from_rgba8(0x10, 0x14, 0x1B, 0xFF),
        border: PColor::from_rgba8(0x2A, 0x31, 0x3E, 0xFF),
        accent: PColor::from_rgba8(0x2B, 0xD9, 0xA6, 0xFF), // teal firma
        fg: PColor::from_rgba8(0xE8, 0xEC, 0xF2, 0xFF),
        fg_muted: PColor::from_rgba8(0x8A, 0x93, 0xA3, 0xFF),
        plot_bg: Color::rgba(0.043, 0.055, 0.078, 1.0),
    }
}

// ───────────────────────── stage central ─────────────────────────

/// Rect (pineal) interior del stage donde pinta el painter, con padding.
fn plot_rect_of(stage: KurboRectLite, pad: f64) -> Rect {
    Rect::new(
        (stage.x + pad) as f32,
        (stage.y + pad) as f32,
        (stage.w - 2.0 * pad) as f32,
        (stage.h - 2.0 * pad) as f32,
    )
}

#[derive(Clone, Copy)]
struct KurboRectLite {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

// ───────────────────────── los 8 beats: cada uno un painter REAL ─────────────────────────
//
// `lp ∈ [0,1]` = progreso local del beat (para animar datos). `area` = rect
// de plot (coords pineal). Todos hablan contra el trait `Canvas` vía
// `SceneCanvas`. El fondo del plot ya está pintado por el caller.

const N_BEATS: usize = 8;

fn beat_label(i: usize) -> (&'static str, &'static str) {
    match i {
        0 => ("bars", "columnas + histograma"),
        1 => ("heatmap", "campo escalar · Viridis"),
        2 => ("contour", "marching squares · 8 niveles"),
        3 => ("polar", "pie / donut + radar"),
        4 => ("treemap", "squarified"),
        5 => ("financial", "candlesticks OHLC"),
        6 => ("flow", "diagrama Sankey"),
        _ => ("mesh", "grafo force-directed"),
    }
}

/// Despacha el painter del beat `i` con progreso local `lp`.
fn paint_beat(i: usize, lp: f32, canvas: &mut SceneCanvas<'_>, area: Rect, plotbg: Color) {
    canvas.fill_rect(area, plotbg);
    match i {
        0 => beat_bars(lp, canvas, area),
        1 => beat_heatmap(lp, canvas, area),
        2 => beat_contour(lp, canvas, area),
        3 => beat_polar(lp, canvas, area),
        4 => beat_treemap(lp, canvas, area),
        5 => beat_financial(lp, canvas, area),
        6 => beat_sankey(lp, canvas, area),
        _ => beat_mesh(lp, canvas, area),
    }
}

/// 0 — bars: barras a la izquierda (con un negativo) + histograma a la
/// derecha. Las alturas crecen desde el baseline con `lp`.
fn beat_bars(lp: f32, canvas: &mut SceneCanvas<'_>, area: Rect) {
    let g = ease_out_cubic(seg(lp, 0.0, 0.85));
    let raw = [4.0, 7.0, 2.0, -3.0, 5.0, 6.5, 3.5, 5.8];
    let bars: Vec<Bar> = raw
        .iter()
        .map(|&v| {
            let c = if v < 0.0 { 0xd08770 } else { 0x2bd9a6 };
            Bar::new(v * g as f64, Color::from_hex(c))
        })
        .collect();
    let half = area.w * 0.5 - 12.0;
    let left = Rect::new(area.x + 8.0, area.y + 8.0, half - 8.0, area.h - 16.0);
    paint_bars(&bars, left, &BarStyle::vertical().with_gap(0.18), canvas);

    // Histograma (gaussiana de un LCG sembrado), altura escalada por lp.
    let mut rng: u32 = 0x0BAD_F00D;
    let mut next = || {
        rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (rng >> 8) as f32 / (1u32 << 24) as f32
    };
    let mut sample = Vec::with_capacity(4000);
    for _ in 0..4000 {
        let gg: f32 = (0..6).map(|_| next()).sum::<f32>() / 6.0;
        sample.push((gg - 0.5) * 6.0);
    }
    let mut hbars = Histogram::new(&sample, 28).to_bars(Color::from_hex(0xb48ead));
    for b in hbars.iter_mut() {
        b.value *= g as f64;
    }
    let right = Rect::new(area.x + half + 16.0, area.y + 8.0, half - 8.0, area.h - 16.0);
    paint_bars(&hbars, right, &BarStyle::vertical().with_gap(0.04), canvas);
}

/// Campo escalar 64×40 animado por `lp` (la fase de las ondas avanza).
fn animated_field(lp: f32) -> HeatmapMatrix {
    const W: usize = 64;
    const H: usize = 40;
    let ph = lp * std::f32::consts::TAU * 0.5;
    let mut m = HeatmapMatrix::new(W, H);
    let mut data = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let v = (x as f32 * 0.22 + ph).sin() + (y as f32 * 0.26 - ph * 0.7).cos()
                + 0.6 * ((x as f32 * 0.1).sin() * (y as f32 * 0.12).cos());
            data.push(v);
        }
    }
    m.replace_data(data);
    m
}

/// 1 — heatmap Viridis del campo animado.
fn beat_heatmap(lp: f32, canvas: &mut SceneCanvas<'_>, area: Rect) {
    let field = animated_field(lp);
    paint_heatmap(&field, Ramp::Viridis, area, canvas);
}

/// 2 — contour: el mismo campo + isolíneas (marching squares). El nº de
/// niveles sube de 3 a 9 con `lp` (las curvas "aparecen").
fn beat_contour(lp: f32, canvas: &mut SceneCanvas<'_>, area: Rect) {
    let field = animated_field(lp * 0.6);
    paint_heatmap(&field, Ramp::Viridis, area, canvas);
    let levels = (3.0 + 6.0 * ease_out_cubic(seg(lp, 0.0, 0.9))) as usize;
    paint_contours(
        &field,
        levels.max(2),
        area,
        Color::rgba(0.35, 0.62, 1.0, 0.9),
        Color::rgba(1.0, 0.42, 0.32, 0.95),
        1.3,
        canvas,
    );
}

/// 3 — polar: donut a la izquierda (barrido por lp) + radar a la derecha
/// (vértices que crecen). Dos painters de pineal-polar en un beat.
fn beat_polar(lp: f32, canvas: &mut SceneCanvas<'_>, area: Rect) {
    let g = ease_out_cubic(seg(lp, 0.0, 0.85));
    // Donut.
    let cx = area.x + area.w * 0.27;
    let cy = area.y + area.h * 0.5;
    let r_out = (area.w.min(area.h) * 0.34).max(20.0);
    let r_in = r_out * 0.5;
    let base = [28.0_f32, 18.0, 14.0, 12.0, 10.0, 8.0];
    let pal = [0x2bd9a6, 0xd08770, 0xa3be8c, 0xebcb8b, 0xb48ead, 0x5e81ac];
    let slices: Vec<Slice> = base
        .iter()
        .zip(pal.iter())
        .map(|(&v, &c)| Slice::new(v * g, Color::from_hex(c)))
        .collect();
    if g > 0.01 {
        paint_pie(&slices, Point::new(cx, cy), r_out, r_in, canvas);
    }
    // Radar.
    let rcx = area.x + area.w * 0.73;
    let rcy = cy;
    let rr = (area.w.min(area.h) * 0.34).max(20.0);
    for step in 1..=4 {
        let t = step as f32 / 4.0;
        let ring: Vec<f32> = (0..=72)
            .flat_map(|k| {
                let a = (k as f32 / 72.0) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                [rcx + (rr * t) * a.cos(), rcy + (rr * t) * a.sin()]
            })
            .collect();
        canvas.stroke_polyline(&ring, StrokeStyle::new(0.6, Color::rgba(0.5, 0.56, 0.66, 0.35)));
    }
    let values: Vec<f32> = [8.0_f32, 6.5, 9.0, 4.0, 7.0, 5.5].iter().map(|v| v * g).collect();
    paint_radar(
        &values,
        10.0,
        Point::new(rcx, rcy),
        rr,
        Color::rgba(0.169, 0.851, 0.651, 0.30),
        StrokeStyle::new(1.8, Color::from_hex(0x2bd9a6)),
        canvas,
    );
}

/// 4 — treemap squarified: 14 tiles, los pesos respiran con `lp`.
fn beat_treemap(lp: f32, canvas: &mut SceneCanvas<'_>, area: Rect) {
    let pal = [
        0x2bd9a6, 0xd08770, 0xa3be8c, 0xebcb8b, 0xb48ead, 0x5e81ac, 0x81a1c1, 0xbf616a,
        0x8fbcbb, 0xd8dee9, 0x88c0d0, 0xebcb8b, 0xa3be8c, 0x5e81ac,
    ];
    let base = [40.0, 28.0, 22.0, 18.0, 14.0, 12.0, 10.0, 8.0, 6.0, 5.0, 4.0, 3.5, 3.0, 2.0];
    let wob = lp * std::f32::consts::TAU;
    let tiles: Vec<Tile> = base
        .iter()
        .zip(pal.iter())
        .enumerate()
        .map(|(k, (&w, &c))| {
            let m = 1.0 + 0.25 * (wob + k as f32 * 0.6).sin();
            Tile::new((w * m) as f64, Color::from_hex(c))
        })
        .collect();
    let inset = Rect::new(area.x + 6.0, area.y + 6.0, area.w - 12.0, area.h - 12.0);
    paint_treemap(&tiles, inset, 3.0, canvas);
}

/// 5 — financial: candlesticks OHLC (random-walk determinista). El nº de
/// velas visibles crece con `lp` (se van "imprimiendo" de izq a der).
fn beat_financial(lp: f32, canvas: &mut SceneCanvas<'_>, area: Rect) {
    const N: usize = 64;
    let mut buf = OhlcBuffer::with_capacity(N);
    let mut rng: u32 = 0xCAFE_1234;
    let mut next = || {
        rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((rng >> 8) as f32 / (1u32 << 24) as f32) - 0.5
    };
    let mut price = 100.0_f32;
    let mut lo_all = f32::MAX;
    let mut hi_all = f32::MIN;
    let shown = ((N as f32) * ease_out_cubic(seg(lp, 0.0, 0.95))).round() as usize;
    let shown = shown.clamp(2, N);
    for i in 0..shown {
        let o = price;
        let drift = next() * 4.0 + 0.25 * (i as f32 * 0.3).sin();
        let c = (o + drift).max(5.0);
        let hi = o.max(c) + next().abs() * 3.0;
        let lo = o.min(c) - next().abs() * 3.0;
        buf.push_values(i as f32, o, hi, lo, c, 1.0);
        price = c;
        lo_all = lo_all.min(lo);
        hi_all = hi_all.max(hi);
    }
    if hi_all <= lo_all {
        return;
    }
    let pad = (hi_all - lo_all) * 0.08;
    let vp = ChartViewport::new(
        -1.0,
        N as f64,
        (lo_all - pad) as f64,
        (hi_all + pad) as f64,
    );
    let plot = Rect::new(area.x + 6.0, area.y + 6.0, area.w - 12.0, area.h - 12.0);
    let cs = CoordinateSystem::new(vp, plot);
    let mut style = CandlestickStyle::default();
    style.bull_color = Color::from_hex(0x2bd9a6);
    style.bear_color = Color::from_hex(0xe06c75);
    style.body_width_ratio = 0.68;
    paint_candlesticks(canvas, &cs, &buf, style);
}

/// 6 — flow: diagrama Sankey de un presupuesto. Los ribbons aparecen con
/// `lp` vía el alpha del color de link.
fn beat_sankey(lp: f32, canvas: &mut SceneCanvas<'_>, area: Rect) {
    let nodes: Vec<SankeyNode> = [
        "Sueldo", "Freelance", "Renta", "Dividendos", "Vivienda", "Comida", "Transporte",
        "Ocio", "Salud", "Ahorro",
    ]
    .iter()
    .map(|n| SankeyNode::new(*n))
    .collect();
    let links: Vec<SankeyLink> = vec![
        SankeyLink { source: 0, target: 4, value: 1200.0 },
        SankeyLink { source: 0, target: 5, value: 600.0 },
        SankeyLink { source: 0, target: 6, value: 250.0 },
        SankeyLink { source: 0, target: 9, value: 950.0 },
        SankeyLink { source: 1, target: 5, value: 200.0 },
        SankeyLink { source: 1, target: 7, value: 300.0 },
        SankeyLink { source: 1, target: 9, value: 400.0 },
        SankeyLink { source: 2, target: 4, value: 400.0 },
        SankeyLink { source: 2, target: 8, value: 150.0 },
        SankeyLink { source: 3, target: 9, value: 350.0 },
        SankeyLink { source: 3, target: 7, value: 80.0 },
    ];
    let inset = Rect::new(area.x + 16.0, area.y + 16.0, area.w - 32.0, area.h - 32.0);
    let layout = compute_layout(&nodes, &links, inset, 16.0, 8.0);
    let ribbon_a = 0.10 + 0.40 * ease_out_cubic(seg(lp, 0.0, 0.85));
    paint_sankey(
        &layout,
        Color::from_hex(0xe5e9f0),
        Color::rgba(0.169, 0.851, 0.651, ribbon_a),
        canvas,
    );
}

/// 7 — mesh: grafo force-directed pre-relajado, que rota lentamente con
/// `lp` para que se note la profundidad de la maraña.
fn beat_mesh(lp: f32, canvas: &mut SceneCanvas<'_>, area: Rect) {
    let g = relaxed_graph();
    let cx = area.x + area.w * 0.5;
    let cy = area.y + area.h * 0.5;
    let scale = (area.w.min(area.h) / 170.0).max(0.5);
    let ang = lp * std::f32::consts::TAU * 0.15;
    let (sa, ca) = ang.sin_cos();
    let rot = |x: f32, y: f32| -> (f32, f32) {
        (cx + (x * ca - y * sa) * scale, cy + (x * sa + y * ca) * scale)
    };
    let edge = StrokeStyle::new(1.0, Color::rgba(0.55, 0.62, 0.7, 0.4));
    for (u, v) in g.edges.iter() {
        let (xu, yu) = g.nodes.pos(u);
        let (xv, yv) = g.nodes.pos(v);
        let (ax, ay) = rot(xu, yu);
        let (bx, by) = rot(xv, yv);
        canvas.stroke_line(Point::new(ax, ay), Point::new(bx, by), edge);
    }
    let n = g.nodes.len();
    for i in 0..n {
        let (x, y) = g.nodes.pos(i);
        let r = g.nodes.radius(i) * scale;
        let (px, py) = rot(x, y);
        let color = if i < N_RING { Color::from_hex(0x2bd9a6) } else { Color::from_hex(0xa3be8c) };
        canvas.fill_rect(Rect::new(px - r, py - r, r * 2.0, r * 2.0), color);
    }
}

// ── grafo del beat mesh (mismo armado que pineal-mesh-demo / la galería) ──

const N_RING: usize = 12;
const N_SAT: usize = 12;

struct Graph {
    nodes: NodeBuffer,
    edges: EdgeBuffer,
}

fn relaxed_graph() -> Graph {
    let mut nodes = NodeBuffer::new();
    for i in 0..N_RING {
        let a = (i as f32 / N_RING as f32) * std::f32::consts::TAU;
        nodes.push(20.0 * a.cos(), 20.0 * a.sin(), 6.0);
    }
    for i in 0..N_SAT {
        let a = (i as f32 / N_SAT as f32) * std::f32::consts::TAU + 0.13;
        nodes.push(60.0 * a.cos(), 60.0 * a.sin(), 4.5);
    }
    let mut edges = EdgeBuffer::new();
    for i in 0..N_RING {
        edges.push(i, (i + 1) % N_RING);
    }
    for i in 0..N_RING {
        edges.push(i, (i + 3) % N_RING);
    }
    for i in 0..N_SAT {
        edges.push(i, N_RING + i);
    }
    let mut sim = ForceLayout::new(ForceParams { k: 38.0, temperature: 60.0, cooling: 0.985 });
    for _ in 0..400 {
        let _ = sim.step(&mut nodes, &edges);
    }
    Graph { nodes, edges }
}

// ───────────────────────── overlays vector (cold-open + wordmark + chrome) ─────────────────────────

fn signature_path(cw: f64, ch: f64) -> BezPath {
    let cx = cw / 2.0;
    let cy = ch / 2.0;
    let mut p = BezPath::new();
    p.move_to((cx - 380.0, cy + 30.0));
    p.curve_to(
        (cx - 150.0, cy - 200.0),
        (cx + 150.0, cy + 200.0),
        (cx + 380.0, cy - 30.0),
    );
    p
}

fn trim_path(full: &BezPath, prog: f64) -> (BezPath, KPoint) {
    use vello::kurbo::ParamCurve;
    let prog = prog.clamp(0.0, 1.0);
    let mut cubic = None;
    let mut start = KPoint::ZERO;
    for el in full.elements() {
        match el {
            vello::kurbo::PathEl::MoveTo(p) => start = *p,
            vello::kurbo::PathEl::CurveTo(c1, c2, p) => {
                cubic = Some(vello::kurbo::CubicBez::new(start, *c1, *c2, *p));
            }
            _ => {}
        }
    }
    let mut out = BezPath::new();
    let mut head = start;
    if let Some(cb) = cubic {
        out.move_to(cb.p0);
        let steps = 96;
        for i in 1..=steps {
            let u = (i as f64 / steps as f64) * prog;
            let pt = cb.eval(u);
            out.line_to(pt);
            head = pt;
        }
    }
    (out, head)
}

// ───────────────────────── la escena por frame ─────────────────────────

fn render_frame(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64, s: &Skin) {
    // Fondo (gradiente sutil).
    let bg_rect = vello::kurbo::Rect::new(0.0, 0.0, cw, ch);
    let grad = Gradient::new_linear(KPoint::new(0.0, 0.0), KPoint::new(0.0, ch))
        .with_stops([s.panel_lo, s.bg].as_slice());
    scene.fill(Fill::NonZero, Affine::IDENTITY, &grad, None, &bg_rect);

    // Geometría del stage central (gran panel donde viven los gráficos).
    let margin_x = 200.0_f64;
    let stage_top = 178.0_f64;
    let stage_bottom = 120.0_f64;
    let stage = KurboRectLite {
        x: margin_x,
        y: stage_top,
        w: cw - 2.0 * margin_x,
        h: ch - stage_top - stage_bottom,
    };

    // Aparición del chrome (panel) en el cold-open; desaparición antes del wordmark.
    let chrome_in = ease_out_cubic(seg(t, 0.06, 0.16));
    let chrome_out = 1.0 - ease_in_out_cubic(seg(t, 0.86, 0.93));
    let chrome_a = (chrome_in * chrome_out).clamp(0.0, 1.0);

    // Ventana de los beats en la timeline.
    let beats_lo = 0.12_f32;
    let beats_hi = 0.86_f32;

    if chrome_a > 0.001 {
        // Panel del stage (card con borde + gradiente + sombra simulada).
        let r = vello::kurbo::RoundedRect::new(
            stage.x,
            stage.y,
            stage.x + stage.w,
            stage.y + stage.h,
            18.0,
        );
        // Sombra simple.
        let shadow = vello::kurbo::RoundedRect::new(
            stage.x,
            stage.y + 10.0,
            stage.x + stage.w,
            stage.y + stage.h + 10.0,
            18.0,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, palpha(PColor::BLACK, 0.45 * chrome_a), None, &shadow);
        let pgrad = Gradient::new_linear(KPoint::new(0.0, stage.y), KPoint::new(0.0, stage.y + stage.h))
            .with_stops([palpha(s.panel_hi, chrome_a), palpha(s.panel_lo, chrome_a)].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &pgrad, None, &r);
        scene.stroke(&Stroke::new(1.2), Affine::IDENTITY, palpha(s.border, chrome_a), None, &r);

        // ── beats: cross-fade entre gráfico saliente y entrante ──
        let phase = seg(t, beats_lo, beats_hi); // 0..1 sobre todos los beats
        let scaled = phase * N_BEATS as f32;
        let cur = (scaled.floor() as usize).min(N_BEATS - 1);
        let frac = scaled - cur as f32; // 0..1 dentro del beat actual
        // Cross-fade en el último 22% de cada beat hacia el siguiente.
        let xf = seg(frac, 0.78, 1.0);

        let plot = plot_rect_of(stage, 18.0);
        let clip = vello::kurbo::RoundedRect::new(
            plot.x as f64,
            plot.y as f64,
            (plot.x + plot.w) as f64,
            (plot.y + plot.h) as f64,
            10.0,
        );

        // Progreso local "vivo" del beat actual: 0..1 a lo largo de su tramo.
        let lp_cur = frac;
        // Gráfico actual (se desvanece durante el cross-fade final).
        let a_cur = (1.0 - xf) * chrome_a;
        if a_cur > 0.004 {
            scene.push_layer(Fill::NonZero, peniko::Mix::Normal, a_cur, Affine::IDENTITY, &clip);
            {
                let mut canvas = SceneCanvas::new(scene, ts);
                paint_beat(cur, lp_cur, &mut canvas, plot, s.plot_bg);
            }
            scene.pop_layer();
        }
        // Gráfico entrante (aparece durante el cross-fade).
        if xf > 0.004 && cur + 1 < N_BEATS {
            let a_next = xf * chrome_a;
            scene.push_layer(Fill::NonZero, peniko::Mix::Normal, a_next, Affine::IDENTITY, &clip);
            {
                let mut canvas = SceneCanvas::new(scene, ts);
                // El entrante arranca su animación desde 0.
                paint_beat(cur + 1, 0.0, &mut canvas, plot, s.plot_bg);
            }
            scene.pop_layer();
        }

        // ── chrome textual: título del beat + subtítulo + barra de progreso ──
        let (name, subtitle) = beat_label(cur);
        let head_a = chrome_a * (1.0 - seg(frac, 0.86, 1.0)); // pequeño parpadeo al cambiar
        let head_a = head_a.max(chrome_a * 0.0);
        // Título arriba del stage.
        let tl = ts.layout(name, 30.0, None, Alignment::Start, 1.0, false, None, 800.0, false, false, 0.0, 0.0);
        let m = measurement(&tl);
        let tx = stage.x + 6.0;
        let ty = stage.y - 48.0;
        // Punto teal de acento a la izquierda del título.
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palpha(s.accent, head_a),
            None,
            &Circle::new(KPoint::new(tx + 6.0, ty + m.height as f64 * 0.5), 6.0),
        );
        let brush = peniko::Brush::Solid(palpha(s.fg, head_a));
        draw_layout_brush_xf(scene, &tl, &brush, Affine::translate((tx + 22.0, ty)));
        // Subtítulo a la derecha del título.
        let sl = ts.layout(subtitle, 15.0, None, Alignment::Start, 1.0, false, None, 400.0, false, false, 0.0, 0.0);
        let sbrush = peniko::Brush::Solid(palpha(s.fg_muted, head_a));
        draw_layout_brush_xf(
            scene,
            &sl,
            &sbrush,
            Affine::translate((tx + 28.0 + m.width as f64, ty + 12.0)),
        );

        // Barra de progreso de la timeline (debajo del stage).
        let bar_y = stage.y + stage.h + 30.0;
        let bar_x = stage.x;
        let bar_w = stage.w;
        let track = vello::kurbo::RoundedRect::new(bar_x, bar_y, bar_x + bar_w, bar_y + 4.0, 2.0);
        scene.fill(Fill::NonZero, Affine::IDENTITY, palpha(s.border, chrome_a), None, &track);
        let fillw = bar_w * phase as f64;
        if fillw > 1.0 {
            let fr = vello::kurbo::RoundedRect::new(bar_x, bar_y, bar_x + fillw, bar_y + 4.0, 2.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, palpha(s.accent, chrome_a), None, &fr);
        }
        // Marcas de los beats (tics).
        for k in 0..=N_BEATS {
            let mx = bar_x + bar_w * (k as f64 / N_BEATS as f64);
            let on = (k as f32 / N_BEATS as f32) <= phase + 0.001;
            let col = if on { s.accent } else { s.border };
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palpha(col, chrome_a),
                None,
                &Circle::new(KPoint::new(mx, bar_y + 2.0), 3.0),
            );
        }

        // Wordmark pequeño persistente arriba a la izquierda (marca).
        let wm = ts.layout("pineal", 22.0, None, Alignment::Start, 1.0, false, None, 800.0, false, false, 0.0, 0.0);
        let wmb = peniko::Brush::Solid(palpha(s.fg, chrome_a * 0.9));
        draw_layout_brush_xf(scene, &wm, &wmb, Affine::translate((margin_x, 60.0)));
        let tagl = ts.layout(
            "data visualization, in Rust",
            13.0, None, Alignment::Start, 1.0, false, None, 400.0, false, false, 0.0, 0.0,
        );
        let tagb = peniko::Brush::Solid(palpha(s.fg_muted, chrome_a * 0.9));
        draw_layout_brush_xf(scene, &tagl, &tagb, Affine::translate((margin_x + 86.0, 68.0)));
    }

    // ── COLD OPEN: trazo bezier draw-on (0–14%) ──
    let line_vis = 1.0 - seg(t, 0.13, 0.20);
    if line_vis > 0.001 && t < 0.21 {
        let path = signature_path(cw, ch);
        let draw_on = ease_out_cubic(seg(t, 0.0, 0.12)) as f64;
        let (trimmed, head) = trim_path(&path, draw_on);
        scene.stroke(
            &Stroke::new(2.2),
            Affine::IDENTITY,
            palpha(s.accent, 0.9 * line_vis),
            None,
            &trimmed,
        );
        let pop = ease_out_back(seg(t, 0.0, 0.1));
        let r = (4.0 + 7.0 * pop as f64).max(0.0);
        let dot_a = (seg(t, 0.0, 0.1) * line_vis).clamp(0.0, 1.0);
        scene.fill(Fill::NonZero, Affine::IDENTITY, palpha(s.accent, 0.18 * dot_a), None, &Circle::new(head, r * 3.2));
        scene.fill(Fill::NonZero, Affine::IDENTITY, palpha(s.accent, dot_a), None, &Circle::new(head, r));
    }

    // ── WORDMARK final (88–100%) ──
    let word_a = ease_out_cubic(seg(t, 0.90, 0.98));
    if word_a > 0.001 {
        let size = 150.0_f32;
        let layout = ts.layout("pineal", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false, 0.0, 0.0);
        let m = measurement(&layout);
        let rise = lerp(28.0, 0.0, word_a as f64);
        let ox = (cw - m.width as f64) / 2.0;
        let oy = (ch - m.height as f64) / 2.0 - 24.0 + rise;
        let brush = peniko::Brush::Solid(palpha(s.fg, word_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));

        let sub_a = ease_out_cubic(seg(t, 0.93, 1.0));
        if sub_a > 0.001 {
            let sy = oy + m.height as f64 + 16.0;
            // Subtítulo con punto teal a la izquierda.
            let sl = ts.layout(
                "data visualization, in Rust",
                28.0, None, Alignment::Start, 1.0, false, None, 400.0, false, false, 0.0, 0.0,
            );
            let sm = measurement(&sl);
            let dot_r = 6.0;
            let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
            let sx = (cw - block_w) / 2.0;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palpha(s.accent, sub_a),
                None,
                &Circle::new(KPoint::new(sx + dot_r, sy + 28.0 * 0.5), dot_r),
            );
            let sbrush = peniko::Brush::Solid(palpha(s.fg_muted, sub_a));
            draw_layout_brush_xf(scene, &sl, &sbrush, Affine::translate((sx + dot_r * 2.0 + 14.0, sy)));
        }
    }
}

// ───────────────────────── main: N frames headless → PNG ─────────────────────────

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| "showreel_frames_pineal".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    create_dir_all(&out_dir).expect("mkdir out_dir");

    let s = skin();
    let [br, bgc, bb, _] = s.bg.components;
    let base = PColor::from_rgba8((br * 255.0) as u8, (bgc * 255.0) as u8, (bb * 255.0) as u8, 255);

    // GPU una sola vez; reusar device/renderer/target para los N frames.
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("showreel-pineal"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut ts = Typesetter::new();
    let cw = w as f64;
    let ch = h as f64;

    for i in 0..n {
        let t = if n <= 1 { 0.0 } else { i as f32 / (n as f32 - 1.0) };
        let mut scene = vello::Scene::new();
        render_frame(&mut scene, &mut ts, t, cw, ch, &s);
        renderer
            .render_to_view(&hal, &scene, &view, w, h, base)
            .expect("render_to_view");
        let path = format!("{out_dir}/frame_{i:04}.png");
        write_png(&hal, &target, &path, w, h);
        if i % 30 == 0 || i == n - 1 {
            eprintln!("pineal_showreel: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("pineal_showreel: {n} frames en {out_dir}/ ({w}x{h})");
}

fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str, w: u32, h: u32) {
    let unpadded = (w * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * h as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for r in 0..h as usize {
        let sidx = r * padded;
        pixels.extend_from_slice(&data[sidx..sidx + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&pixels).unwrap();
}
