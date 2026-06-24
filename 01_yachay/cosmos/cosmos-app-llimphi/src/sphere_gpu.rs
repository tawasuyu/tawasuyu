//! Esfera celeste 3D sobre el motor GPU **`llimphi-3d`**.
//!
//! Reemplaza al wireframe vello proyectado a mano por una escena 3D real con
//! depth + cámara en perspectiva, orbitable. Tres capas en un pase
//! supersampleado (`PostFx`), bajado y confinado al rect del panel:
//!
//! 1. **Campo estelar de fondo** ([`SkyBackdrop`]): panorama cilíndrico de
//!    estrellas tenues generado en CPU que panea con el azimut de la cámara.
//! 2. **Cuerpos y estructura como puntos redondos** ([`Billboards`], sprites
//!    siempre de cara a la cámara): los aros (eclíptica, ecuador), los ticks
//!    del zodíaco, los polos, los ángulos ASC/MC y los planetas/estrellas
//!    son todos **puntos** de distinto tamaño/tinte. **Nada de cubos** — un
//!    aro es un collar de puntitos, un planeta un disco que brilla.
//!
//! Post-proceso suave: SSAA 2× (bordes limpios en los puntos) + un toque de
//! bloom que sólo prende los cuerpos brillantes, sin lavar el campo estelar.
//!
//! Integración Llimphi: la geometría (lista de billboards) se arma en CPU en
//! `view()` (sin GPU) y se sube/dibuja en `View::gpu_paint_with`. El
//! [`SphereGpu`] vive en el `Model` tras `Arc<Mutex<…>>`, creado perezoso en
//! el primer paint (ahí recién hay device).

use std::sync::{Arc, Mutex};

use cosmos_render::{LayerKind, Palette, RenderModel, Rgba};
use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{
    Billboard, Billboards, Camera3d, PostFx, PostFxConfig, SkyBackdrop, SkyParams,
};
use llimphi_ui::llimphi_hal::wgpu;

/// Formato de la textura intermedia de Llimphi (target de `gpu_paint_with`).
pub(crate) const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Oblicuidad de la eclíptica (J2000), para inclinar el ecuador celeste.
const OBLIQUITY_DEG: f32 = 23.439_29;

/// Geometría de un frame: la lista de puntos (billboards) ya armada en CPU.
pub(crate) struct SphereGeom {
    billboards: Vec<Billboard>,
}

/// Estado GPU persistente: post-proceso (SSAA + bloom) + puntos (billboards)
/// + campo estelar. Vive en el `Model` tras `Arc<Mutex<…>>`.
pub(crate) struct SphereGpu {
    fx: PostFx,
    bodies: Billboards,
    sky: SkyBackdrop,
}

/// Ranura compartida que guarda el `SphereGpu` entre frames.
pub(crate) type SphereGpuSlot = Arc<Mutex<Option<SphereGpu>>>;

/// Crea una ranura vacía para el `Model`.
pub(crate) fn slot() -> SphereGpuSlot {
    Arc::new(Mutex::new(None))
}

impl SphereGpu {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let mut bodies = Billboards::new(device, FMT);
        let (aw, ah, atlas) = dot_atlas();
        bodies.set_atlas(device, queue, aw, ah, &atlas);
        let mut sky = SkyBackdrop::new(device, FMT);
        let (sw, sh, field) = starfield_panorama();
        sky.set_texture(device, queue, sw, sh, &field);
        // Bloom MUY suave: sólo un halo en los cuerpos más brillantes. Umbral
        // alto para no encender el campo estelar (lo dejaría todo blobs).
        let fx = PostFx::with_config(
            device,
            FMT,
            PostFxConfig {
                supersample: 2,
                bloom_strength: 0.35,
                bloom_threshold: 0.80,
                bloom_knee: 0.30,
                bloom_radius: 1.8,
            },
        );
        Self { fx, bodies, sky }
    }

    /// Sube los puntos del frame y dibuja la esfera orbitada dentro de `rect`
    /// (px del target), con SSAA + bloom suave. `yaw`/`pitch` en grados.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        (w, h): (u32, u32),
        rect: (f32, f32, f32, f32),
        geom: &SphereGeom,
        yaw_deg: f32,
        pitch_deg: f32,
        dist: f32,
    ) {
        let (_rx, _ry, rw, rh) = rect;
        if w == 0 || h == 0 || rw < 1.0 || rh < 1.0 {
            return;
        }
        let aspect = rw / rh;
        let yaw = yaw_deg.to_radians();
        let pitch = pitch_deg.to_radians();
        let cam = Camera3d::orbit(Vec3::ZERO, yaw, pitch, dist);

        self.bodies.set_billboards(device, &geom.billboards);
        self.bodies.upload(queue, aspect, &cam);
        let fov_x = 2.0 * (aspect * (cam.fovy_rad * 0.5).tan()).atan();
        self.sky.upload(
            queue,
            &SkyParams {
                // La cámara orbita mirando al origen: azimut = yaw + π.
                yaw: yaw + std::f32::consts::PI,
                pitch: -pitch,
                fov_x,
                aspect,
                wraps: 1.0,
                v_scale: 1.0,
                pitch_scale: 0.6,
                v_offset: 0.0,
            },
        );

        // Escena → intermedio supersampleado (tamaño del panel) → blit al rect.
        let ow = rw.round().max(1.0) as u32;
        let oh = rh.round().max(1.0) as u32;
        self.fx.prepare(device, queue, (ow, oh));
        let clear = wgpu::Color { r: 5.0 / 255.0, g: 7.0 / 255.0, b: 16.0 / 255.0, a: 1.0 };
        {
            let mut pass = self.fx.scene_pass(encoder, clear);
            self.sky.draw(&mut pass); // fondo (no escribe depth)
            self.bodies.draw(&mut pass); // puntos (test+escribe depth)
        }
        self.fx.resolve_in(encoder, target, rect, (w, h));
    }
}

/// Punto sobre `slot`: lo crea si hace falta y dibuja. Para la closure de
/// `gpu_paint_with`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    slot: &SphereGpuSlot,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    viewport: (u32, u32),
    rect: (f32, f32, f32, f32),
    geom: &SphereGeom,
    yaw_deg: f32,
    pitch_deg: f32,
    dist: f32,
) {
    let mut guard = match slot.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let gpu = guard.get_or_insert_with(|| SphereGpu::new(device, queue));
    gpu.draw(device, queue, encoder, target, viewport, rect, geom, yaw_deg, pitch_deg, dist);
}

// =====================================================================
// Geometría (CPU) — todo puntos
// =====================================================================

fn rgb(c: Rgba) -> [f32; 3] {
    [c.r, c.g, c.b]
}

/// Punto unidad sobre la eclíptica a longitud `deg` (plano XZ; polo norte
/// eclíptico = +Y). Mismo origen angular que la rueda 2D.
fn eclip(deg: f32) -> Vec3 {
    let l = deg.to_radians();
    Vec3::new(l.cos(), 0.0, l.sin())
}

/// Un punto redondo de tamaño `size` (mundo) con `tint` (color) y alpha `a`.
fn dot(center: Vec3, size: f32, c: [f32; 3], a: f32) -> Billboard {
    Billboard {
        center: center.to_array(),
        size: [size, size],
        uv_min: [0.0, 0.0],
        uv_max: [1.0, 1.0],
        tint: [c[0], c[1], c[2], a],
    }
}

/// Empuja un aro como collar de `n` puntitos sobre la circunferencia
/// `transform(eclip(θ)) * radius`.
fn ring(
    out: &mut Vec<Billboard>,
    n: usize,
    radius: f32,
    size: f32,
    color: [f32; 3],
    alpha: f32,
    transform: impl Fn(Vec3) -> Vec3,
) {
    for k in 0..n {
        let p = transform(eclip(k as f32 / n as f32 * 360.0)) * radius;
        out.push(dot(p, size, color, alpha));
    }
}

/// Construye la esfera celeste como lista de puntos desde el modelo.
/// CPU-puro (no toca GPU).
pub(crate) fn sphere_geometry(model: &RenderModel, pal: &Palette) -> SphereGeom {
    let mut bb: Vec<Billboard> = Vec::new();
    let r = 1.0_f32;
    let eps = Mat4::from_rotation_x(OBLIQUITY_DEG.to_radians());

    // Eclíptica — collar de puntitos prominente (el aro del zodíaco).
    let ecl = rgb(pal.dial_ring);
    ring(&mut bb, 200, r, 0.020, ecl, 0.95, |p| p);
    // Ecuador celeste — inclinado por la oblicuidad, más tenue y fino.
    let equ = rgb(pal.uranus);
    ring(&mut bb, 160, r, 0.015, equ, 0.7, |p| eps.transform_point3(p));
    // Ticks del zodíaco (cada 30°), puntos algo mayores fuera del aro.
    for s in 0..12 {
        bb.push(dot(eclip(s as f32 * 30.0) * r * 1.05, 0.034, ecl, 0.95));
    }
    // Polos eclípticos (N color del aro, S apagado).
    bb.push(dot(Vec3::Y * r, 0.04, ecl, 0.9));
    bb.push(dot(-Vec3::Y * r, 0.04, rgb(pal.fg_muted), 0.8));
    // Ángulos ASC / MC / DSC / IC.
    let ang = rgb(pal.angle_highlight);
    for deg in [
        model.ascendant_deg,
        model.midheaven_deg,
        model.descendant_deg,
        model.imum_coeli_deg,
    ] {
        bb.push(dot(eclip(deg) * r * 1.10, 0.04, ang, 0.95));
    }

    // Cuerpos natales — discos grandes que brillan con su color.
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::Bodies) && layer.module_id == "natal" {
            for g in &layer.glyphs {
                bb.push(dot(eclip(g.deg) * r * 1.03, 0.13, rgb(pal.planet(&g.symbol)), 1.0));
            }
        }
    }
    // Cuerpos topocéntricos — discos chicos atenuados, levemente adentro.
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::Bodies) && layer.module_id == "topocentric" {
            for g in &layer.glyphs {
                bb.push(dot(eclip(g.deg) * r * 0.95, 0.075, rgb(pal.planet(&g.symbol)), 0.7));
            }
        }
    }
    // Estrellas fijas notables (capa del motor, si está activa).
    let star = rgb(pal.fg_text);
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::FixedStars) {
            for g in &layer.glyphs {
                bb.push(dot(eclip(g.deg) * r, 0.045, star, 0.95));
            }
        }
    }

    SphereGeom { billboards: bb }
}

// =====================================================================
// Texturas generadas en CPU
// =====================================================================

/// Atlas de "punto" radial suave (RGBA blanco, alpha = caída radial con un
/// núcleo lleno). El `tint` del billboard lo colorea; el alpha-discard del
/// shader recorta el círculo.
fn dot_atlas() -> (u32, u32, Vec<u8>) {
    const N: u32 = 48;
    let mut data = vec![0u8; (N * N * 4) as usize];
    let c = (N as f32 - 1.0) * 0.5;
    for y in 0..N {
        for x in 0..N {
            let dx = (x as f32 - c) / c;
            let dy = (y as f32 - c) / c;
            let r = (dx * dx + dy * dy).sqrt();
            let a = if r >= 1.0 {
                0.0
            } else if r < 0.5 {
                1.0
            } else {
                let t = (1.0 - r) / 0.5;
                (t * t).clamp(0.0, 1.0)
            };
            let idx = ((y * N + x) * 4) as usize;
            data[idx] = 255;
            data[idx + 1] = 255;
            data[idx + 2] = 255;
            data[idx + 3] = (a * 255.0) as u8;
        }
    }
    (N, N, data)
}

/// Panorama de campo estelar (RGBA): azul-noche con estrellas **tenues** y
/// densas — un telón de fondo que da profundidad sin robar protagonismo ni
/// dispararse con el bloom. Determinista (LCG sembrada) para reproducir.
fn starfield_panorama() -> (u32, u32, Vec<u8>) {
    const W: u32 = 2048;
    const H: u32 = 512;
    let mut data = vec![0u8; (W * H * 4) as usize];
    for px in data.chunks_exact_mut(4) {
        px[0] = 5;
        px[1] = 7;
        px[2] = 16;
        px[3] = 255;
    }
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    // Muchas estrellas tenues (luminancia < umbral de bloom → no se inflan).
    for _ in 0..4200 {
        let x = next() % W;
        let y = next() % H;
        // Brillo bajo-medio: 36..=132 (nunca llega al umbral de bloom 0.8).
        let b = 36 + (next() % 96) as u8;
        let pick = next() % 10;
        let (r, g, bl) = if pick < 7 {
            (b, b, b)
        } else if pick < 9 {
            (b.saturating_sub(20), b.saturating_sub(8), b)
        } else {
            (b, b.saturating_sub(10), b.saturating_sub(28))
        };
        let idx = ((y * W + x) * 4) as usize;
        data[idx] = data[idx].max(r);
        data[idx + 1] = data[idx + 1].max(g);
        data[idx + 2] = data[idx + 2].max(bl);
    }
    (W, H, data)
}
