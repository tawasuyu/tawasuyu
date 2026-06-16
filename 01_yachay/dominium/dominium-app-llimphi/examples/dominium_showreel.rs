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
//! Beats:
//!   - **cold-open** (0–14%): trazo bezier draw-on + punto teal, sobre negro.
//!   - **diorama** (12–72%): la maqueta iso real entra (fade + zoom suave)
//!     mientras la simulación corre — réplicas, extracciones, migración.
//!   - **clusters ψ** (54–74%): los lemmings se recolorean por su clúster
//!     k-means del `vector_psi` (modo `PsiCluster` real de la app).
//!   - **wordmark** (78–100%): "dominium" + subtítulo, diorama en fade-out.
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
use dominium_render_plan::{
    build_plan_with_overrides, Color as PlanColor, PlanConfig, RenderMode, RenderPlan,
};
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

use crate::consts::{GRID, KMEANS_REFRESH_TICKS, LEMMINGS, SNAPSHOT_RING_CAP, TRAIL_CAP};
use crate::packs::default_conceptos;
use crate::worldgen::bioma_palette;

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Ticks que avanzamos ANTES de empezar a capturar — deja que la sociedad
/// arranque (réplicas, primeros asentamientos) antes del primer frame.
const N_TICKS_PRE: u64 = 8;

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
    let seeder = |s: u64| dominium_core::worldgen::seed(s, GRID, LEMMINGS, default_conceptos());
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
        sim.advance(false);
        // Recalculamos las tribus ψ (k-means) en CADA frame: el reel es
        // corto y `KMEANS_REFRESH_TICKS` (30) no alcanzaría a dispararse,
        // así el beat de clústeres siempre tiene asignaciones vivas.
        sim.refresh_clusters();
        out.push(SimSnapshot {
            world: sim.world.clone(),
            clusters: sim.cluster_assignments.clone(),
        });
    }
    out
}

/// Colores fijos de clúster ψ — espejo de `sim::CLUSTER_COLORS` de la app.
const CLUSTER_COLORS: [PlanColor; 3] = [
    [0.96, 0.30, 0.72, 1.0], // magenta
    [0.30, 0.90, 0.90, 1.0], // cian
    [0.96, 0.92, 0.30, 1.0], // amarillo
];

// ───────────────────────── la escena por frame ─────────────────────────

/// Arma el `RenderPlan` de un snapshot con la cámara/relieve del frame `t`.
/// El zoom suave del cold-open se logra ramplando `iso.scale`.
fn plan_for(
    snap: &SimSnapshot,
    weights: &ZWeights,
    scale: f32,
    cluster_mix: f32,
) -> RenderPlan {
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
        // El suelo queda igual en ambos modos; sólo cambia el tinte de los
        // lemmings. Composite siempre — el clúster lo aplicamos en la closure.
        render_mode: RenderMode::Composite,
        texture: false,
    };
    let base_lemming = cfg.palette.lemming;
    let clusters = &snap.clusters;
    build_plan_with_overrides(&snap.world, &iso, weights, &cfg, |i| {
        if cluster_mix <= 0.001 || i >= clusters.len() {
            return base_lemming;
        }
        let c = clusters[i] as usize;
        let target = if c < CLUSTER_COLORS.len() {
            CLUSTER_COLORS[c]
        } else {
            base_lemming
        };
        // Cross-fade del color base al color de clúster.
        [
            base_lemming[0] + (target[0] - base_lemming[0]) * cluster_mix,
            base_lemming[1] + (target[1] - base_lemming[1]) * cluster_mix,
            base_lemming[2] + (target[2] - base_lemming[2]) * cluster_mix,
            1.0,
        ]
    })
}

/// Overlays vector (cold-open + wordmark + punto firma) sobre un nodo
/// full-screen, en función de `t`.
fn draw_overlays(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64) {
    // ── COLD OPEN (0–14%) ──────────────────────────────────────────
    let b1 = seg(t, 0.0, 0.13);
    let line_vis = 1.0 - seg(t, 0.13, 0.22);
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
        let draw_on = motion::ease_out_cubic(seg(t, 0.02, 0.14)) as f64;
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

    // ── WORDMARK (80–100%) ─────────────────────────────────────────
    let word_in = seg(t, 0.80, 0.92);
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

        let sub_a = motion::ease_out_cubic(seg(t, 0.85, 0.98));
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
    let corner_a = seg(t, 0.05, 0.13) * (1.0 - seg(t, 0.76, 0.82));
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
    // Snapshot vivo: el diorama sólo se ve a partir de ~10%; antes es el
    // cold-open sobre negro. Mapeamos el tramo [0.10, 1.0] de t al índice
    // de snapshot para que la simulación corra durante todo el reel.
    let diorama_t = seg(t, 0.10, 1.0);
    let idx = ((diorama_t * (snaps.len() as f32 - 1.0)).round() as usize).min(snaps.len() - 1);
    let snap = &snaps[idx];

    // Entrada del diorama: fade-in (10–24%) y fade-out antes del wordmark.
    let in_a = motion::ease_out_cubic(seg(t, 0.10, 0.26));
    let out_a = 1.0 - motion::ease_in_out_cubic(seg(t, 0.76, 0.84));
    let diorama_a = (in_a * out_a).clamp(0.0, 1.0) as f64;

    // Zoom suave: la cámara entra de un poco más lejos a la escala nominal.
    let scale = lerp(2.8, 3.45, motion::ease_out_cubic(seg(t, 0.10, 0.58)) as f64) as f32;

    // Beat ψ: cross-fade a colores de clúster (54–66%) y de vuelta (70–76%).
    let cluster_mix =
        motion::ease_in_out_cubic(seg(t, 0.54, 0.66)) * (1.0 - motion::ease_in_out_cubic(seg(t, 0.70, 0.76)));

    let mut children: Vec<View<()>> = Vec::new();

    if diorama_a > 0.001 {
        let plan = plan_for(snap, weights, scale, cluster_mix);
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
        .children(vec![canvas_view::<()>(plan, None)]);
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
        ..Default::default()
    })
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

    // GPU una sola vez; reusar device/renderer/target para los N frames.
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
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
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut ts = Typesetter::new();
    let cw = w as f64;
    let ch = h as f64;
    let [br, bgc, bb, _] = bg.components;
    let base = Color::from_rgba8((br * 255.0) as u8, (bgc * 255.0) as u8, (bb * 255.0) as u8, 255);

    for i in 0..n {
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
        let path = format!("{out_dir}/frame_{i:04}.png");
        write_png(&hal, &target, &path, w, h);
        if i % 30 == 0 || i == n - 1 {
            eprintln!("dominium_showreel: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("dominium_showreel: {n} frames en {out_dir}/ ({w}x{h})");
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
