//! Bake **headless** de un [`FondoSpec`] Lottie/rive a la cache de frames.
//!
//! `vello 0.7` es GPU-only (no hay rasterizador CPU de un `Scene`), y las
//! superficies de presentación sin vello (splash, compositor) no pueden pintar
//! Lottie/rive en caliente. La salida: abrir una GPU **una sola vez, offline**,
//! renderizar cada frame a una textura, leerlo de vuelta a CPU y escribirlo como
//! PNG en la cache (`super::cache`). Después, esas superficies sólo bliteant.
//!
//! Sólo se compila con la feature `bake` (arrastra `llimphi-ui` + el stack
//! gráfico). El núcleo de lectura de la cache es liviano y vive aparte.
//!
//! ## Qué se bakea
//!
//! - **Lottie**: `llimphi_lottie::LottieAsset::paint_at_time` por frame, durante
//!   un loop = la duración del clip (o `loop_secs` si se fuerza).
//! - **rive**: el `Project` (`Doc` + `RigDoc`) del studio. Como el formato aún
//!   **no serializa pistas de animación por keyframe** (Fase 1 del studio), el
//!   rig se reproduce con una **deriva idle** procedural sobre su pose de reposo
//!   (un vaivén suave de cada hueso + órbita del objetivo de IK si está activo),
//!   la misma idea que da vida a «Alley Cat». La malla deformada se pinta
//!   texturizada (si el rig trae textura) o sólida.

use std::path::PathBuf;

use llimphi_anim_studio::Project;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_raster::kurbo::Rect;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::PaintRect;

use crate::cache::{self, CacheMeta};
use crate::FondoSpec;

/// Parámetros del bake.
#[derive(Debug, Clone, Copy)]
pub struct BakeOpts {
    /// Ancho de cada frame.
    pub width: u32,
    /// Alto de cada frame.
    pub height: u32,
    /// Cuadros por segundo.
    pub fps: f32,
    /// Duración del loop (segundos). `None` = la natural del asset (la duración
    /// del Lottie; para rive, un default de [`RIVE_LOOP_SECS`]).
    pub loop_secs: Option<f32>,
}

impl Default for BakeOpts {
    fn default() -> Self {
        // 720p / 30 fps: tamaño acotado (como el cap del compositor), suficiente
        // para escalar por GPU al pintar y barato de bakear/almacenar.
        BakeOpts { width: 1280, height: 720, fps: 30.0, loop_secs: None }
    }
}

/// Duración por defecto del loop de un rive bakeado (segundos).
pub const RIVE_LOOP_SECS: f32 = 6.0;
/// Tope duro de frames por bake (red de seguridad ante un loop/fps absurdo):
/// 60 s a 30 fps. Evita llenar el disco por una config errónea.
const MAX_FRAMES: u32 = 1800;
/// Color de limpieza del frame: el fondo de marca (opaco), así las zonas
/// transparentes del Lottie/rive muestran la marca y no negro.
const CLEAR: (u8, u8, u8) = (18, 18, 24);

/// Bakea `spec` a la cache de frames y devuelve `(carpeta, meta)`. Error si
/// `spec` es la chakana (no se bakea — se genera en CPU en caliente) o si el
/// asset no carga / la GPU no abre.
pub fn bake(spec: &FondoSpec, opts: &BakeOpts) -> Result<(PathBuf, CacheMeta), String> {
    match spec {
        FondoSpec::Chakana => {
            Err("la chakana no se bakea: se genera en CPU por frame (chakana_frame)".into())
        }
        FondoSpec::Lottie { path } => bake_lottie(spec, path, opts),
        FondoSpec::Rive { path } => bake_rive(spec, path, opts),
    }
}

/// ¿Hace falta bakear `spec`? `false` si ya hay cache (del mismo asset+mtime) o
/// si es la chakana (que no usa cache).
pub fn needs_bake(spec: &FondoSpec) -> bool {
    spec.needs_bake() && !cache::FrameCache::is_baked(spec)
}

/// Bakea si falta; si ya está fresco, no hace nada. Devuelve la meta de la cache.
pub fn ensure_baked(spec: &FondoSpec, opts: &BakeOpts) -> Result<CacheMeta, String> {
    if !spec.needs_bake() {
        return Err("la chakana no usa cache".into());
    }
    if let Ok(c) = cache::FrameCache::open_for(spec) {
        return Ok(c.meta().clone());
    }
    bake(spec, opts).map(|(_, meta)| meta)
}

fn frame_count(loop_secs: f32, fps: f32) -> u32 {
    ((loop_secs.max(0.0) * fps).round() as u32).clamp(1, MAX_FRAMES)
}

fn full_rect(w: u32, h: u32) -> PaintRect {
    PaintRect { x: 0.0, y: 0.0, w: w as f32, h: h as f32 }
}

fn bake_lottie(spec: &FondoSpec, path: &str, opts: &BakeOpts) -> Result<(PathBuf, CacheMeta), String> {
    let txt = std::fs::read_to_string(path).map_err(|e| format!("Lottie {path:?}: {e}"))?;
    let asset = llimphi_lottie::LottieAsset::from_str(&txt)
        .map_err(|e| format!("Lottie {path:?}: {e:?}"))?;
    let natural = asset.duration_secs() as f32;
    let loop_secs = opts.loop_secs.unwrap_or(if natural > 0.0 { natural } else { 2.0 });
    let frames = frame_count(loop_secs, opts.fps);

    let rect = full_rect(opts.width, opts.height);
    render_frames(spec, opts, loop_secs, frames, |i, scene| {
        let t = i as f64 / opts.fps as f64;
        asset.paint_at_time(scene, rect, t);
    })
}

fn bake_rive(spec: &FondoSpec, path: &str, opts: &BakeOpts) -> Result<(PathBuf, CacheMeta), String> {
    let project = Project::load(path)?;
    let base = project.rig;
    if base.bones.is_empty() {
        return Err(format!("rive {path:?}: el rig no tiene huesos"));
    }
    let loop_secs = opts.loop_secs.unwrap_or(RIVE_LOOP_SECS);
    let frames = frame_count(loop_secs, opts.fps);

    // Malla en bind space (constante); la deformación cambia por frame.
    let mesh = base.mesh();
    if mesh.vertices.is_empty() {
        return Err(format!("rive {path:?}: la malla quedó vacía"));
    }
    // Textura a deformar (si el rig la trae); si no, malla sólida con el acento.
    let texture = base.texture_path.as_deref().and_then(|p| {
        llimphi_image::load_path(std::path::Path::new(p), 64 * 1024 * 1024)
            .map_err(|e| eprintln!("rive: textura {p:?}: {e:?}"))
            .ok()
    });
    let accent = {
        let a = marca::Brand::Suite.meta().accent;
        Color::from_rgba8(a[0], a[1], a[2], a[3])
    };
    // Encaje: los bounds de reposo, **expandidos** para que el vaivén no se corte,
    // dentro del rect del frame.
    let bounds = pad_rect(llimphi_mesh::rest_bounds(&mesh), 0.35);
    let xform = llimphi_mesh::fit_transform(bounds, full_rect(opts.width, opts.height));

    // Amplitud del vaivén idle (radianes) y de la órbita del IK (unidades modelo).
    const SWAY: f64 = 0.18;
    const ORBIT: f64 = 24.0;
    let tau = std::f64::consts::TAU;

    render_frames(spec, opts, loop_secs, frames, |i, scene| {
        let phase = (i as f64 / frames as f64) * tau;
        let mut rig = base.clone();
        for (k, b) in rig.bones.iter_mut().enumerate() {
            b.angle += SWAY * (phase + k as f64 * 0.6).sin();
        }
        if rig.ik_enabled {
            rig.ik_target.0 += ORBIT * phase.cos();
            rig.ik_target.1 += ORBIT * (phase * 2.0).sin() * 0.5;
        }
        let skel = rig.skeleton();
        let positions = mesh.deform(&skel);
        match &texture {
            Some(tex) => llimphi_mesh::paint_textured(scene, &mesh, &positions, xform, tex),
            None => llimphi_mesh::paint_solid(scene, &mesh, &positions, xform, accent),
        }
    })
}

/// Expande un `Rect` por una fracción de su tamaño en cada lado.
fn pad_rect(r: Rect, frac: f64) -> Rect {
    let dx = r.width() * frac;
    let dy = r.height() * frac;
    Rect::new(r.x0 - dx, r.y0 - dy, r.x1 + dx, r.y1 + dy)
}

/// Loop común de bake: abre la GPU, prepara la cache y, para cada frame, deja que
/// `paint` componga el `Scene`, lo renderiza headless y lo escribe como PNG.
fn render_frames(
    spec: &FondoSpec,
    opts: &BakeOpts,
    loop_secs: f32,
    frames: u32,
    mut paint: impl FnMut(u32, &mut vello::Scene),
) -> Result<(PathBuf, CacheMeta), String> {
    let mut hl = Headless::new(opts.width, opts.height)?;
    let meta = CacheMeta {
        width: opts.width,
        height: opts.height,
        fps: opts.fps,
        frame_count: frames,
        loop_secs: frames as f32 / opts.fps,
    };
    let dir = cache::init_cache(spec, &meta).map_err(|e| format!("cache: {e}"))?;
    let clear = Color::from_rgba8(CLEAR.0, CLEAR.1, CLEAR.2, 255);

    for i in 0..frames {
        let mut scene = vello::Scene::new();
        paint(i, &mut scene);
        let rgba = hl.render(&scene, clear)?;
        cache::write_frame_rgba(&dir, i, &rgba, opts.width, opts.height)
            .map_err(|e| format!("frame {i}: {e}"))?;
    }
    eprintln!(
        "fondo-bake: {} → {} frames {}x{} @ {} fps (loop {:.1}s) en {:?}",
        spec.kind(),
        frames,
        opts.width,
        opts.height,
        opts.fps,
        loop_secs,
        dir
    );
    Ok((dir, meta))
}

/// Renderer headless reusable: una GPU + una textura destino + readback. Espeja
/// el patrón de captura de `wawa-panel-llimphi`.
struct Headless {
    hal: Hal,
    renderer: Renderer,
    target: wgpu::Texture,
    w: u32,
    h: u32,
}

impl Headless {
    fn new(w: u32, h: u32) -> Result<Self, String> {
        let hal = pollster::block_on(Hal::new(None)).map_err(|e| format!("GPU (hal): {e:?}"))?;
        let renderer = Renderer::new(&hal).map_err(|e| format!("renderer: {e:?}"))?;
        let target = hal.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fondo-bake-target"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Ok(Headless { hal, renderer, target, w, h })
    }

    /// Renderiza `scene` (limpiando con `clear`) y lee el frame de vuelta como
    /// RGBA8 **tight** (`w*h*4`).
    fn render(&mut self, scene: &vello::Scene, clear: Color) -> Result<Vec<u8>, String> {
        let tview = self.target.create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer
            .render_to_view(&self.hal, scene, &tview, self.w, self.h, clear)
            .map_err(|e| format!("render_to_view: {e:?}"))?;
        self.readback()
    }

    fn readback(&self) -> Result<Vec<u8>, String> {
        let (w, h) = (self.w, self.h);
        let unpadded = (w * 4) as usize;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let padded = unpadded.div_ceil(align) * align;
        let buf = self.hal.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fondo-bake-readback"),
            size: (padded * h as usize) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self
            .hal
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
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
        self.hal.queue.submit(std::iter::once(enc.finish()));
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.hal.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .map_err(|e| format!("readback canal: {e}"))?
            .map_err(|e| format!("readback map: {e:?}"))?;
        let data = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity(unpadded * h as usize);
        for row in 0..h as usize {
            let s = row * padded;
            pixels.extend_from_slice(&data[s..s + unpadded]);
        }
        drop(data);
        buf.unmap();
        Ok(pixels)
    }
}
