//! # llimphi-voxel-app — showcase del motor voxel 3D
//!
//! Un mundo voxel procedural con dos modos:
//!
//! - **Órbita** (default): se mira el continente desde afuera, con atmósfera
//!   (cielo + niebla) y un **monumento-malla** flotante que gira — prueba en
//!   vivo que voxels (ray-march) y triángulos conviven con oclusión correcta
//!   ([`llimphi_3d::Scene3d`]).
//! - **Explorar** (Tab): cámara en **primera persona** caminando el terreno con
//!   **gravedad y colisión** ([`llimphi_voxel::Player`]) — el bucle canónico de
//!   un juego voxel: caminar, mirar, romper, construir.
//!
//! Capas: `llimphi-voxel-app → llimphi-voxel (terreno/jugador/picking) →
//! llimphi-3d (motor) → wgpu`. El contenido vive en [`world`]; el motor no sabe
//! nada de juegos.
//!
//! ```bash
//! cargo run -p llimphi-voxel-app --release            # ventana interactiva
//! cargo run -p llimphi-voxel-app --release -- --shot  # PNG headless a /tmp
//! ```
//! - **Tab**: alterna órbita ↔ explorar (primera persona).
//! - **Arrastrar**: orbita / mira. **Rueda**: zoom (órbita).
//! - **WASD**: caminar (explorar). **Espacio**: saltar.
//! - **b**: romper (cráter donde mira la cámara). **g**: construir.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use llimphi_3d::glam::Vec3;
use llimphi_3d::Camera3d;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::prelude::{
    percent, Position, Size, Style,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::{
    mount, paint_gpu, App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View,
    WheelDelta,
};
use llimphi_voxel::{forward_h, right_h};

mod film;
mod soundtrack;
mod world;
use world::{World, FMT};

const DIM_XZ: u32 = 192;
const SEED: u32 = 1337;
/// Paso de física por frame (el `Tick` periódico corre a ~30 Hz).
const DT: f32 = 1.0 / 30.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Orbit,
    Explore,
}

/// Dirección de caminata pulsada (WASD).
#[derive(Clone, Copy)]
enum Walk {
    Fwd,
    Back,
    Left,
    Right,
}

/// Snapshot del estado de teclas de movimiento (lo lee el paso de física).
#[derive(Clone, Copy, Default)]
struct Input {
    fwd: bool,
    back: bool,
    left: bool,
    right: bool,
    jump: bool,
}

#[derive(Clone)]
enum Msg {
    Orbit(f32, f32),
    Zoom(f32),
    Edit(bool),
    ToggleMode,
    Move(Walk, bool),
    Jump(bool),
    Tick,
}

struct Model {
    mode: Mode,
    yaw: f32,
    pitch: f32,
    dist: f32,
    angle: f32,
    input: Input,
    /// Mundo perezoso: se construye en la 1ª pintada GPU (ahí hay device/queue).
    world: Arc<Mutex<Option<World>>>,
    /// Cola de ediciones a aplicar en el próximo frame: `true` = construir,
    /// `false` = romper. El rayo se deriva de la cámara *de ese frame*.
    edits: Arc<Mutex<Vec<bool>>>,
    /// Pedido de reposar al jugador sobre el terreno (al entrar a explorar).
    respawn: Arc<Mutex<bool>>,
}

/// Cámara de órbita a partir del estado del modelo.
fn camera_of(yaw: f32, pitch: f32, dist: f32) -> Camera3d {
    Camera3d::orbit(Vec3::new(0.0, focus_y(), 0.0), yaw, pitch, dist)
}

/// `dim` del mundo (igual que `World::build`): grilla `[0, dim]`.
fn world_dim() -> Vec3 {
    let dy = (DIM_XZ * 4 / 10).max(48) as f32;
    Vec3::new(DIM_XZ as f32, dy, DIM_XZ as f32)
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
            mode: Mode::Orbit,
            yaw: 35_f32.to_radians(),
            pitch: 22_f32.to_radians(),
            dist: DIM_XZ as f32 * 1.5,
            angle: 0.0,
            input: Input::default(),
            world: Arc::new(Mutex::new(None)),
            edits: Arc::new(Mutex::new(Vec::new())),
            respawn: Arc::new(Mutex::new(false)),
        }
    }

    fn on_wheel(_m: &Model, delta: WheelDelta, _c: (f32, f32), _mods: Modifiers) -> Option<Msg> {
        Some(Msg::Zoom(delta.y))
    }

    fn on_key(_m: &Model, ev: &KeyEvent) -> Option<Msg> {
        let pressed = matches!(ev.state, KeyState::Pressed);
        match &ev.key {
            Key::Named(NamedKey::Tab) if pressed && !ev.repeat => Some(Msg::ToggleMode),
            Key::Named(NamedKey::Space) => Some(Msg::Jump(pressed)),
            Key::Character(c) => match c.to_ascii_lowercase().as_str() {
                "w" => Some(Msg::Move(Walk::Fwd, pressed)),
                "s" => Some(Msg::Move(Walk::Back, pressed)),
                "a" => Some(Msg::Move(Walk::Left, pressed)),
                "d" => Some(Msg::Move(Walk::Right, pressed)),
                "b" if pressed && !ev.repeat => Some(Msg::Edit(false)), // romper
                "g" if pressed && !ev.repeat => Some(Msg::Edit(true)),  // construir
                _ => None,
            },
            _ => None,
        }
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
            Msg::Edit(build) => model.edits.lock().unwrap().push(build),
            Msg::ToggleMode => {
                model.mode = match model.mode {
                    Mode::Orbit => {
                        *model.respawn.lock().unwrap() = true;
                        Mode::Explore
                    }
                    Mode::Explore => Mode::Orbit,
                };
            }
            Msg::Move(w, on) => match w {
                Walk::Fwd => model.input.fwd = on,
                Walk::Back => model.input.back = on,
                Walk::Left => model.input.left = on,
                Walk::Right => model.input.right = on,
            },
            Msg::Jump(on) => model.input.jump = on,
            Msg::Tick => model.angle += 0.01,
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let mode = model.mode;
        let (yaw, pitch, dist, angle, input) =
            (model.yaw, model.pitch, model.dist, model.angle, model.input);
        let world = model.world.clone();
        let edits = model.edits.clone();
        let respawn = model.respawn.clone();

        let canvas = View::new(fill_absolute())
            .gpu_paint_with(move |device, queue, encoder, target, _rect, vp| {
                let mut guard = world.lock().unwrap();
                let w = guard.get_or_insert_with(|| World::build(device, queue, DIM_XZ, SEED));

                // Cámara del frame: en explorar, primero avanza la física.
                let camera = match mode {
                    Mode::Explore => {
                        if std::mem::take(&mut *respawn.lock().unwrap()) {
                            w.respawn_player();
                        }
                        let mut wish = Vec3::ZERO;
                        if input.fwd {
                            wish += forward_h(yaw);
                        }
                        if input.back {
                            wish -= forward_h(yaw);
                        }
                        if input.right {
                            wish += right_h(yaw);
                        }
                        if input.left {
                            wish -= right_h(yaw);
                        }
                        let eye = w.step_player(wish, input.jump, DT);
                        Camera3d::fly(eye, yaw, pitch)
                    }
                    Mode::Orbit => camera_of(yaw, pitch, dist),
                };

                // Ediciones: el rayo sale de la cámara de este frame (en órbita,
                // desde afuera; en explorar, desde el ojo del jugador).
                let pending = std::mem::take(&mut *edits.lock().unwrap());
                if !pending.is_empty() {
                    let ro = camera.eye + world_dim() * 0.5; // a espacio de grilla
                    let rd = (camera.target - camera.eye).normalize();
                    for build in pending {
                        w.apply_edit(queue, [ro.x, ro.y, ro.z], [rd.x, rd.y, rd.z], build);
                    }
                }

                w.tick(DT); // la manada deambula
                w.animate(angle);
                w.render(device, queue, encoder, target, vp, &camera);
                if mode == Mode::Explore {
                    w.draw_hud(device, queue, encoder, target, vp);
                }
            })
            .draggable(|phase, dx, dy| match phase {
                DragPhase::Move => Some(Msg::Orbit(dx, dy)),
                DragPhase::End => None,
            });

        View::new(root()).children(vec![canvas])
    }
}

/// Raíz a pantalla completa que aloja el canvas 3D.
fn root() -> Style {
    Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    }
}

/// Canvas a pantalla completa, posicionado absoluto.
fn fill_absolute() -> Style {
    Style {
        position: Position::Absolute,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--film") {
        film::film();
        return;
    }
    if args.iter().any(|a| a == "--born") {
        film::born();
        return;
    }
    if args.iter().any(|a| a == "--poses") {
        film::poses_shot();
        return;
    }
    if args.iter().any(|a| a == "--vox") {
        film::vox_shot();
        return;
    }
    if args.iter().any(|a| a == "--shot") {
        shot();
        return;
    }
    llimphi_ui::run::<VoxelApp>();
}

/// Render headless de la escena por el compositor real (mount → paint_gpu),
/// para verificar sin pantalla que el nodo `gpu_paint_with` de la app corre y
/// produce el mundo. Vuelca dos PNG: la vista órbita y la primera persona.
fn shot() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    // Mundo compartido entre las dos tomas (se construye en la 1ª).
    let world: Arc<Mutex<Option<World>>> = Arc::new(Mutex::new(None));

    // --- Toma 1: órbita (con un cráter, prueba romper desde afuera) ---
    let cam_orbit = Camera3d::orbit(
        Vec3::new(0.0, focus_y(), 0.0),
        35_f32.to_radians(),
        22_f32.to_radians(),
        DIM_XZ as f32 * 1.5,
    );
    shot_one(&hal, &mut renderer, &world, cam_orbit, true, "/tmp/voxel_app.png");

    // --- Toma 2: primera persona, parado en un mirador del borde, mirando al
    // bicho más cercano (encuadra la manada que deambula el terreno) ---
    let cam_fps = {
        let mut guard = world.lock().unwrap();
        let w = guard.as_mut().expect("mundo ya construido en la toma 1");
        w.spawn_player_at(DIM_XZ / 5, DIM_XZ / 5);
        // Unos pasos de física para asentar al jugador sobre el suelo.
        let mut eye = Vec3::ZERO;
        for _ in 0..4 {
            eye = w.step_player(Vec3::ZERO, false, DT);
        }
        let target = w.nearest_critter(eye).unwrap_or(eye + Vec3::Z);
        Camera3d { eye, target, ..Camera3d::default() }
    };
    shot_one(&hal, &mut renderer, &world, cam_fps, false, "/tmp/voxel_app_fps.png");
}

/// Renderiza una toma con `camera` a `path`. Si `edit`, cava un cráter por el
/// camino real (`apply_edit`) para verificar también la mecánica de romper.
fn shot_one(
    hal: &Hal,
    renderer: &mut Renderer,
    world: &Arc<Mutex<Option<World>>>,
    camera: Camera3d,
    edit: bool,
    path: &str,
) {
    const W: u32 = 1000;
    const H: u32 = 720;
    let world = world.clone();
    let ro = camera.eye + world_dim() * 0.5;
    let rd = (camera.target - camera.eye).normalize();
    let canvas: View<Msg> = View::new(fill_absolute()).gpu_paint_with(
        move |device, queue, encoder, target, _rect, vp| {
            let mut guard = world.lock().unwrap();
            let w = guard.get_or_insert_with(|| World::build(device, queue, DIM_XZ, SEED));
            if edit {
                w.apply_edit(queue, [ro.x, ro.y, ro.z], [rd.x, rd.y, rd.z], false);
            }
            w.animate(0.6);
            w.render(device, queue, encoder, target, vp, &camera);
            // La toma de primera persona (`!edit`) lleva el HUD, como en vivo.
            if !edit {
                w.draw_hud(device, queue, encoder, target, vp);
            }
        },
    );
    let view: View<Msg> = View::new(root()).children(vec![canvas]);

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
        .render_to_view(hal, &vello::Scene::new(), &inter_view, W, H, Color::from_rgba8(0, 0, 0, 255))
        .expect("base");

    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gpu") });
    let any = paint_gpu(&mounted, &computed, &hal.device, &hal.queue, &mut enc, &inter_view, (W, H));
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    assert!(any, "el gpu_painter de la app no corrió");

    write_png(hal, &inter, W, H, path);
    eprintln!("voxel_app: escrito {path} ({W}x{H})");
}

fn write_png(hal: &Hal, target: &wgpu::Texture, w: u32, h: u32, path: &str) {
    let pixels = readback_rgba(hal, target, w, h);
    encode_png(&pixels, w, h, path);
}

/// Lee de vuelta una textura RGBA8 `w×h` a un `Vec<u8>` plano (sin padding).
fn readback_rgba(hal: &Hal, target: &wgpu::Texture, w: u32, h: u32) -> Vec<u8> {
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
    pixels
}

/// Codifica RGBA8 plano `w×h` a un PNG en `path`.
fn encode_png(pixels: &[u8], w: u32, h: u32, path: &str) {
    use std::fs::File;
    use std::io::BufWriter;
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wtr = enc.write_header().unwrap();
    wtr.write_image_data(pixels).unwrap();
}

/// Lee una textura `src_w×src_h` y la **baja por supersampling** (promedio de
/// bloques `factor×factor`) a `(src_w/factor)×(src_h/factor)` antes de escribir el
/// PNG — antialias de los bordes duros del ray-march (SSAA). `factor=1` = directo.
fn write_png_downsampled(hal: &Hal, target: &wgpu::Texture, src_w: u32, src_h: u32, factor: u32, path: &str) {
    let src = readback_rgba(hal, target, src_w, src_h);
    let f = factor.max(1);
    if f == 1 {
        encode_png(&src, src_w, src_h, path);
        return;
    }
    let (dw, dh) = (src_w / f, src_h / f);
    let mut dst = vec![0u8; (dw * dh * 4) as usize];
    let n = (f * f) as u32;
    for dy in 0..dh {
        for dx in 0..dw {
            let mut acc = [0u32; 4];
            for sy in 0..f {
                for sx in 0..f {
                    let px = (dx * f + sx) as usize;
                    let py = (dy * f + sy) as usize;
                    let o = (py * src_w as usize + px) * 4;
                    for c in 0..4 {
                        acc[c] += src[o + c] as u32;
                    }
                }
            }
            let o = ((dy * dw + dx) * 4) as usize;
            for c in 0..4 {
                dst[o + c] = (acc[c] / n) as u8;
            }
        }
    }
    encode_png(&dst, dw, dh, path);
}
