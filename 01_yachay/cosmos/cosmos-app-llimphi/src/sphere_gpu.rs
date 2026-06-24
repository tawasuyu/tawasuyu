//! Esfera celeste 3D sobre el motor GPU **`llimphi-3d`** (asimilado
//! 2026-06-24). Reemplaza al wireframe vello proyectado a mano
//! (`cosmos-render::sphere3d`) por geometría real con depth buffer, cámara
//! en perspectiva y orbitado en GPU.
//!
//! La escena se arma como **cuentas** (cubos chicos vértice-coloreados, vía
//! `llimphi_3d::push_cube`): la eclíptica, el ecuador celeste inclinado por
//! la oblicuidad, los ticks del zodíaco, los polos, los ángulos (ASC/MC/…)
//! y los cuerpos natales/topocéntricos y estrellas fijas que trae el
//! `RenderModel`. Las cuentas se leen como un aro 3D desde cualquier ángulo
//! de cámara (a diferencia de una cinta plana, que desaparece de canto).
//!
//! El pipeline de mallas de `llimphi-3d` pinta el color de vértice tal cual
//! cuando no hay luces (ambiente = blanco) — justo lo que querés para un
//! mapa estelar que "emite": sin setup de iluminación.
//!
//! Integración con Llimphi: la geometría se arma en CPU en `view()` (sin
//! GPU) y se sube/dibuja dentro de `View::gpu_paint_with`, que corre con el
//! `wgpu::Device` del frame. El [`SphereGpu`] (que posee el `Scene3d` con su
//! depth y el `Renderer3d`) vive en el `Model` tras un `Arc<Mutex<…>>` y se
//! crea perezosamente en el primer paint (ahí recién hay device).

use std::sync::{Arc, Mutex};

use cosmos_render::{LayerKind, Palette, RenderModel, Rgba};
use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{push_cube, Camera3d, Renderer3d, Scene3d, Vertex3d};
use llimphi_ui::llimphi_hal::wgpu;

/// Formato de la textura intermedia de Llimphi (target de `gpu_paint_with`).
pub(crate) const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Oblicuidad de la eclíptica (J2000), para inclinar el ecuador celeste.
const OBLIQUITY_DEG: f32 = 23.439_29;

/// Estado GPU persistente de la esfera: el `Scene3d` (dueño del depth) y la
/// malla. Vive en el `Model` tras un `Arc<Mutex<Option<…>>>`.
pub(crate) struct SphereGpu {
    scene: Scene3d,
    mesh: Renderer3d,
}

/// Ranura compartida que guarda el `SphereGpu` entre frames (se crea en el
/// primer `gpu_paint_with`, cuando hay device).
pub(crate) type SphereGpuSlot = Arc<Mutex<Option<SphereGpu>>>;

/// Crea una ranura vacía para el `Model`.
pub(crate) fn slot() -> SphereGpuSlot {
    Arc::new(Mutex::new(None))
}

impl SphereGpu {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            scene: Scene3d::new(),
            mesh: Renderer3d::new(device, FMT),
        }
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
        viewport: (u32, u32),
        rect: (f32, f32, f32, f32),
        verts: &[Vertex3d],
        indices: &[u16],
        yaw_deg: f32,
        pitch_deg: f32,
        dist: f32,
    ) {
        if indices.is_empty() {
            return;
        }
        self.mesh.set_geometry(device, verts, indices);
        self.mesh.set_model(Mat4::IDENTITY);
        let cam = Camera3d::orbit(
            Vec3::ZERO,
            yaw_deg.to_radians(),
            pitch_deg.to_radians(),
            dist,
        );
        // Campos disjuntos: scene mutable + mesh inmutable a la vez.
        self.scene.render_in(
            device,
            queue,
            encoder,
            target,
            viewport,
            rect,
            &cam,
            None,
            &[&self.mesh],
        );
    }
}

/// Punto sobre `slot`: lo crea si hace falta y dibuja. Pensado para llamar
/// desde la closure de `gpu_paint_with`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    slot: &SphereGpuSlot,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    viewport: (u32, u32),
    rect: (f32, f32, f32, f32),
    verts: &[Vertex3d],
    indices: &[u16],
    yaw_deg: f32,
    pitch_deg: f32,
    dist: f32,
) {
    let mut guard = match slot.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let gpu = guard.get_or_insert_with(|| SphereGpu::new(device));
    gpu.draw(
        device, queue, encoder, target, viewport, rect, verts, indices, yaw_deg, pitch_deg, dist,
    );
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

/// Construye la geometría de cuentas de la esfera celeste desde el modelo.
/// Es CPU-puro (no toca GPU) — la `view()` la arma y la closure de
/// `gpu_paint_with` la sube.
pub(crate) fn sphere_geometry(model: &RenderModel, pal: &Palette) -> (Vec<Vertex3d>, Vec<u16>) {
    let mut v: Vec<Vertex3d> = Vec::new();
    let mut i: Vec<u16> = Vec::new();
    let r = 1.0_f32;
    let eps = Mat4::from_rotation_x(OBLIQUITY_DEG.to_radians());

    // Eclíptica — el aro prominente del zodíaco.
    let ecl = rgb(pal.dial_ring);
    for k in 0..168 {
        let deg = k as f32 / 168.0 * 360.0;
        bead(&mut v, &mut i, eclip(deg) * r, 0.015, ecl);
    }
    // Ecuador celeste — inclinado por la oblicuidad.
    let equ = rgb(pal.uranus);
    for k in 0..132 {
        let deg = k as f32 / 132.0 * 360.0;
        let p = eps.transform_point3(eclip(deg)) * r;
        bead(&mut v, &mut i, p, 0.010, equ);
    }
    // Ticks del zodíaco (cada 30°), un poco fuera del aro.
    for s in 0..12 {
        bead(&mut v, &mut i, eclip(s as f32 * 30.0) * r * 1.05, 0.026, ecl);
    }
    // Polos eclípticos (N dorado, S apagado).
    bead(&mut v, &mut i, Vec3::Y * r, 0.03, ecl);
    bead(&mut v, &mut i, -Vec3::Y * r, 0.03, rgb(pal.fg_muted));

    // Ángulos ASC / MC / DSC / IC.
    let ang = rgb(pal.angle_highlight);
    for deg in [
        model.ascendant_deg,
        model.midheaven_deg,
        model.descendant_deg,
        model.imum_coeli_deg,
    ] {
        bead(&mut v, &mut i, eclip(deg) * r * 1.09, 0.03, ang);
    }

    // Cuerpos natales (geocéntricos) — cuentas grandes color planeta.
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::Bodies) && layer.module_id == "natal" {
            for g in &layer.glyphs {
                bead(&mut v, &mut i, eclip(g.deg) * r, 0.05, rgb(pal.planet(&g.symbol)));
            }
        }
    }
    // Cuerpos topocéntricos — cuentas chicas, color atenuado, levemente
    // adentro del aro (la separación con su par natal = la paralaje).
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::Bodies) && layer.module_id == "topocentric" {
            for g in &layer.glyphs {
                let c = pal.planet(&g.symbol);
                bead(
                    &mut v,
                    &mut i,
                    eclip(g.deg) * r * 0.92,
                    0.03,
                    [c.r * 0.7, c.g * 0.7, c.b * 0.7],
                );
            }
        }
    }
    // Estrellas fijas notables (si la capa está activa).
    let star = rgb(pal.fg_muted);
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::FixedStars) {
            for g in &layer.glyphs {
                bead(&mut v, &mut i, eclip(g.deg) * r, 0.012, star);
            }
        }
    }

    (v, i)
}
