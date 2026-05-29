//! multimedia-app — primer reproductor del dominio.
//!
//! Pipeline: [`multimedia_core::TestCard`] genera RGBA cada ~33 ms,
//! lo empuja a un [`llimphi_surface::ExternalSurface`], y la UI
//! Llimphi lo expone en un canvas central vía `View::gpu_paint_with`.
//! Es el MVP feo: gradiente animado + círculo rebotando. Sin
//! ffmpeg/gstreamer ni nada. Sirve para validar que la cadena
//! productor → surface → frame anda extremo a extremo.
//!
//! Corre con: `cargo run -p multimedia-app --release`.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use llimphi_surface::ExternalSurface;
use llimphi_ui::llimphi_hal::wgpu;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};
use multimedia_core::{FrameSource, TestCard};
use parking_lot::Mutex;

const SOURCE_W: u32 = 480;
const SOURCE_H: u32 = 270;
const SOURCE_FPS: f32 = 30.0;
const TICK_MS: u64 = 33;

#[derive(Clone)]
enum Msg {
    Tick,
}

struct Model {
    frames: u64,
    started_at: Instant,
}

struct Pipeline {
    surface: ExternalSurface,
    source: Mutex<TestCard>,
    buf: Mutex<Vec<u8>>,
    last_tick: Mutex<Instant>,
}

fn pipeline_slot() -> &'static OnceLock<Pipeline> {
    static SLOT: OnceLock<Pipeline> = OnceLock::new();
    &SLOT
}

fn pipeline_for(device: &wgpu::Device, queue: &wgpu::Queue) -> &'static Pipeline {
    pipeline_slot().get_or_init(|| Pipeline {
        surface: ExternalSurface::new(device, queue),
        source: Mutex::new(TestCard::new(SOURCE_W, SOURCE_H, SOURCE_FPS)),
        buf: Mutex::new(Vec::with_capacity((SOURCE_W * SOURCE_H * 4) as usize)),
        last_tick: Mutex::new(Instant::now()),
    })
}

struct MultimediaApp;

impl App for MultimediaApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "multimedia · testcard"
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);
        Model {
            frames: 0,
            started_at: Instant::now(),
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => Model {
                frames: model.frames.wrapping_add(1),
                ..model
            },
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let secs = model.started_at.elapsed().as_secs_f32().max(0.001);
        let fps = model.frames as f32 / secs;

        let title = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(44.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            format!("multimedia — testcard {SOURCE_W}×{SOURCE_H} @ {SOURCE_FPS:.0} fps"),
            22.0,
            Color::from_rgba8(220, 230, 245, 255),
        );

        let canvas_style = Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        };

        // Marco oscuro debajo de la surface — vello pinta el fill, GPU
        // pinta el video encima con LoadOp::Load. El callback de
        // gpu_paint_with inicializa el pipeline en el primer frame
        // (cuando ya tenemos device/queue) y avanza el TestCard.
        let canvas = View::new(canvas_style)
            .fill(Color::from_rgba8(10, 12, 18, 255))
            .radius(10.0)
            .gpu_paint_with(move |device, queue, encoder, view, rect, viewport| {
                let pipe = pipeline_for(device, queue);
                let mut last = pipe.last_tick.lock();
                let now = Instant::now();
                let dt = now - *last;
                *last = now;
                let mut buf = pipe.buf.lock();
                if let Some((w, h)) = pipe.source.lock().tick(dt, &mut buf) {
                    pipe.surface.upload(&buf, w, h);
                }
                drop(buf);
                pipe.surface.blit(queue, encoder, view, rect, viewport);
            });

        let footer = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            format!("ticks {} · ui ≈ {fps:.1} fps", model.frames),
            14.0,
            Color::from_rgba8(150, 165, 185, 255),
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(12.0_f32),
            },
            padding: TaffyRect {
                left: length(24.0_f32),
                right: length(24.0_f32),
                top: length(16.0_f32),
                bottom: length(16.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(22, 26, 34, 255))
        .children(vec![title, canvas, footer])
    }
}

fn main() {
    llimphi_ui::run::<MultimediaApp>();
}
