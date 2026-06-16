//! **Showreel** de khipu — notas al olvido / mapa mental: un
//! canvas-como-interfaz que ordena el caos por atención. NO es eye-candy
//! abstracto: anima la representación **real** del mapa de khipu
//! (`src/map.rs::paint_map`, calcada fiel en `examples/pantallazo_mapa.rs`):
//! nodos que respiran por masa viva, filamentos de afinidad del nodo
//! seleccionado (activación por difusión), topónimos de continente y el
//! chip de bautizo de un clúster emergente. El estado se deriva del tiempo
//! normalizado `t∈[0,1]` — los nodos aparecen con stagger, los filamentos se
//! dibujan, la masa late, la cámara hace un zoom semántico y, al cierre, el
//! wordmark «khipu».
//!
//! El render es **headless y determinista** (sin reloj, sin runtime, sin
//! winit): frame `i` de `N` → `t = i/(N-1)` → `View` → layout → `vello::Scene`
//! → wgpu → PNG. Mismo patrón que `llimphi-compositor/examples/showreel.rs` y
//! `pata-llimphi/examples/showreel.rs`.
//!
//! ```text
//! cargo run -p khipu-app --example showreel --release -- [out_dir] [n] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames_khipu`, `n=300`, `W=1600`, `H=900`.

use std::fs::{create_dir_all, File};
use std::io::BufWriter;

use llimphi_theme::{motion, Theme};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::{
    self,
    prelude::{length, percent, Position, Rect, Size, Style},
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle as KurboCircle, Point, Stroke};
use llimphi_ui::llimphi_raster::peniko::{self, Color, Fill};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{
    draw_block, draw_layout_brush_xf, measurement, Alignment, TextBlock, Typesetter,
};
use llimphi_ui::{measure_text_node, mount, paint, PaintRect, View};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

// ───────────────────────── utilidades ─────────────────────────

fn with_alpha(c: Color, a: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, a.clamp(0.0, 1.0)])
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Reescala `t` desde el subintervalo `[lo,hi]` de la timeline a `[0,1]`,
/// clampado. Fuera del intervalo: 0 (antes) o 1 (después).
fn seg(t: f32, lo: f32, hi: f32) -> f32 {
    ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// RGB → HSV → rota H → RGB. Calco de `rotate_hue` (src/map.rs): los matices
/// de clúster salen del accent del theme rotado por golden-ratio.
fn rotate_hue(c: Color, dh: f32) -> Color {
    let [r, g, b, a] = c.components;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { (max - min) / max };
    let h = if (max - min).abs() < 1e-6 {
        0.0
    } else if max == r {
        ((g - b) / (max - min)) % 6.0
    } else if max == g {
        (b - r) / (max - min) + 2.0
    } else {
        (r - g) / (max - min) + 4.0
    };
    let h2 = ((h / 6.0) + dh).rem_euclid(1.0) * 6.0;
    let c2 = v * s;
    let x = c2 * (1.0 - ((h2 % 2.0) - 1.0).abs());
    let (r2, g2, b2) = match h2 as i32 {
        0 => (c2, x, 0.0),
        1 => (x, c2, 0.0),
        2 => (0.0, c2, x),
        3 => (0.0, x, c2),
        4 => (x, 0.0, c2),
        _ => (c2, 0.0, x),
    };
    let m = v - c2;
    Color::new([r2 + m, g2 + m, b2 + m, a])
}

/// Pseudo-random determinista por índice (para el orden de aparición y el
/// jitter del latido), sin dependencias.
fn hash01(i: u32) -> f32 {
    let mut x = i.wrapping_mul(2654435761).wrapping_add(0x9E3779B9);
    x ^= x >> 15;
    x = x.wrapping_mul(0x85EBCA6B);
    x ^= x >> 13;
    (x & 0xFFFFFF) as f32 / 0xFFFFFF as f32
}

// ───────────────────────── el sustrato del mapa ─────────────────────────

/// Un nodo del mapa: domicilio fijo en mundo + datos de pintura. La masa
/// base la modulamos en tiempo de render (el latido).
#[derive(Clone)]
struct Nodo {
    x: f32,
    y: f32,
    /// Masa base "vivida": enciende brillo y tamaño; late con `t`.
    mass: f32,
    visible: bool,
    color: Color,
    label: &'static str,
    cluster: u8,
    /// Orden de aparición (stagger) en el beat de ensamblado.
    seed: u32,
}

/// El elenco de notas — las mismas 20 de `pantallazo_mapa.rs` (tres
/// constelaciones: huerta / lecturas / tareas, sueltas en el anillo, dos
/// bajo el horizonte). Reproduce el dato real del mapa.
fn elenco(accent: Color) -> Vec<Nodo> {
    let c_huerta = accent;
    let c_lecturas = rotate_hue(accent, 0.16);
    let c_tareas = rotate_hue(accent, 0.33);
    let c_suelto = rotate_hue(accent, 0.50);
    let mut idx = 0u32;
    let mut n = |x, y, mass, visible, color, label, cluster| {
        let s = idx;
        idx += 1;
        Nodo { x, y, mass, visible, color, label, cluster, seed: s }
    };
    vec![
        // — huerta (región bautizada, arriba-izquierda) —
        n(-340.0, -180.0, 1.7, true, c_huerta, "trasplantar los tomates", 0),
        n(-265.0, -230.0, 1.1, true, c_huerta, "compost: girar el lunes", 0),
        n(-410.0, -120.0, 0.8, true, c_huerta, "semillas de albahaca", 0),
        n(-290.0, -105.0, 0.5, true, c_huerta, "riego por goteo (plano)", 0),
        n(-380.0, -255.0, 0.3, true, c_huerta, "podar el limonero", 0),
        // — lecturas (región bautizada, derecha) —
        n(300.0, -90.0, 1.4, true, c_lecturas, "Borges: el jardin de se…", 1),
        n(380.0, -30.0, 0.9, true, c_lecturas, "\"la memoria es porosa\"", 1),
        n(245.0, -10.0, 0.7, true, c_lecturas, "releer cap. 3 de Wiener", 1),
        n(355.0, -160.0, 0.45, true, c_lecturas, "cita: mapas != territorio", 1),
        // — tareas (clúster denso aún sin nombre, abajo-centro) —
        n(-60.0, 172.0, 1.2, true, c_tareas, "migrar backup al NAS", 2),
        n(15.0, 245.0, 0.95, true, c_tareas, "factura de la imprenta", 2),
        n(-130.0, 250.0, 0.6, true, c_tareas, "turno del dentista", 2),
        n(-35.0, 300.0, 0.4, true, c_tareas, "renovar el dominio", 2),
        // — sueltas en el anillo exterior —
        n(120.0, -280.0, 0.85, true, c_suelto, "idea: glosario quechua", 3),
        n(-160.0, -10.0, 0.55, true, c_suelto, "sinestesia y tipografia?", 3),
        n(430.0, 170.0, 0.35, true, c_suelto, "numero de la ferreteria", 3),
        n(45.0, 45.0, 0.7, true, c_suelto, "llamar a Ema el sabado", 3),
        n(-250.0, 85.0, 0.5, true, c_huerta, "croquis de las acequias", 0),
        // — bajo el horizonte: la atención las dejó ir —
        n(-480.0, 120.0, 0.05, false, c_suelto, "borrador viejo", 3),
        n(180.0, 310.0, 0.08, false, c_lecturas, "link que nunca abri", 1),
    ]
}

/// Topónimos bautizados (rótulos de continente detrás de los nodos).
fn regiones() -> Vec<(&'static str, f32, f32)> {
    vec![("huerta", -340.0, -138.0), ("lecturas", 315.0, -75.0)]
}

/// Filamentos del nodo seleccionado ("trasplantar los tomates"): sus
/// parientes más afines — activación por difusión, el motor de serendipia.
fn filamentos() -> Vec<((f32, f32), (f32, f32), f32)> {
    let sel = (-340.0_f32, -180.0_f32);
    vec![
        (sel, (-265.0, -230.0), 0.82),
        (sel, (-290.0, -105.0), 0.66),
        (sel, (-410.0, -120.0), 0.58),
        (sel, (-380.0, -255.0), 0.41),
        (sel, (-160.0, -10.0), 0.24),
    ]
}

// ───────────────────────── el pintor del mapa (animado) ─────────────────────────

#[allow(clippy::too_many_arguments)]
fn paint_map(
    scene: &mut vello::Scene,
    ts: &mut Typesetter,
    rect: PaintRect,
    nodes: &[Nodo],
    links: &[((f32, f32), (f32, f32), f32)],
    regions: &[(&'static str, f32, f32)],
    theme: &Theme,
    t: f32,
    pan: (f32, f32),
    zoom: f32,
    // beats:
    region_a: f32,   // alpha de topónimos
    link_prog: f32,  // 0..1 draw-on de filamentos
    sel_alpha: f32,  // alpha del anillo de selección + filamentos
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    // Mundo → pantalla local (calco de world_to_local con cámara móvil).
    let to_screen = |wx: f32, wy: f32| -> (f64, f64) {
        let lx = rect.w * 0.5 + (wx + pan.0) * zoom;
        let ly = rect.h * 0.5 + (wy + pan.1) * zoom;
        ((rect.x + lx) as f64, (rect.y + ly) as f64)
    };

    // Topónimos al fondo: nombre grande y tenue + halo de territorio.
    for (name, rx, ry) in regions {
        if region_a <= 0.001 {
            break;
        }
        let (cx, cy) = to_screen(*rx, *ry);
        let blob = KurboCircle::new((cx, cy), (96.0 * zoom as f64).max(34.0));
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(theme.accent, 0.05 * region_a),
            None,
            &blob,
        );
        let size = (15.0 * zoom).clamp(11.0, 30.0);
        let est_w = name.chars().count() as f64 * size as f64 * 0.52;
        draw_block(
            scene,
            ts,
            &TextBlock::simple(
                name,
                size,
                with_alpha(theme.fg_text, 0.30 * region_a),
                (cx - est_w * 0.5, cy - size as f64 * 0.6),
            ),
        );
    }

    // Filamentos (debajo de los nodos), dibujándose con `link_prog`.
    for (a, b, aff) in links {
        if sel_alpha <= 0.001 || link_prog <= 0.001 {
            break;
        }
        let (ax, ay) = to_screen(a.0, a.1);
        let (bx, by) = to_screen(b.0, b.1);
        // draw-on: interpolá el extremo hacia el destino.
        let ex = lerp(ax as f32, bx as f32, link_prog) as f64;
        let ey = lerp(ay as f32, by as f32, link_prog) as f64;
        let mut path = BezPath::new();
        path.move_to((ax, ay));
        path.line_to((ex, ey));
        let alpha = ((0.18 + aff * 0.55).clamp(0.0, 0.85)) * sel_alpha;
        scene.stroke(
            &Stroke::new((0.8 + *aff as f64 * 1.6).max(0.6)),
            Affine::IDENTITY,
            with_alpha(theme.accent, alpha),
            None,
            &path,
        );
    }

    // Nodos: tamaño/brillo crecen con la masa viva — y la masa LATE con `t`.
    for n in nodes {
        // Stagger de entrada: cada nodo emerge en su ventana.
        let appear = motion::ease_out_back(seg(
            t,
            0.14 + 0.0085 * n.seed as f32,
            0.14 + 0.0085 * n.seed as f32 + 0.20,
        ))
        .clamp(0.0, 1.0);
        if appear <= 0.001 {
            continue;
        }
        // El latido: cada nota respira con una fase propia (decae/renace).
        let phase = hash01(n.seed) * std::f32::consts::TAU;
        let breath = 1.0 + 0.18 * (t * std::f32::consts::TAU * 1.3 + phase).sin();
        let m = (n.mass * breath).clamp(0.0, 2.0);

        let (px, py) = to_screen(n.x, n.y);
        let r = (3.0 + m * 4.5) * (0.6 + 0.4 * zoom.clamp(0.5, 1.5)) * appear;
        let glow = if n.visible {
            (0.35 + m * 0.45).clamp(0.0, 1.0)
        } else {
            0.18
        } * appear;
        let color = with_alpha(n.color, glow);

        // Halo de las notas más encendidas (respira con la masa).
        if n.visible && m > 0.6 {
            let halo = KurboCircle::new((px, py), (r + 5.0) as f64);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(n.color, 0.10 * appear),
                None,
                &halo,
            );
        }
        let circle = KurboCircle::new((px, py), r as f64);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &circle);

        // Anillo de selección sobre el nodo raíz de los filamentos.
        let is_sel = (n.x - (-340.0)).abs() < 1.0 && (n.y - (-180.0)).abs() < 1.0;
        if is_sel && sel_alpha > 0.001 {
            let ring = KurboCircle::new((px, py), (r + 3.0) as f64);
            scene.stroke(
                &Stroke::new(2.0),
                Affine::IDENTITY,
                with_alpha(theme.accent, sel_alpha),
                None,
                &ring,
            );
        }

        // Etiqueta: con zoom o si es la seleccionada — para no saturar.
        if (zoom >= 0.9 || is_sel) && n.visible && appear > 0.6 {
            let la = ((glow + 0.25).clamp(0.0, 1.0)) * seg(appear, 0.6, 1.0);
            let lbl_col = with_alpha(theme.fg_text, la);
            draw_block(
                scene,
                ts,
                &TextBlock::simple(n.label, 10.0 * zoom.clamp(0.85, 1.4), lbl_col, (px + r as f64 + 4.0, py - 7.0)),
            );
        }
    }
}

// ───────────────────────── overlays vector (cold-open + wordmark) ─────────────────────────

fn draw_overlays(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64, theme: &Theme) {
    // ── COLD OPEN: el "cordón" de khipu se anuda (0–13%) ─────────────
    // Un khipu es una cuerda con nudos: dibujamos una hebra que cae y
    // tres nudos que aparecen — guiño a la metáfora, antes del mapa.
    let line_vis = 1.0 - seg(t, 0.13, 0.20);
    if line_vis > 0.001 {
        let cx = cw / 2.0;
        let cy = ch / 2.0;
        // draw-on por muestreo del cúbico.
        use llimphi_ui::llimphi_raster::kurbo::{CubicBez, ParamCurve};
        let cb = CubicBez::new(
            Point::new(cx - 300.0, cy - 70.0),
            Point::new(cx - 120.0, cy + 120.0),
            Point::new(cx + 120.0, cy - 120.0),
            Point::new(cx + 300.0, cy + 70.0),
        );
        let prog = motion::ease_out_cubic(seg(t, 0.02, 0.14)) as f64;
        let mut trimmed = BezPath::new();
        trimmed.move_to(cb.p0);
        let steps = 90;
        let mut head = cb.p0;
        for i in 1..=steps {
            let u = (i as f64 / steps as f64) * prog;
            let pt = cb.eval(u);
            trimmed.line_to(pt);
            head = pt;
        }
        scene.stroke(
            &Stroke::new(2.2),
            Affine::IDENTITY,
            with_alpha(theme.accent, 0.9 * line_vis),
            None,
            &trimmed,
        );
        // Nudos del khipu: tres bultos que aparecen escalonados sobre la hebra.
        for (k, u) in [0.25_f64, 0.55, 0.85].into_iter().enumerate() {
            let knot_in = motion::ease_out_back(seg(t, 0.05 + k as f32 * 0.022, 0.05 + k as f32 * 0.022 + 0.08));
            if knot_in <= 0.001 || u > prog + 0.02 {
                continue;
            }
            let pt = cb.eval(u);
            let kr = 6.0 * knot_in as f64;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(theme.accent, 0.18 * line_vis * knot_in),
                None,
                &KurboCircle::new(pt, kr * 2.4),
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(theme.accent, line_vis * knot_in),
                None,
                &KurboCircle::new(pt, kr),
            );
        }
        // Punto cabeza teal.
        let head_a = motion::ease_out_cubic(seg(t, 0.03, 0.10)) * line_vis;
        if head_a > 0.001 {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(theme.accent, head_a),
                None,
                &KurboCircle::new(head, 4.5),
            );
        }
    }

    // ── WORDMARK «khipu» (84–100%) ──────────────────────────────────
    let word_a = motion::ease_out_cubic(seg(t, 0.85, 0.95));
    if word_a > 0.001 {
        let size = 132.0_f32;
        let layout = ts.layout(
            "khipu", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false,
        );
        let m = measurement(&layout);
        let rise = lerp(24.0, 0.0, word_a);
        let ox = (cw - m.width as f64) / 2.0;
        let oy = (ch - m.height as f64) / 2.0 - 18.0 + rise as f64;
        let brush = peniko::Brush::Solid(with_alpha(theme.fg_text, word_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));

        let sub_a = motion::ease_out_cubic(seg(t, 0.89, 0.99));
        if sub_a > 0.001 {
            let ssz = 26.0_f32;
            let sub = ts.layout(
                "a canvas for thought, in Rust", ssz, None, Alignment::Start, 1.0, false, None,
                400.0, false, false,
            );
            let sm = measurement(&sub);
            let dot_r = 6.0;
            let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
            let sx = (cw - block_w) / 2.0;
            let sy = oy + m.height as f64 + 18.0;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(theme.accent, sub_a),
                None,
                &KurboCircle::new(Point::new(sx + dot_r, sy + ssz as f64 * 0.42), dot_r),
            );
            let sbrush = peniko::Brush::Solid(with_alpha(theme.fg_muted, sub_a));
            draw_layout_brush_xf(scene, &sub, &sbrush, Affine::translate((sx + dot_r * 2.0 + 14.0, sy)));
        }
    }

    // Punto teal de firma (esquina inf-der), ancla de marca.
    let corner_a = seg(t, 0.05, 0.13) * (1.0 - seg(t, 0.80, 0.86));
    if corner_a > 0.001 {
        let cx = cw - 54.0;
        let cy = ch - 54.0;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(theme.accent, 0.16 * corner_a),
            None,
            &KurboCircle::new(Point::new(cx, cy), 18.0),
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(theme.accent, 0.9 * corner_a),
            None,
            &KurboCircle::new(Point::new(cx, cy), 6.0),
        );
    }
}

// ───────────────────────── chip de bautizo (widget real, anclado) ─────────────────────────

/// Calco de `name_region_chip` + `pinned` (src/map.rs): ofrece bautizar el
/// clúster denso con el topónimo propuesto. Aparece tras el zoom semántico.
fn chip_nombrar(sx: f32, sy: f32, name: &str, alpha: f32, theme: &Theme) -> View<()> {
    let (w, h) = (170.0_f32, 26.0_f32);
    let chip = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(taffy::AlignItems::Center),
        justify_content: Some(taffy::JustifyContent::Center),
        ..Default::default()
    })
    .fill(with_alpha(theme.bg_button, alpha))
    .radius(13.0)
    .border(1.0, with_alpha(theme.accent, 0.45 * alpha))
    .text_aligned(format!("✛ {name}"), 11.5, with_alpha(theme.fg_text, alpha), Alignment::Center);
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(sx - w * 0.5),
            top: length(sy - h * 0.5),
            right: taffy::prelude::auto(),
            bottom: taffy::prelude::auto(),
        },
        size: Size { width: length(w), height: length(h) },
        ..Default::default()
    })
    .alpha(alpha)
    .children(vec![chip])
}

// ───────────────────────── la escena por frame ─────────────────────────

fn build_view(t: f32, cw: f32, ch: f32, theme: &Theme, nodes: &[Nodo]) -> View<()> {
    let regions = regiones();
    let links = filamentos();

    // ── Cámara: vista general (0) → zoom semántico al clúster "tareas"
    //    (centro-abajo) (1) → vuelve a abrir. El zoom revela detalle.
    let zoom_in = motion::ease_in_out_cubic(seg(t, 0.40, 0.58));
    let zoom_out = motion::ease_in_out_cubic(seg(t, 0.66, 0.80));
    let zoom_amt = zoom_in - zoom_out; // 0 → 1 → 0
    let zoom = lerp(0.86, 1.6, zoom_amt);
    // Pan base de encuadre: el bounding box de la masa es asimétrico (más
    // peso arriba-izquierda), así que descentramos un poco para que la
    // constelación "huerta" no se coma el borde en la vista general.
    let base_pan = (40.0_f32, 30.0_f32);
    // pan hacia el clúster "tareas" (centroide ~ (-52, 247) en mundo).
    let target = (52.0_f32, -247.0_f32); // pan = -centroide para centrarlo
    let pan = (
        lerp(base_pan.0, target.0, zoom_amt),
        lerp(base_pan.1, target.1, zoom_amt),
    );

    // beats de los overlays del mapa
    let region_a = motion::ease_out_cubic(seg(t, 0.30, 0.44));
    let link_prog = motion::ease_out_cubic(seg(t, 0.22, 0.40));
    let sel_alpha = motion::ease_out_cubic(seg(t, 0.20, 0.30)) * (1.0 - seg(t, 0.80, 0.86));

    // Fade-out del mapa antes del wordmark.
    let map_fade = 1.0 - seg(t, 0.80, 0.86);

    let nodes = nodes.to_vec();
    let theme_c = theme.clone();
    let canvas = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        position: Position::Relative,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .alpha(map_fade.clamp(0.0, 1.0))
    .paint_with(move |scene, ts, rect| {
        paint_map(
            scene, ts, rect, &nodes, &links, &regions, &theme_c, t, pan, zoom, region_a, link_prog,
            sel_alpha,
        );
    });

    // Chip de bautizo del clúster "tareas" (Cocina): aparece con el zoom.
    let chip_a = motion::ease_out_cubic(seg(t, 0.50, 0.62)) * (1.0 - seg(t, 0.74, 0.80)) * map_fade;
    let mut canvas_children: Vec<View<()>> = Vec::new();
    if chip_a > 0.001 {
        // centroide "tareas" en mundo (-52, 247) → pantalla con cámara actual.
        let wx = -52.0_f32;
        let wy = 247.0_f32;
        let lx = cw * 0.5 + (wx + pan.0) * zoom;
        let ly = ch * 0.5 + (wy + pan.1) * zoom;
        canvas_children.push(chip_nombrar(lx, ly - 60.0, "Cocina", chip_a.clamp(0.0, 1.0), theme));
    }
    let canvas = canvas.children(canvas_children);

    // Panel con padding 4 (misma composición que gravity_panel).
    let panel = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        position: Position::Relative,
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![canvas]);

    // Nodo overlay full-screen para el vector (cold-open + wordmark).
    let theme_o = theme.clone();
    let cwf = cw as f64;
    let chf = ch as f64;
    let overlay = View::<()>::new(Style {
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
    .paint_with(move |scene, ts, _rect: PaintRect| {
        draw_overlays(scene, ts, t, cwf, chf, &theme_o);
    });

    View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        position: Position::Relative,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![panel, overlay])
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| "showreel_frames_khipu".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    create_dir_all(&out_dir).expect("mkdir out_dir");

    let theme = Theme::dark(); // el theme canónico de khipu (src/main.rs)
    let nodes = elenco(theme.accent);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("showreel-khipu"),
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

    let [br, bg, bb, _] = theme.bg_app.components;
    let base = Color::from_rgba8((br * 255.0) as u8, (bg * 255.0) as u8, (bb * 255.0) as u8, 255);

    let mut ts = Typesetter::new();
    let cw = w as f32;
    let ch = h as f32;

    for i in 0..n {
        let t = if n <= 1 { 0.0 } else { i as f32 / (n as f32 - 1.0) };
        let root = build_view(t, cw, ch, &theme, &nodes);

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
            eprintln!("showreel-khipu: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("showreel-khipu: {n} frames en {out_dir}/ ({w}x{h})");
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
        let s = r * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
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
