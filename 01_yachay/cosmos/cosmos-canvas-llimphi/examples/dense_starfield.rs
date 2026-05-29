//! Caller real de Fase 5 del SDD `02_ruway/llimphi/SDD.md`
//! §"GPU directo wgpu" — un starfield denso (N estrellas sintéticas
//! distribuidas en una esfera celeste con concentración en el plano
//! galáctico) renderizado en una sola draw call con `gpu_paint_with`.
//!
//! No es producción: las estrellas son sintéticas (no HYG, no Gaia DR3).
//! Lo que valida es la cadena completa:
//!
//!   cosmos-canvas-llimphi
//!     → pineal-render::GpuSceneCanvas (Canvas trait)
//!       → llimphi-raster::GpuBatch (rects/lines/tris)
//!         → llimphi-ui::View::gpu_paint_with (encoder + view)
//!           → wgpu (draw call instanciada)
//!
//! El painter es agnóstico — habla contra `pineal_render::Canvas` con
//! `fill_rect` por estrella, y elegir GPU vs vello es decisión de la
//! app al enchufar `gpu_paint_with` vs `paint_with`. Cambio el N con
//! teclas: + sube, - baja.
//!
//! Corre con: `cargo run -p cosmos-canvas-llimphi --example dense_starfield --release`.

use std::sync::{Arc, OnceLock};

use llimphi_ui::llimphi_hal::wgpu;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::peniko::Color as PenikoColor;
use llimphi_ui::llimphi_raster::{GpuBatch, GpuPipelines};
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, PaintRect, View};
use pineal_render::{Canvas, Color, GpuSceneCanvas, Rect};

/// Conteo inicial. Las teclas + / - lo doblan/parten dentro de
/// [10K, 4M] — útil para ver dónde empieza a caerse el frame rate
/// en GPU real.
const START_N: u32 = 250_000;

#[derive(Clone)]
enum Msg {
    Multiply(f32),
}

struct DenseStarfield;

impl App for DenseStarfield {
    type Model = u32;
    type Msg = Msg;

    fn title() -> &'static str {
        "cosmos · dense_starfield (GPU directo)"
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        START_N
    }

    fn update(model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Multiply(f) => {
                let next = (model as f32 * f).round() as u32;
                next.clamp(10_000, 4_000_000)
            }
        }
    }

    fn on_key(_model: &Self::Model, ev: &KeyEvent) -> Option<Self::Msg> {
        if !matches!(ev.state, KeyState::Pressed) {
            return None;
        }
        match &ev.key {
            Key::Character(c) if c.as_str() == "+" || c.as_str() == "=" => {
                Some(Msg::Multiply(2.0))
            }
            Key::Character(c) if c.as_str() == "-" => Some(Msg::Multiply(0.5)),
            Key::Named(NamedKey::Space) => Some(Msg::Multiply(1.0)), // re-roll seed
            _ => None,
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let n = *model;
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(PenikoColor::from_rgba8(6, 8, 16, 255))
        // Texto informativo lo dibuja vello (paint_with) PRIMERO; el
        // starfield denso queda encima vía gpu_paint_with. No hay
        // texto en el GPU directo por diseño.
        .paint_with(move |scene, ts, rect: PaintRect| {
            use llimphi_ui::llimphi_text::{
                draw_layout, layout_block, Alignment, TextBlock,
            };
            let block = TextBlock {
                text: &format!(
                    "{n} estrellas · GpuSceneCanvas + GpuBatch · ±/= para escalar"
                ),
                size_px: 16.0,
                color: PenikoColor::from_rgba8(200, 215, 240, 220),
                origin: (rect.x as f64 + 16.0, rect.y as f64 + 14.0),
                max_width: Some(rect.w - 32.0),
                alignment: Alignment::Start,
                line_height: 1.2,
                italic: false,
                font_family: None,
            };
            let layout = layout_block(ts, &block);
            draw_layout(scene, &layout, block.color, block.origin);
        })
        .gpu_paint_with(move |device, queue, encoder, view, rect, _viewport| {
            let pipelines = pipelines_for(device);
            let mut batch = GpuBatch::new(&pipelines);
            {
                let mut canvas = GpuSceneCanvas::new(&mut batch);
                paint_starfield(&mut canvas, rect, n);
            }
            batch.flush(
                device,
                queue,
                encoder,
                view,
                (rect.w, rect.h),
                wgpu::LoadOp::Load,
            );
        })
    }
}

fn pipelines_for(device: &wgpu::Device) -> Arc<GpuPipelines> {
    // Una sola GpuPipelines viva por proceso. El swap format del
    // intermediate de llimphi-hal es Rgba8Unorm — el `view` que recibimos
    // en gpu_paint_with es esa textura.
    static SLOT: OnceLock<Arc<GpuPipelines>> = OnceLock::new();
    SLOT.get_or_init(|| {
        Arc::new(GpuPipelines::new(device, wgpu::TextureFormat::Rgba8Unorm))
    })
    .clone()
}

fn paint_starfield<C: Canvas>(canvas: &mut C, rect: PaintRect, n: u32) {
    // Distribución sintética estilo "esfera celeste vista de frente":
    // densidad ~uniforme en la franja central + cresta diagonal que
    // simula el plano galáctico. Determinista (LCG con seed fijo) para
    // que el resultado sea reproducible entre frames y entre apps.
    let mut state: u32 = 0xCAFEBABEu32;
    let lcg = |s: &mut u32| -> f32 {
        *s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*s & 0x00FF_FFFF) as f32 / 16_777_215.0
    };

    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    // Cresta galáctica: una franja inclinada con peso gaussiano.
    let galactic_angle: f32 = 0.42; // rad
    let (sa, ca) = galactic_angle.sin_cos();

    let radius = (rect.w.min(rect.h)) * 0.49;

    for _ in 0..n {
        // 30% va a la cresta, 70% al campo difuso.
        let in_galaxy = lcg(&mut state) < 0.30;
        let (px, py, brightness) = if in_galaxy {
            // Coordenadas locales (u along, v across) gauss.
            let u = lcg(&mut state) - 0.5;
            let v_u1 = lcg(&mut state) - 0.5;
            let v_u2 = lcg(&mut state) - 0.5;
            let v = (v_u1 + v_u2) * 0.08; // ~gauss strict thin
            // Rotar (u, v) → (x, y) por galactic_angle.
            let lx = u * 2.0 * radius;
            let ly = v * 2.0 * radius;
            let x = cx + ca * lx - sa * ly;
            let y = cy + sa * lx + ca * ly;
            // Brillo mayor cerca del centro galáctico (u ~ 0).
            let b = 0.4 + (1.0 - u.abs() * 2.0).max(0.0) * 0.6;
            (x, y, b)
        } else {
            // Disco circular relleno.
            let r2 = lcg(&mut state);
            let theta = lcg(&mut state) * std::f32::consts::TAU;
            let r = radius * r2.sqrt();
            let x = cx + theta.cos() * r;
            let y = cy + theta.sin() * r;
            let b = (1.0 - lcg(&mut state).powi(3)).clamp(0.15, 1.0);
            (x, y, b)
        };

        // Pequeñas variaciones de color: blanco-azulado a amarillo.
        let t = lcg(&mut state);
        let r_col = 0.85 + 0.15 * t;
        let g_col = 0.88 + 0.10 * (1.0 - t);
        let b_col = 0.95 + 0.05 * (1.0 - t);
        let alpha = brightness * 0.85;

        // Pintar como cuadrado 1.2px — el SDD §"GPU directo" usa
        // exactamente este tamaño para starfield denso. El GpuBatch
        // emite un rect instanciado por estrella.
        let size = 1.2 + brightness * 0.6;
        let r = Rect {
            x: px - size * 0.5,
            y: py - size * 0.5,
            w: size,
            h: size,
        };
        canvas.fill_rect(r, Color::rgba(r_col, g_col, b_col, alpha));
    }
}

fn main() {
    llimphi_ui::run::<DenseStarfield>();
}
