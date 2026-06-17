//! # llimphi-voxel-app — showcase del motor voxel 3D
//!
//! Un mundo voxel procedural que se **orbita con el mouse**, con atmósfera
//! (cielo + niebla) y un **monumento-malla** flotante que gira: prueba en vivo
//! que voxels (ray-march) y triángulos conviven en una escena con oclusión
//! correcta ([`llimphi_3d::Scene3d`]).
//!
//! Capas: `llimphi-voxel-app → llimphi-voxel (terreno) → llimphi-3d (motor) →
//! wgpu`. Arranca como demo; pensada para ganar personalidad (un juego) sin
//! tocar el motor — el contenido vive en [`world`].
//!
//! ```bash
//! cargo run -p llimphi-voxel-app --release            # ventana interactiva
//! cargo run -p llimphi-voxel-app --release -- --shot  # PNG headless a /tmp
//! ```
//! - **Arrastrar**: orbita. **Rueda**: zoom.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use llimphi_3d::glam::Vec3;
use llimphi_3d::Camera3d;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::{mount, paint_gpu, App, DragPhase, Handle, Modifiers, View, WheelDelta};

mod world;
use world::{World, FMT};

const DIM_XZ: u32 = 192;
const SEED: u32 = 1337;

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
    angle: f32,
    /// Mundo perezoso: se construye en la 1ª pintada GPU (ahí hay device/queue).
    world: Arc<Mutex<Option<World>>>,
}

/// `y` del centro de órbita (sobre el nivel medio del mundo).
fn focus_y() -> f32 {
    let dy = (DIM_XZ * 4 / 10).max(48) as f32;
    dy * 0.10
}

struct VoxelApp;

impl App for VoxelApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi-voxel-app"
    }
    fn initial_size() -> (u32, u32) {
        (1000, 720)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(Duration::from_millis(33), || Msg::Tick);
        Model {
            yaw: 35_f32.to_radians(),
            pitch: 22_f32.to_radians(),
            dist: DIM_XZ as f32 * 1.5,
            angle: 0.0,
            world: Arc::new(Mutex::new(None)),
        }
    }

    fn on_wheel(_m: &Model, delta: WheelDelta, _c: (f32, f32), _mods: Modifiers) -> Option<Msg> {
        Some(Msg::Zoom(delta.y))
    }

    fn update(mut model: Model, msg: Msg, _handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Orbit(dx, dy) => {
                model.yaw -= dx * 0.008;
                let lim = std::f32::consts::FRAC_PI_2 - 0.05;
                model.pitch = (model.pitch + dy * 0.008).clamp(-lim, lim);
            }
            Msg::Zoom(dy) => {
                let f = (1.0 + dy * 0.1).clamp(0.5, 1.5);
                model.dist = (model.dist * f).clamp(DIM_XZ as f32 * 0.6, DIM_XZ as f32 * 3.0);
            }
            Msg::Tick => model.angle += 0.01,
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let camera = Camera3d::orbit(Vec3::new(0.0, focus_y(), 0.0), model.yaw, model.pitch, model.dist);
        let world = model.world.clone();
        let angle = model.angle;

        let canvas = View::new(fill())
            .gpu_paint_with(move |device, queue, encoder, target, _rect, vp| {
                let mut guard = world.lock().unwrap();
                let w = guard.get_or_insert_with(|| World::build(device, queue, DIM_XZ, SEED));
                w.animate(angle);
                w.render(device, queue, encoder, target, vp, &camera);
            })
            .draggable(|phase, dx, dy| match phase {
                DragPhase::Move => Some(Msg::Orbit(dx, dy)),
                DragPhase::End => None,
            });

        View::new(fill()).children(vec![canvas])
    }
}

fn fill() -> Style {
    Style {
        size: Size {
            width: percent(1.0),
            height: percent(1.0),
        },
        ..Default::default()
    }
}

fn main() {
    if std::env::args().any(|a| a == "--shot") {
        shot();
        return;
    }
    llimphi_ui::run::<VoxelApp>();
}

/// Render headless de la escena por el compositor real (mount → paint_gpu),
/// para verificar sin pantalla que el nodo `gpu_paint_with` de la app corre y
/// produce el mundo. Vuelca /tmp/voxel_app.png.
fn shot() {
    const W: u32 = 1000;
    const H: u32 = 720;
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    let world: Arc<Mutex<Option<World>>> = Arc::new(Mutex::new(None));
    let camera = Camera3d::orbit(
        Vec3::new(0.0, focus_y(), 0.0),
        35_f32.to_radians(),
        22_f32.to_radians(),
        DIM_XZ as f32 * 1.5,
    );
    let w_arc = world.clone();
    let canvas: View<Msg> = View::new(fill()).gpu_paint_with(move |device, queue, encoder, target, _rect, vp| {
        let mut guard = w_arc.lock().unwrap();
        let w = guard.get_or_insert_with(|| World::build(device, queue, DIM_XZ, SEED));
        w.animate(0.6);
        w.render(device, queue, encoder, target, vp, &camera);
    });
    let view: View<Msg> = View::new(fill()).children(vec![canvas]);

    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, view);
    let computed = layout.compute(mounted.root, (W as f32, H as f32)).expect("layout");

    let inter = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inter"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
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
    renderer
        .render_to_view(&hal, &vello::Scene::new(), &inter_view, W, H, Color::from_rgba8(0, 0, 0, 255))
        .expect("base");

    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gpu") });
    let any = paint_gpu(&mounted, &computed, &hal.device, &hal.queue, &mut enc, &inter_view, (W, H));
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    assert!(any, "el gpu_painter de la app no corrió");

    write_png(&hal, &inter, W, H, "/tmp/voxel_app.png");
    eprintln!("voxel_app: escrito /tmp/voxel_app.png ({W}x{H})");
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
    for row in 0..h as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wtr = enc.write_header().unwrap();
    wtr.write_image_data(&pixels).unwrap();
}
