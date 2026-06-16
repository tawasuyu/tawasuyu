//! **Showreel** del motor Llimphi — para r/rust. NO es eye-candy abstracto:
//! es una vitrina de **widgets reales** del toolkit, *en acción*. Cada frame
//! reconstruye un árbol `View` con widgets de verdad (`llimphi-widget-switch`,
//! `-slider`, `-progress`, `-button`, `-segmented`) cuyo **estado** se deriva
//! del tiempo normalizado `t∈[0,1]` — el toggle se enciende, el slider sube,
//! la barra avanza, el segmented cambia de pestaña. Se montan con el `mount` /
//! `paint` / `compute_with_measure` reales (taffy + parley + vello), idéntico
//! al eventloop. No se dibujan a mano: si existe el widget, se usa el widget.
//!
//! El render es **headless y determinista** (sin reloj, sin runtime, sin
//! winit): frame `i` de `N` → `t = i/(N-1)` → View → layout → vello::Scene →
//! wgpu → PNG. El cold-open (trazo bezier draw-on) y el wordmark de cierre
//! son `paint_with` sobre un nodo full-screen, superpuestos sobre los widgets.
//!
//! ```text
//! cargo run -p llimphi-compositor --example showreel --release -- \
//!     [out_dir] [n_frames] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames`, `n_frames=360`, `W=1600`, `H=900`.

use std::fs::{create_dir_all, File};
use std::io::BufWriter;

use llimphi_compositor::{
    measure_text_node, mount, paint, DragPhase, PaintRect, Shadow, View,
};
use llimphi_hal::{wgpu, Hal};
use llimphi_layout::taffy;
use llimphi_layout::taffy::prelude::{
    auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style,
};
use llimphi_layout::taffy::Rect;
use llimphi_layout::LayoutTree;
use llimphi_raster::peniko::{self, Color, Gradient};
use llimphi_raster::{vello, Renderer};
use llimphi_text::{draw_layout_brush_xf, measurement, Alignment, Typesetter};
use llimphi_theme::motion;
use vello::kurbo::{Affine, BezPath, Circle, Point, Stroke};

use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_progress::{linear_progress_view, radial_progress_view};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_switch::{switch_view, SwitchPalette};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

// ───────────────────────── utilidades ─────────────────────────

/// Color con alpha escalado a `a∈[0,1]` (para fade del overlay vector).
fn with_alpha(c: Color, a: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, a.clamp(0.0, 1.0)])
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Reescala `t` desde el subintervalo `[lo,hi]` de la timeline a `[0,1]`,
/// clampado. Fuera del intervalo devuelve 0 (antes) o 1 (después).
fn seg(t: f32, lo: f32, hi: f32) -> f32 {
    ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
}

// ───────────────────────── tema / paleta ─────────────────────────

#[derive(Clone)]
struct Skin {
    theme: llimphi_theme::Theme,
    accent: Color,
    panel: Color,
    panel_hi: Color,
    border: Color,
    border_accent: Color,
    fg: Color,
    fg_muted: Color,
    bg: Color,
}

// ───────────────────────── geometría de las tarjetas ─────────────────────────

#[derive(Clone, Copy)]
struct CardRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl CardRect {
    fn lerp(self, b: CardRect, t: f64) -> CardRect {
        CardRect {
            x: lerp(self.x, b.x, t),
            y: lerp(self.y, b.y, t),
            w: lerp(self.w, b.w, t),
            h: lerp(self.h, b.h, t),
        }
    }
}

const N_CARDS: usize = 6;

/// Disposición A — grilla 3×2 centrada (beat de ensamblado).
fn layout_grid(cw: f64, ch: f64) -> [CardRect; N_CARDS] {
    let card_w = 360.0;
    let card_h = 196.0;
    let gap = 40.0;
    let cols = 3.0;
    let rows = 2.0;
    let total_w = cols * card_w + (cols - 1.0) * gap;
    let total_h = rows * card_h + (rows - 1.0) * gap;
    let x0 = (cw - total_w) / 2.0;
    let y0 = (ch - total_h) / 2.0;
    let mut out = [CardRect { x: 0.0, y: 0.0, w: card_w, h: card_h }; N_CARDS];
    for (i, c) in out.iter_mut().enumerate() {
        let col = (i % 3) as f64;
        let row = (i / 3) as f64;
        c.x = x0 + col * (card_w + gap);
        c.y = y0 + row * (card_h + gap);
    }
    out
}

/// Disposición B — fila única ancha, alturas escalonadas (beat de morph).
/// Los MISMOS widgets adentro, otra geometría: "cualquier layout con taffy".
fn layout_row(cw: f64, ch: f64) -> [CardRect; N_CARDS] {
    let gap = 22.0;
    let n = N_CARDS as f64;
    let card_w = (cw - 2.0 * 90.0 - (n - 1.0) * gap) / n;
    let x0 = 90.0;
    let cy = ch / 2.0;
    // alturas tipo "ecualizador" — silueta dinámica al reacomodar.
    let hs = [240.0, 300.0, 210.0, 320.0, 260.0, 230.0];
    let mut out = [CardRect { x: 0.0, y: 0.0, w: card_w, h: 220.0 }; N_CARDS];
    for (i, c) in out.iter_mut().enumerate() {
        c.x = x0 + i as f64 * (card_w + gap);
        c.h = hs[i];
        c.y = cy - c.h / 2.0;
        c.w = card_w;
    }
    out
}

// ───────────────────────── contenido de cada card ─────────────────────────

/// Header de card: chip de acento + título.
fn card_header(title: &str, s: &Skin, accented: bool) -> View<()> {
    let chip = View::new(Style {
        size: Size { width: length(28.0), height: length(8.0) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .radius(4.0)
    .fill(if accented { s.accent } else { s.fg_muted });
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(10.0), height: length(0.0) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![
        chip,
        View::new(Style { flex_grow: 1.0, ..Default::default() })
            .text_aligned(title.to_string(), 12.5, s.fg_muted, Alignment::Start)
            .bold(),
    ])
}

/// Línea de "valor" grande (estado legible) bajo el control.
fn value_line(text: &str, color: Color, size: f32) -> View<()> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(size + 6.0) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(text.to_string(), size, color, Alignment::Start)
    .bold()
}

/// Cuerpo de una card según índice — cada una hospeda widgets REALES cuyo
/// estado deriva de `p∈[0,1]` (progreso del beat de widgets).
fn card_body(i: usize, p: f32, s: &Skin) -> Vec<View<()>> {
    match i {
        // ── 0: Switch (off → on) ──────────────────────────────────────
        0 => {
            // El thumb se desliza en una rampa centrada del beat.
            let prog = motion::ease_in_out_cubic(seg(p, 0.15, 0.6));
            let on = prog > 0.5;
            let pal = SwitchPalette::from_theme(&s.theme);
            let sw_row = View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(26.0) },
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::Center),
                gap: Size { width: length(14.0), height: length(0.0) },
                ..Default::default()
            })
            .children(vec![
                switch_view(prog, (), &pal),
                View::new(Style { flex_grow: 1.0, ..Default::default() })
                    .text_aligned(
                        "Sincronizar".to_string(),
                        13.0,
                        s.fg,
                        Alignment::Start,
                    ),
            ]);
            vec![
                card_header("switch", s, true),
                spacer(8.0),
                sw_row,
                spacer(10.0),
                value_line(if on { "ENCENDIDO" } else { "apagado" }, if on { s.accent } else { s.fg_muted }, 22.0),
            ]
        }
        // ── 1: Slider (20% → 75%) ─────────────────────────────────────
        1 => {
            let v = lerp(0.2, 0.75, motion::ease_in_out_cubic(seg(p, 0.1, 0.7)) as f64) as f32;
            let mut pal = SliderPalette::from_theme(&s.theme);
            pal.track_width = 168.0;
            pal.label_width = 0.0;
            pal.value_width = 50.0;
            pal.track_thickness = 8.0;
            pal.row_height = 26.0;
            let sld = slider_view::<(), _>(
                "",
                v,
                0.0,
                1.0,
                &pal,
                |_phase: DragPhase, _dv: f32| None,
            );
            vec![
                card_header("slider", s, false),
                spacer(10.0),
                sld,
                spacer(12.0),
                value_line(&format!("{:>3.0}%", v * 100.0), s.fg, 26.0),
            ]
        }
        // ── 2: Linear progress (avanza) ───────────────────────────────
        2 => {
            let v = motion::ease_out_cubic(seg(p, 0.05, 0.85));
            let bar = View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(12.0) },
                position: Position::Relative,
                ..Default::default()
            })
            .fill(s.theme.bg_button)
            .radius(6.0)
            .children(vec![linear_progress_view(
                v,
                s.theme.bg_button,
                s.accent,
                12.0,
            )]);
            vec![
                card_header("progress", s, true),
                spacer(14.0),
                bar,
                spacer(14.0),
                value_line(&format!("{:>3.0}%  ·  compilando", v * 100.0), s.fg_muted, 13.0),
            ]
        }
        // ── 3: Segmented control (cambia de pestaña activa) ───────────
        3 => {
            // 3 segmentos; el activo recorre 0 → 1 → 2 a lo largo del beat.
            let phase = seg(p, 0.1, 0.95);
            let active = ((phase * 3.0).floor() as usize).min(2);
            let labels = ["Día", "Semana", "Mes"];
            let pal = SegmentedPalette::from_theme(&s.theme);
            let seg_ctrl = segmented_view::<(), _>(&labels, active, |_| (), &pal);
            vec![
                card_header("segmented", s, false),
                spacer(14.0),
                seg_ctrl,
                spacer(14.0),
                value_line(labels[active], s.accent, 22.0),
            ]
        }
        // ── 4: Botones (primario teal + ghost) ────────────────────────
        4 => {
            // Paleta primaria: fondo teal, texto sobre fondo.
            let mut prim = ButtonPalette::from_theme(&s.theme);
            prim.bg = s.accent;
            prim.bg_hover = s.accent;
            prim.fg = s.bg; // texto oscuro sobre teal
            prim.radius = 8.0;
            let mut ghost = ButtonPalette::from_theme(&s.theme);
            ghost.bg = s.theme.bg_button;
            ghost.fg = s.fg;
            ghost.radius = 8.0;
            let row = View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(38.0) },
                flex_direction: FlexDirection::Row,
                gap: Size { width: length(12.0), height: length(0.0) },
                ..Default::default()
            })
            .children(vec![
                View::new(Style {
                    size: Size { width: length(132.0), height: length(38.0) },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .children(vec![button_view("Regenerar", &prim, ())]),
                View::new(Style {
                    size: Size { width: length(110.0), height: length(38.0) },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .children(vec![button_view("Difundir", &ghost, ())]),
            ]);
            vec![
                card_header("button", s, true),
                spacer(14.0),
                row,
                spacer(14.0),
                value_line("primario · ghost", s.fg_muted, 13.0),
            ]
        }
        // ── 5: Radial progress (anillo que se llena) ──────────────────
        _ => {
            let v = motion::ease_out_cubic(seg(p, 0.1, 0.9));
            let ring = View::new(Style {
                size: Size { width: length(96.0), height: length(96.0) },
                position: Position::Relative,
                ..Default::default()
            })
            .children(vec![radial_progress_view(
                v,
                s.theme.bg_button,
                s.accent,
                0.14,
            )]);
            let ring_row = View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(96.0) },
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .children(vec![ring]);
            vec![
                card_header("radial", s, false),
                spacer(6.0),
                ring_row,
            ]
        }
    }
}

/// Espaciador vertical de alto fijo.
fn spacer(h: f32) -> View<()> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(h) },
        flex_shrink: 0.0,
        ..Default::default()
    })
}

/// Una card como contenedor absoluto, hospedando widgets reales.
fn card_view(i: usize, rect: CardRect, alpha: f32, scale: f64, p: f32, s: &Skin) -> View<()> {
    let accented = i == 0 || i == 2 || i == 4;
    let border_col = if accented { s.border_accent } else { s.border };

    // Pop de entrada: escala desde el centro de la card.
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    let xf = Affine::translate((cx, cy)) * Affine::scale(scale) * Affine::translate((-cx, -cy));

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(rect.x as f32),
            top: length(rect.y as f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(rect.w as f32), height: length(rect.h as f32) },
        flex_direction: FlexDirection::Column,
        gap: Size { width: length(0.0), height: length(0.0) },
        padding: Rect {
            left: length(22.0),
            right: length(22.0),
            top: length(20.0),
            bottom: length(18.0),
        },
        ..Default::default()
    })
    .fill_gradient(
        Gradient::new_linear(Point::new(0.0, 0.0), Point::new(0.0, 1.0))
            .with_stops([s.panel_hi, s.panel].as_slice()),
    )
    .radius(18.0)
    .border(if accented { 1.4 } else { 1.0 }, border_col)
    .shadow(Shadow::soft(120, 26.0).offset(0.0, 12.0))
    .transform(xf)
    .alpha(alpha)
    .children(card_body(i, p, s))
}

// ───────────────────────── overlays vector (cold-open + wordmark) ─────────────────────────

/// Curva bezier "firma" del cold-open.
fn signature_path(cw: f64, ch: f64) -> BezPath {
    let cx = cw / 2.0;
    let cy = ch / 2.0;
    let mut p = BezPath::new();
    p.move_to((cx - 360.0, cy + 40.0));
    p.curve_to(
        (cx - 150.0, cy - 220.0),
        (cx + 150.0, cy + 220.0),
        (cx + 360.0, cy - 40.0),
    );
    p
}

/// Recorta un `BezPath` cúbico a su fracción inicial `prog`. Devuelve la
/// cabeza del trazo para anclar el punto teal.
fn trim_path(full: &BezPath, prog: f64) -> (BezPath, Point) {
    use vello::kurbo::ParamCurve;
    let prog = prog.clamp(0.0, 1.0);
    let mut cubic = None;
    let mut start = Point::ZERO;
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

/// Dibuja los overlays vector (cold-open + wordmark + punto firma) sobre un
/// nodo full-screen, en función de `t`. Los widgets ya están pintados debajo.
fn draw_overlays(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64, s: &Skin) {
    // ── COLD OPEN (0–12%) ──────────────────────────────────────────
    let b1 = seg(t, 0.0, 0.12);
    let line_vis = 1.0 - seg(t, 0.12, 0.20);
    if line_vis > 0.001 {
        let path = signature_path(cw, ch);
        let draw_on = motion::ease_out_cubic(seg(t, 0.02, 0.13)) as f64;
        let (trimmed, head) = trim_path(&path, draw_on);
        let line_col = with_alpha(s.accent, 0.9 * line_vis);
        scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, line_col, None, &trimmed);
        let pop = motion::ease_out_back(b1);
        let r = (4.0 + 7.0 * pop as f64).max(0.0);
        let dot_a = (b1 * line_vis).clamp(0.0, 1.0);
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.18 * dot_a),
            None,
            &Circle::new(head, r * 3.2),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, dot_a),
            None,
            &Circle::new(head, r),
        );
    }

    // ── WORDMARK (82–100%) ─────────────────────────────────────────
    let word_in = seg(t, 0.84, 0.95);
    let word_a = motion::ease_out_cubic(word_in);
    if word_a > 0.001 {
        let size = 132.0_f32;
        let layout = ts.layout(
            "Llimphi", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false,
        );
        let m = measurement(&layout);
        let rise = lerp(24.0, 0.0, word_a as f64);
        let ox = (cw - m.width as f64) / 2.0;
        let oy = (ch - m.height as f64) / 2.0 - 18.0 + rise;
        let brush = peniko::Brush::Solid(with_alpha(s.fg, word_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));

        let sub_a = motion::ease_out_cubic(seg(t, 0.88, 0.99));
        if sub_a > 0.001 {
            let ssz = 26.0_f32;
            let sub = ts.layout(
                "a Rust GUI framework", ssz, None, Alignment::Start, 1.0, false, None, 400.0,
                false, false,
            );
            let sm = measurement(&sub);
            let dot_r = 6.0;
            let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
            let sx = (cw - block_w) / 2.0;
            let sy = oy + m.height as f64 + 18.0;
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(s.accent, sub_a),
                None,
                &Circle::new(Point::new(sx + dot_r, sy + ssz as f64 * 0.42), dot_r as f64),
            );
            let sbrush = peniko::Brush::Solid(with_alpha(s.fg_muted, sub_a));
            draw_layout_brush_xf(
                scene,
                &sub,
                &sbrush,
                Affine::translate((sx + dot_r * 2.0 + 14.0, sy)),
            );
        }
    }

    // ── punto teal de firma (esquina inf-der), ancla de marca ───────
    let corner_a = seg(t, 0.04, 0.12) * (1.0 - seg(t, 0.80, 0.86));
    if corner_a > 0.001 {
        let cx = cw - 54.0;
        let cy = ch - 54.0;
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.16 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 18.0),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.9 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 6.0),
        );
    }
}

// ───────────────────────── la escena por frame ─────────────────────────

/// Construye el árbol `View` completo del frame `t`: las cards con widgets
/// reales (con su estado derivado de t) + un nodo overlay full-screen que
/// pinta cold-open / wordmark encima.
fn build_view(t: f32, cw: f64, ch: f64, s: &Skin) -> View<()> {
    let grid = layout_grid(cw, ch);
    let row = layout_row(cw, ch);

    // Progreso del "estado" de los widgets (toggle/slider/progress/…).
    let widget_p = seg(t, 0.16, 0.58);
    // Morph grid → fila (58–80%).
    let morph = motion::ease_in_out_cubic(seg(t, 0.60, 0.80)) as f64;
    // Fade-out de las cards antes del wordmark.
    let cards_fade = 1.0 - seg(t, 0.80, 0.86);

    let mut children: Vec<View<()>> = Vec::new();

    if cards_fade > 0.001 {
        for i in 0..N_CARDS {
            // Stagger de entrada: cada card arranca con retraso incremental.
            let delay = i as f32 * 0.035;
            let enter = motion::ease_out_back(seg(t, 0.12 + delay, 0.12 + delay + 0.16));
            if enter <= 0.001 {
                continue;
            }
            let rect = grid[i].lerp(row[i], morph);
            let scale = lerp(0.88, 1.0, enter.min(1.0) as f64);
            let alpha = (enter.min(1.0) * cards_fade).clamp(0.0, 1.0);
            children.push(card_view(i, rect, alpha, scale, widget_p, s));
        }
    }

    // Nodo overlay full-screen para el vector (cold-open + wordmark).
    let overlay = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0),
            top: length(0.0),
            right: length(0.0),
            bottom: length(0.0),
        },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .paint_with({
        let s = s.clone();
        move |scene, ts, _rect: PaintRect| {
            draw_overlays(scene, ts, t, cw, ch, &s);
        }
    });
    children.push(overlay);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        position: Position::Relative,
        ..Default::default()
    })
    .fill(s.bg)
    .children(children)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| "showreel_frames".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(360);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    create_dir_all(&out_dir).expect("mkdir out_dir");

    let theme = llimphi_theme::Theme::by_name("Tawa").expect("tema Tawa");
    let accent = Color::from_rgba8(0x2B, 0xD9, 0xA6, 0xFF); // teal #2BD9A6 (acento firma)
    let skin = Skin {
        accent,
        panel: theme.bg_panel,
        panel_hi: theme.bg_button,
        border: theme.border,
        border_accent: with_alpha(accent, 0.55),
        fg: theme.fg_text,
        fg_muted: theme.fg_muted,
        bg: theme.bg_app,
        theme,
    };
    let [br, bg, bb, _] = skin.bg.components;
    let base = Color::from_rgba8((br * 255.0) as u8, (bg * 255.0) as u8, (bb * 255.0) as u8, 255);

    // GPU una sola vez; reusar device/renderer/target/buffer para los N frames.
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("showreel"),
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

        let root = build_view(t, cw, ch, &skin);

        // view → layout (con medición de texto real) → scene — idéntico al eventloop.
        let mut layout = LayoutTree::new();
        let mounted = mount(&mut layout, root);
        let computed = {
            let tmap = &mounted.text_measures;
            layout
                .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                        None => taffy::Size::ZERO,
                    }
                })
                .expect("layout")
        };
        let mut scene = vello::Scene::new();
        paint(&mut scene, &mounted, &computed, &mut ts, None, None);

        renderer
            .render_to_view(&hal, &scene, &view, w, h, base)
            .expect("render_to_view");
        let path = format!("{out_dir}/frame_{i:04}.png");
        write_png(&hal, &target, &path, w, h);
        if i % 30 == 0 || i == n - 1 {
            eprintln!("showreel: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("showreel: {n} frames en {out_dir}/ ({w}x{h})");
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
