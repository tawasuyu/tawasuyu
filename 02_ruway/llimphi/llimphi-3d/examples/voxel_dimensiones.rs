//! Demo de M5 — **dimensiones / mundos paralelos**. Tres mundos voxel
//! independientes (Jardín, Inframundo, Cristal), cada uno con su grid, su cielo,
//! su sol y sus entidades. La cámara ve la dimensión activa; "viajar" = cambiar
//! cuál se renderiza.
//!
//! - **Arrastrar**: orbita. **Rueda**: zoom.
//! - **Tab / N**: siguiente dimensión. **P**: anterior. **1/2/3**: ir a una.
//! - Las entidades de la dimensión activa orbitan solas.
//!
//! `cargo run -p llimphi-3d --example voxel_dimensiones --release`
//! `… --release -- --shot` → vuelca un PNG por dimensión a /tmp/m5_*.png

use std::sync::{Arc, Mutex};
use std::time::Duration;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Camera3d, Dimension, Entity3d, Multiverse, VoxelGrid};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::{
    mount, paint_gpu, App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View,
    WheelDelta,
};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DIM: u32 = 64;

// ── Construcción de los tres mundos ─────────────────────────────────────────

fn world_jardin(d: u32) -> Dimension {
    Dimension::new("Jardín", VoxelGrid::demo_scene([d, d, d]))
        .with_sky([20, 30, 26])
        .with_sun([0.5, 1.0, 0.35])
        .with_entities(orbit_entities(
            d,
            &[[235, 70, 70], [70, 220, 110], [90, 130, 250], [240, 200, 60]],
        ))
}

fn world_inframundo(d: u32) -> Dimension {
    let mut g = VoxelGrid::new([d, d, d]);
    // Piso de lava (damero rojo/naranja).
    for z in 0..d {
        for x in 0..d {
            let chk = ((x / 4 + z / 4) % 2) == 0;
            let c = if chk { [150, 45, 22] } else { [185, 70, 28] };
            for y in 0..2 {
                g.set(x, y, z, c);
            }
        }
    }
    // Estalagmitas (columnas que se afinan hacia arriba).
    for &(sx, sz, h) in &[(d / 4, d / 4, d * 2 / 5), (d * 3 / 4, d / 3, d / 2), (d / 2, d * 3 / 4, d * 3 / 5), (d / 5, d * 4 / 5, d * 3 / 10)] {
        for y in 2..(2 + h).min(d) {
            let t = (y - 2) as f32 / h as f32;
            let r = ((1.0 - t) * 3.0).round() as i32;
            for dx in -r..=r {
                for dz in -r..=r {
                    let x = sx as i32 + dx;
                    let z = sz as i32 + dz;
                    if x >= 0 && z >= 0 {
                        let shade = 60 + (t * 70.0) as u8;
                        g.set(x as u32, y, z as u32, [120 + shade / 2, 50, 30]);
                    }
                }
            }
        }
    }
    Dimension::new("Inframundo", g)
        .with_sky([28, 8, 8])
        .with_sun([0.35, 0.7, 0.5])
        .with_entities(orbit_entities(d, &[[255, 140, 30], [255, 90, 20], [255, 200, 60]]))
}

fn world_cristal(d: u32) -> Dimension {
    let mut g = VoxelGrid::new([d, d, d]);
    // Cristales octaédricos flotando en el vacío (sin piso).
    let crystals: [(u32, u32, u32, [u8; 3]); 6] = [
        (d / 2, d * 3 / 4, d / 2, [120, 220, 255]),
        (d / 3, d / 2, d * 2 / 3, [200, 160, 255]),
        (d * 2 / 3, d * 3 / 5, d / 3, [160, 255, 220]),
        (d / 4, d * 2 / 3, d / 4, [255, 240, 200]),
        (d * 3 / 4, d / 2, d * 3 / 4, [180, 200, 255]),
        (d / 2, d / 3, d * 4 / 5, [220, 180, 255]),
    ];
    for (cx, cy, cz, col) in crystals {
        let r = 4i32;
        for dx in -r..=r {
            for dy in -r..=r {
                for dz in -r..=r {
                    if dx.abs() + dy.abs() + dz.abs() <= r {
                        let x = cx as i32 + dx;
                        let y = cy as i32 + dy;
                        let z = cz as i32 + dz;
                        if x >= 0 && y >= 0 && z >= 0 {
                            g.set(x as u32, y as u32, z as u32, col);
                        }
                    }
                }
            }
        }
    }
    Dimension::new("Cristal", g)
        .with_sky([10, 10, 22])
        .with_sun([0.4, 0.8, 0.45])
        .with_entities(orbit_entities(d, &[[120, 240, 255], [220, 180, 255]]))
}

/// Entidades distribuidas en una órbita ecuatorial (se animan girando).
fn orbit_entities(d: u32, colors: &[[u8; 3]]) -> Vec<Entity3d> {
    let n = colors.len();
    let df = d as f32;
    (0..n)
        .map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            Entity3d {
                pos: [df * 0.5 + a.cos() * df * 0.42, df * 0.45, df * 0.5 + a.sin() * df * 0.42],
                half: [df * 0.05, df * 0.05, df * 0.05],
                color: colors[k],
            }
        })
        .collect()
}

fn build_multiverse(d: u32) -> Multiverse {
    Multiverse::new(vec![world_jardin(d), world_inframundo(d), world_cristal(d)])
}

fn rotate_y(e: &mut Entity3d, center: [f32; 3], ang: f32) {
    let dx = e.pos[0] - center[0];
    let dz = e.pos[2] - center[2];
    let (s, c) = ang.sin_cos();
    e.pos[0] = center[0] + dx * c - dz * s;
    e.pos[2] = center[2] + dx * s + dz * c;
}

// ── App interactiva ─────────────────────────────────────────────────────────

#[derive(Clone)]
enum Msg {
    Orbit(f32, f32),
    Zoom(f32),
    Tick,
    Next,
    Prev,
    Go(usize),
}

struct Model {
    yaw: f32,
    pitch: f32,
    dist: f32,
    active: usize,
    names: Vec<String>,
    skies: Vec<[u8; 3]>,
    mv: Arc<Mutex<Multiverse>>,
}

struct DimApp;

impl App for DimApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi-3d · dimensiones"
    }
    fn initial_size() -> (u32, u32) {
        (1000, 720)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(Duration::from_millis(33), || Msg::Tick);
        let mv = build_multiverse(DIM);
        Model {
            yaw: 35_f32.to_radians(),
            pitch: 30_f32.to_radians(),
            dist: DIM as f32 * 1.7,
            active: mv.active(),
            names: mv.names(),
            skies: mv.skies(),
            mv: Arc::new(Mutex::new(mv)),
        }
    }

    fn window_title(model: &Model) -> Option<String> {
        Some(format!(
            "llimphi-3d · {} ({}/{})  —  Tab=siguiente",
            model.names[model.active],
            model.active + 1,
            model.names.len()
        ))
    }

    fn on_key(_model: &Model, ev: &KeyEvent) -> Option<Msg> {
        if !matches!(ev.state, KeyState::Pressed) {
            return None;
        }
        match &ev.key {
            Key::Named(NamedKey::Tab) => Some(Msg::Next),
            Key::Character(c) => match c.as_str() {
                "n" | "N" => Some(Msg::Next),
                "p" | "P" => Some(Msg::Prev),
                "1" => Some(Msg::Go(0)),
                "2" => Some(Msg::Go(1)),
                "3" => Some(Msg::Go(2)),
                _ => None,
            },
            _ => None,
        }
    }

    fn on_wheel(_m: &Model, delta: WheelDelta, _c: (f32, f32), _mods: Modifiers) -> Option<Msg> {
        Some(Msg::Zoom(delta.y))
    }

    fn update(mut model: Model, msg: Msg, _handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Orbit(dx, dy) => {
                model.yaw -= dx * 0.008;
                model.pitch += dy * 0.008;
            }
            Msg::Zoom(dy) => {
                let f = (1.0 + dy * 0.1).clamp(0.5, 1.5);
                model.dist = (model.dist * f).clamp(DIM as f32 * 0.5, DIM as f32 * 4.0);
            }
            Msg::Tick => {
                // Anima las entidades de la dimensión activa.
                let mut mv = model.mv.lock().unwrap();
                let c = [DIM as f32 * 0.5, DIM as f32 * 0.45, DIM as f32 * 0.5];
                for e in &mut mv.active_dim_mut().entities {
                    rotate_y(e, c, 0.02);
                }
            }
            Msg::Next => {
                model.mv.lock().unwrap().next();
                model.active = model.mv.lock().unwrap().active();
            }
            Msg::Prev => {
                model.mv.lock().unwrap().prev();
                model.active = model.mv.lock().unwrap().active();
            }
            Msg::Go(i) => {
                let mut mv = model.mv.lock().unwrap();
                mv.switch(i);
                model.active = mv.active();
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let camera = Camera3d::orbit(Vec3::ZERO, model.yaw, model.pitch, model.dist);
        let mv = model.mv.clone();

        let canvas = View::new(fill())
            .gpu_paint_with(move |device, queue, encoder, target, _rect, vp| {
                mv.lock().unwrap().render(device, queue, encoder, target, vp, &camera);
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
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--shot") {
        shot();
        return;
    }
    llimphi_ui::run::<DimApp>();
}

/// Vuelca un PNG por dimensión por el compositor real (mount → paint_gpu).
fn shot() {
    const W: u32 = 1000;
    const H: u32 = 720;
    let mv = Arc::new(Mutex::new(build_multiverse(DIM)));
    let camera = Camera3d::orbit(Vec3::ZERO, 35_f32.to_radians(), 30_f32.to_radians(), DIM as f32 * 1.7);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let count = mv.lock().unwrap().count();

    for i in 0..count {
        let (name, sky) = {
            let mut g = mv.lock().unwrap();
            g.switch(i);
            (g.active_name().to_string(), g.active_dim().sky)
        };
        let model_mv = mv.clone();
        let cam = camera;
        let canvas: View<Msg> = View::new(fill()).gpu_paint_with(
            move |device, queue, encoder, target, _rect, vp| {
                model_mv.lock().unwrap().render(device, queue, encoder, target, vp, &cam);
            },
        );
        let view: View<Msg> = View::new(fill()).children(vec![canvas]);

        let mut layout = LayoutTree::new();
        let mounted = mount(&mut layout, view);
        let computed = layout.compute(mounted.root, (W as f32, H as f32)).expect("layout");

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
        renderer
            .render_to_view(&hal, &vello::Scene::new(), &inter_view, W, H, Color::from_rgba8(sky[0], sky[1], sky[2], 255))
            .expect("base");

        let mut enc = hal
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gpu") });
        let any = paint_gpu(&mounted, &computed, &hal.device, &hal.queue, &mut enc, &inter_view, (W, H));
        hal.queue.submit(std::iter::once(enc.finish()));
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
        assert!(any, "gpu_painter no corrió");

        let out = format!("/tmp/m5_{i}_{}.png", name.to_lowercase());
        write_png(&hal, &inter, W, H, &out);
        eprintln!("dimensión {i} = {name} → {out}");
    }
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
