//! Esfera celeste 3D sobre el motor GPU **`llimphi-3d`** (asimilado
//! 2026-06-24; billboards + campo estelar 2026-06-24).
//!
//! Reemplaza al wireframe vello proyectado a mano por geometría real con
//! depth buffer y cámara en perspectiva, orbitable. Tres capas en un único
//! pase (depth compartido, confinado al rect del panel):
//!
//! 1. **Campo estelar de fondo** ([`SkyBackdrop`]): un panorama cilíndrico
//!    de estrellas generado en CPU que panea con el azimut de la cámara
//!    (paralaje de estrellas infinitamente lejanas).
//! 2. **Estructura** ([`Renderer3d`], cubos vértice-coloreados): eclíptica,
//!    ecuador inclinado por la oblicuidad, ticks del zodíaco, polos y
//!    ángulos ASC/MC/DSC/IC.
//! 3. **Cuerpos luminosos** ([`Billboards`], sprites redondos siempre de
//!    cara a la cámara): planetas (natales + topocéntricos) y estrellas
//!    fijas, como puntos que brillan con su color — el look de un mapa
//!    estelar, no de cubos.
//!
//! El pipeline de mallas pinta el color de vértice tal cual sin luces
//! (ambiente = blanco) → emisivo, ideal para un cielo. El de billboards usa
//! alpha-discard contra un atlas de "punto" radial suave.
//!
//! Integración Llimphi: la geometría se arma en CPU en `view()` (sin GPU) y
//! se sube/dibuja dentro de `View::gpu_paint_with`. El [`SphereGpu`] (dueño
//! de los renderers y el depth) vive en el `Model` tras `Arc<Mutex<…>>` y se
//! crea perezosamente en el primer paint (ahí recién hay device).

use std::sync::{Arc, Mutex};

use cosmos_render::{LayerKind, Palette, RenderModel, Rgba};
use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{push_cube, Billboard, Billboards, Camera3d, Renderer3d, SkyBackdrop, SkyParams, Vertex3d};
use llimphi_ui::llimphi_hal::wgpu;

/// Formato de la textura intermedia de Llimphi (target de `gpu_paint_with`).
pub(crate) const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Depth de la escena 3D. Debe coincidir con el de `llimphi-3d` (su
/// `scene::DEPTH_FORMAT` es `pub(crate)`, así que lo espejamos acá).
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Oblicuidad de la eclíptica (J2000), para inclinar el ecuador celeste.
const OBLIQUITY_DEG: f32 = 23.439_29;

/// Geometría de un frame ya armada en CPU (sin GPU).
pub(crate) struct SphereGeom {
    verts: Vec<Vertex3d>,
    indices: Vec<u16>,
    billboards: Vec<Billboard>,
}

/// Depth attachment cacheado, recreado cuando cambia el tamaño del frame.
struct DepthBuffer {
    view: wgpu::TextureView,
    w: u32,
    h: u32,
}

/// Estado GPU persistente de la esfera: depth + estructura (malla) + cuerpos
/// (billboards) + campo estelar. Vive en el `Model` tras `Arc<Mutex<…>>`.
pub(crate) struct SphereGpu {
    depth: Option<DepthBuffer>,
    mesh: Renderer3d,
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
        Self {
            depth: None,
            mesh: Renderer3d::new(device, FMT),
            bodies,
            sky,
        }
    }

    fn ensure_depth(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if matches!(&self.depth, Some(d) if d.w == w && d.h == h) {
            return;
        }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cosmos-sphere-depth"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.depth = Some(DepthBuffer { view, w, h });
    }

    /// Sube la geometría del frame y dibuja la esfera orbitada dentro de
    /// `rect` (px del target). `yaw`/`pitch` en grados; `dist` = distancia
    /// de la cámara al centro.
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
        let (rx, ry, rw, rh) = rect;
        if w == 0 || h == 0 || rw < 1.0 || rh < 1.0 {
            return;
        }
        let aspect = rw / rh;
        let yaw = yaw_deg.to_radians();
        let pitch = pitch_deg.to_radians();
        let cam = Camera3d::orbit(Vec3::ZERO, yaw, pitch, dist);

        // Subir buffers/uniforms antes de abrir el pase.
        self.mesh.set_geometry(device, &geom.verts, &geom.indices);
        self.mesh.set_model(Mat4::IDENTITY);
        self.mesh.upload(queue, aspect, &cam);
        self.bodies.set_billboards(device, &geom.billboards);
        self.bodies.upload(queue, aspect, &cam);
        // El panorama panea con el azimut/cabeceo de la cámara orbital.
        let fov_x = 2.0 * (aspect * (cam.fovy_rad * 0.5).tan()).atan();
        self.sky.upload(
            queue,
            &SkyParams {
                // La cámara orbita: el azimut con el que mira el centro es
                // yaw + π (mira hacia el origen desde el offset).
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

        self.ensure_depth(device, w, h);
        let depth_view = &self.depth.as_ref().unwrap().view;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cosmos-sphere-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_viewport(rx, ry, rw, rh, 0.0, 1.0);
        let sx = rx.max(0.0);
        let sy = ry.max(0.0);
        let sw = (rw.min(w as f32 - sx)).max(0.0) as u32;
        let sh = (rh.min(h as f32 - sy)).max(0.0) as u32;
        if sw == 0 || sh == 0 {
            return;
        }
        pass.set_scissor_rect(sx as u32, sy as u32, sw, sh);

        // Fondo (sin escribir depth) → estructura (escribe depth) → cuerpos.
        self.sky.draw(&mut pass);
        self.mesh.draw(&mut pass);
        self.bodies.draw(&mut pass);
    }
}

/// Punto sobre `slot`: lo crea si hace falta y dibuja. Pensado para la
/// closure de `gpu_paint_with`.
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
// Geometría (CPU) — desde el RenderModel
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

/// Apila un cubo-cuenta centrado en `p` (mundo), lado `s`, color `c`.
fn bead(verts: &mut Vec<Vertex3d>, indices: &mut Vec<u16>, p: Vec3, s: f32, c: [f32; 3]) {
    let m = Mat4::from_translation(p) * Mat4::from_scale(Vec3::splat(s));
    push_cube(verts, indices, m, c);
}

/// Un billboard de cuerpo luminoso: punto redondo de tamaño `size` (mundo)
/// con `tint` (color + alpha) sobre todo el atlas.
fn dot(center: Vec3, size: f32, c: [f32; 3], a: f32) -> Billboard {
    Billboard {
        center: center.to_array(),
        size: [size, size],
        uv_min: [0.0, 0.0],
        uv_max: [1.0, 1.0],
        tint: [c[0], c[1], c[2], a],
    }
}

/// Construye la geometría de la esfera celeste desde el modelo. CPU-puro
/// (no toca GPU). La estructura va como cubos; los cuerpos luminosos como
/// billboards.
pub(crate) fn sphere_geometry(model: &RenderModel, pal: &Palette) -> SphereGeom {
    let mut v: Vec<Vertex3d> = Vec::new();
    let mut i: Vec<u16> = Vec::new();
    let mut bb: Vec<Billboard> = Vec::new();
    let r = 1.0_f32;
    let eps = Mat4::from_rotation_x(OBLIQUITY_DEG.to_radians());

    // Eclíptica — el aro prominente del zodíaco.
    let ecl = rgb(pal.dial_ring);
    for k in 0..168 {
        bead(&mut v, &mut i, eclip(k as f32 / 168.0 * 360.0) * r, 0.013, ecl);
    }
    // Ecuador celeste — inclinado por la oblicuidad.
    let equ = rgb(pal.uranus);
    for k in 0..132 {
        let p = eps.transform_point3(eclip(k as f32 / 132.0 * 360.0)) * r;
        bead(&mut v, &mut i, p, 0.009, equ);
    }
    // Ticks del zodíaco (cada 30°), un poco fuera del aro.
    for s in 0..12 {
        bead(&mut v, &mut i, eclip(s as f32 * 30.0) * r * 1.05, 0.024, ecl);
    }
    // Polos eclípticos (N color aro, S apagado).
    bead(&mut v, &mut i, Vec3::Y * r, 0.028, ecl);
    bead(&mut v, &mut i, -Vec3::Y * r, 0.028, rgb(pal.fg_muted));
    // Ángulos ASC / MC / DSC / IC.
    let ang = rgb(pal.angle_highlight);
    for deg in [
        model.ascendant_deg,
        model.midheaven_deg,
        model.descendant_deg,
        model.imum_coeli_deg,
    ] {
        bead(&mut v, &mut i, eclip(deg) * r * 1.09, 0.028, ang);
    }

    // Cuerpos natales — billboards grandes que brillan con su color.
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::Bodies) && layer.module_id == "natal" {
            for g in &layer.glyphs {
                bb.push(dot(eclip(g.deg) * r * 1.03, 0.14, rgb(pal.planet(&g.symbol)), 1.0));
            }
        }
    }
    // Cuerpos topocéntricos — billboards chicos atenuados, levemente adentro.
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::Bodies) && layer.module_id == "topocentric" {
            for g in &layer.glyphs {
                bb.push(dot(eclip(g.deg) * r * 0.95, 0.08, rgb(pal.planet(&g.symbol)), 0.7));
            }
        }
    }
    // Estrellas fijas notables (capa del motor, si está activa) — puntitos.
    let star = rgb(pal.fg_text);
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::FixedStars) {
            for g in &layer.glyphs {
                bb.push(dot(eclip(g.deg) * r, 0.05, star, 0.95));
            }
        }
    }

    SphereGeom { verts: v, indices: i, billboards: bb }
}

// =====================================================================
// Texturas generadas en CPU
// =====================================================================

/// Atlas de "punto" radial suave (RGBA blanco, alpha = caída radial con un
/// núcleo brillante). El `tint` del billboard lo colorea; el alpha-discard
/// del shader recorta el círculo.
fn dot_atlas() -> (u32, u32, Vec<u8>) {
    const N: u32 = 48;
    let mut data = vec![0u8; (N * N * 4) as usize];
    let c = (N as f32 - 1.0) * 0.5;
    for y in 0..N {
        for x in 0..N {
            let dx = (x as f32 - c) / c;
            let dy = (y as f32 - c) / c;
            let r = (dx * dx + dy * dy).sqrt();
            // Núcleo lleno hasta 0.45, caída suave al borde.
            let a = if r >= 1.0 {
                0.0
            } else if r < 0.45 {
                1.0
            } else {
                let t = (1.0 - r) / 0.55;
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

/// Panorama de campo estelar (RGBA, fondo azul-noche con estrellas
/// dispersas de brillo y tinte variados). Determinista (LCG sembrada) para
/// que el render sea reproducible.
fn starfield_panorama() -> (u32, u32, Vec<u8>) {
    const W: u32 = 1024;
    const H: u32 = 256;
    let mut data = vec![0u8; (W * H * 4) as usize];
    // Fondo: azul-noche muy oscuro.
    for px in data.chunks_exact_mut(4) {
        px[0] = 6;
        px[1] = 8;
        px[2] = 18;
        px[3] = 255;
    }
    // ~1500 estrellas vía LCG (sin depender de `rand`).
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    for _ in 0..1500 {
        let x = next() % W;
        let y = next() % H;
        let b = 120 + (next() % 136) as u8; // 120..=255
        // Tinte: la mayoría blancas, algunas azuladas/cálidas.
        let pick = next() % 10;
        let (r, g, bl) = if pick < 6 {
            (b, b, b)
        } else if pick < 8 {
            (b.saturating_sub(30), b.saturating_sub(10), b)
        } else {
            (b, b.saturating_sub(15), b.saturating_sub(40))
        };
        let idx = ((y * W + x) * 4) as usize;
        data[idx] = r;
        data[idx + 1] = g;
        data[idx + 2] = bl;
        // Las más brillantes ocupan también un vecino, para un destello.
        if b > 220 && x + 1 < W {
            let j = idx + 4;
            data[j] = data[j].max(r / 2);
            data[j + 1] = data[j + 1].max(g / 2);
            data[j + 2] = data[j + 2].max(bl / 2);
        }
    }
    (W, H, data)
}
