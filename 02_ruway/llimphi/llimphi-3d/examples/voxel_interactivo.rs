//! Demo **interactivo** del motor 3D: el mundo voxel (M1-M4) dentro de un
//! `View` vivo de Llimphi, manejado con el mouse.
//!
//! - **Arrastrar** (botón izquierdo): orbita la cámara (yaw/pitch).
//! - **Rueda**: zoom (acerca/aleja).
//! - Las 4 entidades de colores orbitan solas (animación por `spawn_periodic`).
//!
//! Es el cableado real a una app: el `VoxelRenderer` se compone dentro del
//! árbol `View<Msg>` por `View::gpu_paint_with` (corre DESPUÉS de la pasada
//! vello, con `LoadOp::Load`). El renderer se crea perezosamente en la primera
//! llamada GPU (ahí recién hay `Device`/`Queue`) y se cachea en el Model tras
//! un `Arc<Mutex<…>>`.
//!
//! `cargo run -p llimphi-3d --example voxel_interactivo --release -- [dim]`

use std::sync::{Arc, Mutex};
use std::time::Duration;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Camera3d, Entity3d, VoxelGrid, VoxelRenderer};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::{mount, paint_gpu, App, DragPhase, Handle, Modifiers, View, WheelDelta};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Orbit(f32, f32),
    Zoom(f32),
    Tick,
}

struct Model {
    yaw: f32,
    pitch: f32,
    dist: f32,
    phase: f32,
    dim: u32,
    grid: Arc<VoxelGrid>,
    /// Renderer voxel, creado en la 1ª pintada GPU (necesita el Device).
    engine: Arc<Mutex<Option<VoxelRenderer>>>,
}

fn entities_at(phase: f32, dim: u32) -> Vec<Entity3d> {
    let d = dim as f32;
    let colors = [[235u8, 70, 70], [70, 220, 110], [90, 130, 250], [240, 200, 60]];
    (0..4)
        .map(|k| {
            let a = phase + k as f32 * std::f32::consts::FRAC_PI_2;
            let radius = d * 0.42;
            Entity3d {
                pos: [
                    d * 0.5 + a.cos() * radius,
                    d * (0.45 + 0.12 * (a * 1.3).sin()),
                    d * 0.5 + a.sin() * radius,
                ],
                half: [d * 0.05, d * 0.05, d * 0.05],
                color: colors[k],
            }
        })
        .collect()
}

struct VoxelApp;

impl App for VoxelApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi-3d · motor voxel interactivo"
    }

    fn initial_size() -> (u32, u32) {
        (1000, 720)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let dim: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(64);
        // Anima las entidades a ~30 fps.
        handle.spawn_periodic(Duration::from_millis(33), || Msg::Tick);
        Model {
            yaw: 35_f32.to_radians(),
            pitch: 30_f32.to_radians(),
            dist: dim as f32 * 1.7,
            phase: 0.0,
            dim,
            grid: Arc::new(VoxelGrid::demo_scene([dim, dim, dim])),
            engine: Arc::new(Mutex::new(None)),
        }
    }

    fn update(mut model: Model, msg: Msg, _handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Orbit(dx, dy) => {
                model.yaw -= dx * 0.008;
                model.pitch += dy * 0.008;
            }
            Msg::Zoom(dy) => {
                // Rueda hacia adelante = acercar (reduce la distancia). El signo
                // va invertido respecto del delta crudo para que sea natural.
                let f = (1.0 + dy * 0.1).clamp(0.5, 1.5);
                let d = model.dim as f32;
                model.dist = (model.dist * f).clamp(d * 0.5, d * 4.0);
            }
            Msg::Tick => {
                model.phase += 0.035;
            }
        }
        model
    }

    fn on_wheel(
        _model: &Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Msg> {
        Some(Msg::Zoom(delta.y))
    }

    fn view(model: &Model) -> View<Msg> {
        let camera = Camera3d::orbit(Vec3::ZERO, model.yaw, model.pitch, model.dist);
        let entities = entities_at(model.phase, model.dim);
        let engine = model.engine.clone();
        let grid = model.grid.clone();

        let canvas = View::new(Style {
            size: Size {
                width: percent(1.0),
                height: percent(1.0),
            },
            ..Default::default()
        })
        .gpu_paint_with(move |device, queue, encoder, target, _rect, vp| {
            let mut guard = engine.lock().unwrap();
            let er = guard.get_or_insert_with(|| VoxelRenderer::new(device, queue, FMT, &grid));
            er.entities = entities.clone();
            er.render(device, queue, encoder, target, vp, &camera);
        })
        .draggable(|phase, dx, dy| match phase {
            DragPhase::Move => Some(Msg::Orbit(dx, dy)),
            DragPhase::End => None,
        });

        View::new(Style {
            size: Size {
                width: percent(1.0),
                height: percent(1.0),
            },
            ..Default::default()
        })
        .children(vec![canvas])
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Modo verificación headless: monta el MISMO View por el compositor real
    // (mount → compute → paint_gpu) y vuelca un PNG, sin abrir ventana.
    if let Some(i) = args.iter().position(|a| a == "--shot") {
        let out = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "/tmp/voxel_interactivo.png".to_string());
        shot(&out);
        return;
    }
    llimphi_ui::run::<VoxelApp>();
}

/// Render headless del árbol `View` de la app a través del compositor real.
fn shot(out: &str) {
    const W: u32 = 1000;
    const H: u32 = 720;
    let dim = 64u32;
    let model = Model {
        yaw: 35_f32.to_radians(),
        pitch: 30_f32.to_radians(),
        dist: dim as f32 * 1.7,
        phase: 0.6,
        dim,
        grid: Arc::new(VoxelGrid::demo_scene([dim, dim, dim])),
        engine: Arc::new(Mutex::new(None)),
    };

    // Árbol real de la app → mount + layout (igual que el runtime por frame).
    let view = VoxelApp::view(&model);
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, view);
    let computed = layout
        .compute(mounted.root, (W as f32, H as f32))
        .expect("layout");

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let inter = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inter"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());

    // Pasada vello base (fondo) — igual que el frame real.
    renderer
        .render_to_view(&hal, &vello::Scene::new(), &inter_view, W, H, Color::from_rgba8(18, 22, 32, 255))
        .expect("base");

    // Pasada GPU directo: dispara los gpu_painter del árbol (nuestro voxel).
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gpu") });
    let any = paint_gpu(&mounted, &computed, &hal.device, &hal.queue, &mut enc, &inter_view, (W, H));
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    assert!(any, "ningún gpu_painter corrió — el cableado no llegó al compositor");

    write_png(&hal, &inter, W, H, out);
    eprintln!("voxel_interactivo --shot: {out} ({W}x{H}) — gpu_painter del View ejecutado por el compositor");
}

fn write_png(hal: &Hal, target: &wgpu::Texture, w: u32, h: u32, path: &str) {
    use std::fs::File;
    use std::io::BufWriter;
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
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
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
    for row in 0..h as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut penc = png::Encoder::new(BufWriter::new(file), w, h);
    penc.set_color(png::ColorType::Rgba);
    penc.set_depth(png::BitDepth::Eight);
    let mut wr = penc.write_header().unwrap();
    wr.write_image_data(&pixels).unwrap();
}
