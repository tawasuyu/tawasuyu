//! **Showreel** de `dominium` — el simulador determinista de campo medio.
//!
//! No es eye-candy abstracto: monta la **visualización real** de dominium —
//! la maqueta isométrica que produce `dominium-render-plan` y pinta
//! `dominium-canvas-llimphi::canvas_view` — alimentada por una simulación
//! **viva** de `dominium-physics` que avanza tick a tick a lo largo del
//! reel. Lo que ves en cada frame es una sociedad de lemmings fluyendo
//! sobre el sustrato numérico (5 capas), con los Conceptos (iglesia /
//! banco / comuna / laboratorio) emitiendo sus campos. El motor sólo suma
//! flotantes; la civilización emerge.
//!
//! Render **headless y determinista**: una simulación se siembra con seed
//! fijo y se avanza `N_TICKS_PRE` ticks; luego, frame `i` de `N` →
//! `t = i/(N-1)` → se elige el snapshot vivo correspondiente → se arma el
//! `RenderPlan` con la cámara/relieve del frame → vello → wgpu → PNG.
//!
//! Beats (rediseñados 2026-06-16 para que el reel TENGA MOVIMIENTO):
//!   - **cold-open** (0–8%): trazo bezier draw-on + punto teal, sobre negro.
//!     Breve — no debe dominar.
//!   - **diorama** (6–100%): la maqueta iso real corre TODO el resto del reel
//!     con la cámara en movimiento CONTINUO: un **zoom-in** claro (la escala
//!     iso casi se duplica) combinado con un **paneo** que recorre el
//!     continente en arco (el nodo del canvas es más grande que el viewport y
//!     se desliza). La simulación avanza varios ticks por frame para que
//!     lemmings/réplicas/migración se muevan a ojo. La cámara es la fuente
//!     principal de dinamismo: cada frame se ve distinto.
//!   - **wordmark** (86–100%): "dominium" + subtítulo, diorama en leve
//!     fade-out detrás. (El viejo beat ψ de clústeres k-means se eliminó:
//!     dejaba ~96% de lemmings en un clúster y el recoloreo era invisible —
//!     un beat muerto. Mejor sacarlo que fingirlo.)
//!
//! ```text
//! cargo run -p dominium-app-llimphi --example dominium_showreel --release -- \
//!     [out_dir] [n_frames] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames_dominium`, `n_frames=300`, `W=1600`, `H=900`.
#![allow(dead_code)]

// La app es un binario sin lib: incluimos sus módulos reales por `#[path]`
// para usar exactamente el mismo `Sim`, colores y pack que la app.
#[path = "../src/consts.rs"]
mod consts;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/packs.rs"]
mod packs;
#[path = "../src/sim.rs"]
mod sim;
#[path = "../src/view.rs"]
mod view;
#[path = "../src/worldgen.rs"]
mod worldgen;

use std::fs::{create_dir_all, File};
use std::io::BufWriter;

use dominium_core::{SimParams, World};
use dominium_iso::{IsoProjector, ZWeights};
use dominium_render_plan::{build_plan, PlanConfig, RenderMode, RenderPlan, SpritePrim};
use dominium_sim::Sim;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, Position, Size, Style,
};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::{self, Color, Gradient};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{draw_layout_brush_xf, measurement, Alignment, Typesetter};
use llimphi_theme::motion;
use llimphi_ui::{measure_text_node, mount, paint, PaintRect, View};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Point, Stroke};

use dominium_canvas_llimphi::canvas_view;

use crate::consts::{KMEANS_REFRESH_TICKS, SNAPSHOT_RING_CAP, TRAIL_CAP};
use crate::packs::default_conceptos;
use crate::worldgen::bioma_palette;

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Ticks que avanzamos ANTES de empezar a capturar — deja que la sociedad
/// arranque (réplicas, primeros asentamientos) antes del primer frame.
const N_TICKS_PRE: u64 = 120;

/// Ticks de simulación que avanzamos POR cada frame capturado. Con 1 tick
/// el cambio entre frames es imperceptible (el campo medio se mueve lento);
/// con varios, lemmings/réplicas/migración se mueven a ojo desnudo.
const TICKS_PER_FRAME: u64 = 1;

/// Grilla del reel — MÁS CHICA que la de la app (240). Cada celda emite un
/// techo + caras laterales: a 240×240 son ~150k polígonos y, a 1600×900, el
/// rasterizador por software del entorno wedgea y produce frames negros. Con
/// una grilla menor la maqueta es idéntica en carácter pero la escena pesa
/// una fracción, y el render sale vivo de punta a punta. (La app real sigue
/// usando 240; esto es sólo presentación del reel.)
const SHOW_GRID: usize = 64;

/// Lemmings sembrados en el reel. `consts::LEMMINGS` (2500) está dimensionado
/// para la grilla 240×240 de la app; aplicado a la grilla chica del reel daría
/// una densidad ~15× y un boom-bust violento. Esta cifra siembra al mundo
/// chico cerca de su población de equilibrio (N* ≈ 1000 para grid=64), así el
/// pre-calentamiento converge limpio a la meseta.
const SHOW_LEMMINGS: usize = 180;

/// Color de acento (teal de marca tawasuyu).
const ACCENT: Color = Color::from_rgba8(0x2B, 0xD9, 0xA6, 0xFF);

// ───────────────────────── utilidades ─────────────────────────

fn with_alpha(c: Color, a: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, a.clamp(0.0, 1.0)])
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Reescala `t` desde el subintervalo `[lo,hi]` a `[0,1]`, clampado.
fn seg(t: f32, lo: f32, hi: f32) -> f32 {
    ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
}

// ───────────────────────── snapshots de simulación ─────────────────────────

/// Un fotograma vivo de la simulación: el `World` + las asignaciones de
/// clúster ψ vigentes (para el modo `PsiCluster`).
struct SimSnapshot {
    world: World,
    clusters: Vec<u8>,
}

/// Calibración idéntica a `Dominium::init` / `pantallazo_dominium`: la
/// población crece de forma controlada (réplica barata, regrowth limitado).
fn demo_params() -> SimParams {
    SimParams {
        diffusion_rate: 0.02,
        entropy_rate: 0.004,
        regrowth_rate: 0.004,
        carrying_capacity: 40.0,
        metabolic_cost: 0.05,
        replicate_threshold: 28.0,
        child_energy_frac: 0.45,
        abundance_threshold: 50.0,
        ..SimParams::default()
    }
}

/// Relieve visual por bioma (mares hunden, picos elevan) — calco de `init`.
fn demo_weights() -> ZWeights {
    ZWeights {
        materia: 0.02,
        psique: -0.075,
        poder: 0.40,
        oro: 0.0,
        degradacion: 1.30,
    }
}

/// Corre la simulación real y captura `n` snapshots vivos (uno por frame),
/// avanzando un tick de `dominium-physics` por cada uno. Determinista: mismo
/// seed → misma película, bit a bit.
fn capture_snapshots(n: usize) -> Vec<SimSnapshot> {
    let rng_seed = 0xD0_31_31_07_u64;
    let seeder =
        |s: u64| dominium_core::worldgen::seed(s, SHOW_GRID, SHOW_LEMMINGS, default_conceptos());
    let mut sim = Sim::new(
        seeder(rng_seed),
        demo_params(),
        rng_seed,
        SNAPSHOT_RING_CAP,
        TRAIL_CAP,
        KMEANS_REFRESH_TICKS,
        true,
        Box::new(seeder),
    );

    // Calentamiento: que la sociedad ya esté en marcha al primer frame.
    for _ in 0..N_TICKS_PRE {
        sim.advance(true);
    }

    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        // Varios ticks por frame: el movimiento de la sociedad (lemmings,
        // réplicas, migración) se nota frame a frame, no sólo la cámara.
        for _ in 0..TICKS_PER_FRAME {
            sim.advance(false);
        }
        out.push(SimSnapshot {
            world: sim.world.clone(),
            clusters: sim.cluster_assignments.clone(),
        });
    }
    out
}

// ───────────────────────── la escena por frame ─────────────────────────

/// Desplaza TODA la geometría del plan por `(dx, dy)` pero deja la caja
/// envolvente (`min/max`) intacta. `canvas_view` centra el plan según su
/// bbox, así que al mover la geometría sin mover el bbox la maqueta se
/// **panea** dentro del rect — la cámara recorre el continente. (Hacerlo
/// así, en vez de agrandar el nodo del canvas, evita un nodo gigante que en
/// el render headless por software dejaba el readback en frame congelado.)
fn pan_plan(mut plan: RenderPlan, dx: f32, dy: f32) -> RenderPlan {
    for q in &mut plan.quads {
        q.x += dx;
        q.y += dy;
    }
    for p in &mut plan.polygons {
        for v in &mut p.vertices {
            v.0 += dx;
            v.1 += dy;
        }
    }
    for g in &mut plan.glyphs {
        g.x += dx;
        g.y += dy;
    }
    for s in &mut plan.sprites {
        match s {
            SpritePrim::Fill { points, .. } | SpritePrim::Stroke { points, .. } => {
                for pt in points {
                    pt.0 += dx;
                    pt.1 += dy;
                }
            }
            SpritePrim::Disc { cx, cy, .. } => {
                *cx += dx;
                *cy += dy;
            }
        }
    }
    // bbox a propósito SIN tocar: el paneo nace de la diferencia entre el
    // centro del bbox (donde canvas_view ancla) y la geometría ya movida.
    plan
}

/// Recorta del plan toda la geometría que cae FUERA del viewport (con un
/// margen). Imprescindible a zoom alto: la maqueta de 240×240 emite ~150k
/// polígonos, pero acercada sólo una fracción es visible — pintar los 150k a
/// 1600×900 satura el rasterizador por software y wedgea el device. Culling
/// deja sólo lo on-screen y mantiene la escena liviana de punta a punta.
///
/// `canvas_view` ancla el centro del bbox en el centro del rect, así que la
/// posición en pantalla de un vértice es `vértice + (centro_rect − centro_bbox)`.
/// El bbox se deja INTACTO (el culling no debe mover la cámara).
fn cull_plan(mut plan: RenderPlan, cw: f64, ch: f64, margin: f32) -> RenderPlan {
    let bbox_cx = (plan.min_x + plan.max_x) * 0.5;
    let bbox_cy = (plan.min_y + plan.max_y) * 0.5;
    let off_x = cw as f32 * 0.5 - bbox_cx;
    let off_y = ch as f32 * 0.5 - bbox_cy;
    let lo_x = -margin;
    let lo_y = -margin;
    let hi_x = cw as f32 + margin;
    let hi_y = ch as f32 + margin;
    let on_screen = |x: f32, y: f32, w: f32, h: f32| -> bool {
        let sx = x + off_x;
        let sy = y + off_y;
        sx + w >= lo_x && sx <= hi_x && sy + h >= lo_y && sy <= hi_y
    };
    plan.quads.retain(|q| on_screen(q.x, q.y, q.w, q.h));
    plan.polygons.retain(|p| {
        let (mut nx, mut ny, mut xx, mut xy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (vx, vy) in p.vertices {
            nx = nx.min(vx);
            ny = ny.min(vy);
            xx = xx.max(vx);
            xy = xy.max(vy);
        }
        on_screen(nx, ny, xx - nx, xy - ny)
    });
    plan
}

/// Arma el `RenderPlan` de un snapshot con la escala iso del frame `t`. El
/// zoom-in del reel se logra ramplando `iso.scale` (la cámara se acerca).
fn plan_for(snap: &SimSnapshot, weights: &ZWeights, scale: f32) -> RenderPlan {
    let iso = IsoProjector::new(scale, 0.55);
    let cfg = PlanConfig {
        tile: scale,
        lemming_size: 2.6,
        lemming_lift: 0.6,
        concepto_size: 7.5,
        concepto_lift: 2.0,
        light_dir: (0.55, 0.35),
        andina_layers: 0,
        andina_threshold: 1.0,
        palette: bioma_palette(),
        render_mode: RenderMode::Composite,
        texture: false,
    };
    build_plan(&snap.world, &iso, weights, &cfg)
}

/// Overlays vector (cold-open + wordmark + punto firma) sobre un nodo
/// full-screen, en función de `t`.
fn draw_overlays(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64) {
    // ── COLD OPEN (0–8%) ───────────────────────────────────────────
    let b1 = seg(t, 0.0, 0.05);
    let line_vis = 1.0 - seg(t, 0.05, 0.08);
    if line_vis > 0.001 {
        let cx = cw / 2.0;
        let cy = ch / 2.0;
        let mut path = BezPath::new();
        path.move_to((cx - 360.0, cy + 40.0));
        let c1 = (cx - 150.0, cy - 220.0);
        let c2 = (cx + 150.0, cy + 220.0);
        let p3 = (cx + 360.0, cy - 40.0);
        let cb = vello::kurbo::CubicBez::new(
            Point::new(cx - 360.0, cy + 40.0),
            Point::new(c1.0, c1.1),
            Point::new(c2.0, c2.1),
            Point::new(p3.0, p3.1),
        );
        use vello::kurbo::ParamCurve;
        let draw_on = motion::ease_out_cubic(seg(t, 0.01, 0.055)) as f64;
        let mut trimmed = BezPath::new();
        let mut head = cb.p0;
        trimmed.move_to(cb.p0);
        let steps = 96;
        for k in 1..=steps {
            let u = (k as f64 / steps as f64) * draw_on;
            let pt = cb.eval(u);
            trimmed.line_to(pt);
            head = pt;
        }
        let line_col = with_alpha(ACCENT, 0.9 * line_vis);
        scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, line_col, None, &trimmed);
        let pop = motion::ease_out_back(b1) as f64;
        let r = (4.0_f64 + 7.0 * pop).max(0.0);
        let dot_a = (b1 * line_vis).clamp(0.0, 1.0);
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(ACCENT, 0.18 * dot_a),
            None,
            &Circle::new(head, r * 3.2),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(ACCENT, dot_a),
            None,
            &Circle::new(head, r),
        );
    }

    // ── WORDMARK (86–100%) ─────────────────────────────────────────
    let word_in = seg(t, 0.86, 0.95);
    let word_a = motion::ease_out_cubic(word_in);
    if word_a > 0.001 {
        let size = 136.0_f32;
        let layout = ts.layout(
            "dominium", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false,
        );
        let m = measurement(&layout);
        let rise = lerp(26.0, 0.0, word_a as f64);
        let ox = (cw - m.width as f64) / 2.0;
        let oy = (ch - m.height as f64) / 2.0 - 22.0 + rise;
        let brush = peniko::Brush::Solid(with_alpha(Color::from_rgba8(0xF2, 0xF4, 0xF3, 0xFF), word_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));

        let sub_a = motion::ease_out_cubic(seg(t, 0.90, 0.99));
        if sub_a > 0.001 {
            let ssz = 25.0_f32;
            let sub = ts.layout(
                "un simulador donde la civilización emerge de la aritmética",
                ssz, None, Alignment::Start, 1.0, false, None, 400.0, false, false,
            );
            let sm = measurement(&sub);
            let dot_r = 6.0;
            let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
            let sx = (cw - block_w) / 2.0;
            let sy = oy + m.height as f64 + 20.0;
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(ACCENT, sub_a),
                None,
                &Circle::new(Point::new(sx + dot_r, sy + ssz as f64 * 0.42), dot_r),
            );
            let sbrush = peniko::Brush::Solid(with_alpha(Color::from_rgba8(0x9A, 0xA3, 0xA0, 0xFF), sub_a));
            draw_layout_brush_xf(scene, &sub, &sbrush, Affine::translate((sx + dot_r * 2.0 + 14.0, sy)));
        }
    }

    // ── punto teal de firma (esquina inf-der) ───────────────────────
    let corner_a = seg(t, 0.04, 0.09) * (1.0 - seg(t, 0.84, 0.90));
    if corner_a > 0.001 {
        let cx = cw - 54.0;
        let cy = ch - 54.0;
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(ACCENT, 0.16 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 18.0),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(ACCENT, 0.9 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 6.0),
        );
    }
}

/// Construye el árbol `View` del frame `t`.
fn build_view(
    t: f32,
    cw: f64,
    ch: f64,
    snaps: &[SimSnapshot],
    weights: &ZWeights,
    bg: Color,
) -> View<()> {
    // Snapshot vivo: el diorama se ve a partir de ~6%; antes es el cold-open
    // sobre negro. Mapeamos el tramo [0.06, 1.0] de t al índice de snapshot
    // para que la simulación corra durante todo el reel.
    let diorama_t = seg(t, 0.06, 1.0);
    let idx = ((diorama_t * (snaps.len() as f32 - 1.0)).round() as usize).min(snaps.len() - 1);
    let snap = &snaps[idx];

    // Entrada del diorama: fade-in rápido (6–14%) y leve fade-out bajo el
    // wordmark (no a negro — la maqueta sigue viva detrás del título).
    let in_a = motion::ease_out_cubic(seg(t, 0.06, 0.14));
    let out_a = 1.0 - 0.55 * motion::ease_in_out_cubic(seg(t, 0.86, 0.97)) as f32;
    let diorama_a = (in_a * out_a).clamp(0.0, 1.0) as f64;

    // ── CÁMARA: zoom-in continuo + paneo a lo largo de TODO el reel.
    // CLAVE: la velocidad de cámara debe ser ~constante (lineal), NO un
    // ease-in-out — un ease-in-out concentra todo el movimiento en los bordes
    // y deja un PLATEAU muerto en el medio (frames idénticos). Acá `cam`
    // avanza lineal con `t`, así CADA frame difiere del anterior por igual.
    let cam = seg(t, 0.06, 1.0) as f64;
    // Zoom: acercamiento parejo y perceptible (lineal). La grilla del reel es
    // chica (64), así que arrancamos con la escena llenando el cuadro y la
    // escala se DUPLICA hacia el primer plano — el zoom es la fuente principal
    // de movimiento del reel, no la sim. Con culling la escena se mantiene
    // liviana en todo el rango.
    let scale = lerp(9.0, 18.0, cam) as f32;

    // Paneo: desplazamos la geometría del plan (sin tocar su bbox), así
    // `canvas_view` —que ancla en el centro del bbox— deja la maqueta corrida
    // dentro del rect. Recorrido en arco diagonal (avance lineal en X +
    // curva en Y) para que la cámara cruce el continente revelando regiones
    // distintas a medida que el zoom aprieta. El nodo sigue siendo del tamaño
    // del viewport (estable en el render headless).
    const PAN_AMP_X: f64 = 360.0;
    const PAN_AMP_Y: f64 = 220.0;
    // X: barrido lineal izquierda→derecha (velocidad constante).
    let pan_x = lerp(PAN_AMP_X, -PAN_AMP_X, cam);
    // Y: avance lineal arriba→abajo MÁS un arco (sin) — sobrevuelo, no recta;
    // siempre en movimiento.
    let arc = (std::f64::consts::PI * cam).sin();
    let pan_y = lerp(PAN_AMP_Y, -PAN_AMP_Y, cam) + arc * PAN_AMP_Y * 0.5;

    let mut children: Vec<View<()>> = Vec::new();

    if diorama_a > 0.001 {
        let plan = pan_plan(plan_for(snap, weights, scale), pan_x as f32, pan_y as f32);
        // Cull a viewport: a zoom alto recorta el grueso de los ~150k
        // polígonos fuera de cuadro y mantiene la escena liviana (clave para
        // que el GPU por software no wedgee a pantalla completa).
        let plan = cull_plan(plan, cw, ch, 64.0);
        let canvas = View::new(Style {
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
        .alpha(diorama_a as f32)
        .children(vec![canvas_view::<()>(plan, None, (0.0, 0.0))]);
        children.push(canvas);
    }

    // Viñeta sutil para asentar el diorama sobre el fondo negro.
    let vignette = {
        let [r, g, b, _] = bg.components;
        let edge = Color::new([r, g, b, 0.0]);
        let dark = Color::new([r * 0.3, g * 0.3, b * 0.3, 0.55]);
        View::new(Style {
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
        .paint_with(move |scene, _ts, rect: PaintRect| {
            let cx = (rect.x + rect.w * 0.5) as f64;
            let cy = (rect.y + rect.h * 0.5) as f64;
            let radius = (rect.w.max(rect.h)) as f64 * 0.75;
            let grad = Gradient::new_radial(Point::new(cx, cy), radius as f32)
                .with_stops([edge, edge, dark].as_slice());
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                &grad,
                None,
                &vello::kurbo::Rect::new(
                    rect.x as f64,
                    rect.y as f64,
                    (rect.x + rect.w) as f64,
                    (rect.y + rect.h) as f64,
                ),
            );
        })
    };
    children.push(vignette);

    // Overlay vector full-screen (cold-open + wordmark).
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
    .paint_with(move |scene, ts, _rect: PaintRect| {
        draw_overlays(scene, ts, t, cw, ch);
    });
    children.push(overlay);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        position: Position::Relative,
        overflow: taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Hidden,
        },
        ..Default::default()
    })
    .clip(true)
    .fill(bg)
    .children(children)
}

fn main() {
    rimay_localize::init();
    let mut args = std::env::args().skip(1);
    let out_dir = args
        .next()
        .unwrap_or_else(|| "showreel_frames_dominium".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    // Ventana de frames a renderar [start, end) — para chunkear el render en
    // varios procesos (el GPU por software del entorno wedgea tras ~18 frames
    // pesados en un mismo device; un proceso por chunk lo sortea). El `t` se
    // computa siempre contra `n`, así la ventana es un subconjunto del reel
    // completo, no un reel recortado. Default: todo.
    let start: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let end: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(n).min(n);
    create_dir_all(&out_dir).expect("mkdir out_dir");

    // Fondo: un negro azulado profundo, espacio negativo elegante.
    let bg = Color::from_rgba8(0x07, 0x09, 0x0B, 0xFF);
    let weights = demo_weights();

    eprintln!("dominium_showreel: sembrando simulación y capturando {n} snapshots vivos…");
    let snaps = capture_snapshots(n);
    eprintln!(
        "dominium_showreel: pob inicial {} → final {} lemmings",
        snaps.first().map(|s| s.world.lemmings.len()).unwrap_or(0),
        snaps.last().map(|s| s.world.lemmings.len()).unwrap_or(0),
    );

    // GPU: un device por proceso. El cuello de botella real del render
    // headless por software (llvmpipe) NO es el zoom sino el VOLUMEN de
    // geometría: la grilla de la app (240) emite ~150k polígonos y a 1600×900
    // satura el rasterizador hasta dejar frames negros/congelados. Por eso el
    // reel usa `SHOW_GRID` (120) + culling a viewport: con la escena liviana,
    // los 300 frames salen vivos en un solo proceso. Los args `start`/`end`
    // quedan disponibles por si hiciera falta chunkear en otro entorno.
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = make_target(&hal, w, h);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut ts = Typesetter::new();
    let cw = w as f64;
    let ch = h as f64;
    let [br, bgc, bb, _] = bg.components;
    let base = Color::from_rgba8((br * 255.0) as u8, (bgc * 255.0) as u8, (bb * 255.0) as u8, 255);

    for i in start..end {
        let t = if n <= 1 { 0.0 } else { i as f32 / (n as f32 - 1.0) };
        let root = build_view(t, cw, ch, &snaps, &weights, bg);

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
        // Bloqueo explícito: que el trabajo de vello (compute + blit) termine
        // ANTES de copiar la textura. Sin esto, en el GPU por software del
        // entorno headless, escenas pesadas (zoom alto = polígonos grandes a
        // pantalla completa) dejaban el readback en frame congelado a partir
        // de cierto punto. Drenar la cola entre render y copia lo evita.
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
        let path = format!("{out_dir}/frame_{i:04}.png");
        write_png(&hal, &target, &path, w, h);
        if i % 30 == 0 || i == n - 1 {
            eprintln!("dominium_showreel: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("dominium_showreel: {n} frames en {out_dir}/ ({w}x{h})");
}

/// Crea la textura destino del render (reusada dentro de un bloque de device).
fn make_target(hal: &Hal, w: u32, h: u32) -> wgpu::Texture {
    hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dominium-showreel"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
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
