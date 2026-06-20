//! Fase 3.57 — **god rays volumétricos** desde luces puntuales.
//!
//! El pendiente más recurrente del renderer (deferido fase tras fase desde
//! 3.22). Cada [`WorldLight`] (sprite full-bright: antorcha, lámpara,
//! proyectil, muzzle) dispersa luz en el aire alrededor de su fuente — un
//! halo suave que el ojo lee como volumen y profundidad atmosférica, el
//! análogo luminoso de la niebla de los muros lejanos. Modelado como un
//! resplandor radial **aditivo** en screen-space: barato (un fill de
//! gradiente por luz), perceptual puro (no toca geometría, hitboxes ni
//! timing — espíritu de la capa de modernización opt-in).
//!
//! No es ray-marching real de scattering (caro, necesitaría un pase de
//! profundidad). Es el resplandor del aire alrededor del foco, que es la
//! parte que más se nota; las "varas" de luz a través de rendijas quedan
//! como defer (necesitan oclusión volumétrica).

use super::*;
use llimphi_ui::llimphi_raster::kurbo::Circle;
use llimphi_ui::llimphi_raster::peniko::{
    color::AlphaColor, BlendMode, Compose, Gradient, Mix,
};

/// Radio mundo del halo de resplandor a la altura de la fuente. Se
/// proyecta a pantalla por `focal/x_cam`, así una antorcha cercana irradia
/// un halo grande y una lejana uno chico (perspectiva correcta).
const GODRAY_GLOW_WORLD: f32 = 110.0;
/// Clamp del radio en pantalla — evita halos microscópicos (lejos) o que
/// inunden el frame (muy cerca).
const GODRAY_MIN_PX: f32 = 18.0;
const GODRAY_MAX_PX: f32 = 420.0;
/// Distancia (cam-space forward) a la que el resplandor cae a ~la mitad.
/// Más allá la luz aporta poco halo (ya está atenuada por distancia).
const GODRAY_HALF_DIST: f32 = 360.0;

/// Halo resuelto de una luz: posición en pantalla, radio en px y alpha del
/// núcleo. Puro y testeable — `draw_god_rays` sólo lo rasteriza.
pub(crate) struct GodrayHalo {
    pub center: Point,
    pub radius: f32,
    pub alpha: f32,
}

/// Resuelve el halo de una luz, o `None` si no contribuye (detrás del near
/// o aporte despreciable). `strength` = `cfg.god_rays` ya clampeado; `near`
/// = `cfg.near`. El radio escala con `focal/x_cam` (perspectiva) y el alpha
/// decae con la distancia forward (`GODRAY_HALF_DIST`).
pub(crate) fn godray_halo(
    l: &WorldLight,
    proj: &Projection,
    near: f32,
    strength: f32,
) -> Option<GodrayHalo> {
    // Detrás del near plane ⇒ sin halo (la fuente no está a la vista).
    if l.x_cam <= near.max(1.0) {
        return None;
    }
    let center = proj.project(l.x_cam, l.y_cam, l.z_cam);
    let radius = (GODRAY_GLOW_WORLD * proj.focal / l.x_cam).clamp(GODRAY_MIN_PX, GODRAY_MAX_PX);
    // Atenuación por distancia (forward): cerca brilla, lejos se apaga.
    let atten = 1.0 / (1.0 + (l.x_cam / GODRAY_HALF_DIST).powi(2));
    let alpha = (strength * atten).clamp(0.0, 1.0);
    if alpha < 0.01 {
        return None;
    }
    Some(GodrayHalo { center, radius, alpha })
}

/// Pinta el resplandor volumétrico de cada luz del mundo sobre la escena ya
/// rasterizada. Aditivo (`Compose::Plus`): los halos acumulan brillo y
/// nunca oscurecen — sumar dos antorchas da más luz, no una resta. Se llama
/// tras la geometría y antes del arma/overlays. `cfg.god_rays == 0` ⇒ no-op
/// (sin capa, sin costo).
pub(crate) fn draw_god_rays(
    scene: &mut Scene,
    rect: PaintRect,
    proj: &Projection,
    lights: &[WorldLight],
    cfg: &RenderConfig,
) {
    if cfg.god_rays <= 0.0 || lights.is_empty() || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let strength = cfg.god_rays.clamp(0.0, 1.5);
    let near = cfg.near;

    // Pre-resolvemos los halos visibles para no abrir la capa aditiva si
    // ninguna luz contribuye (todas detrás del near o fuera de pantalla).
    let mut halos: Vec<(Point, f32, (u8, u8, u8), f32)> = Vec::new();
    for l in lights {
        let Some(h) = godray_halo(l, proj, near, strength) else {
            continue;
        };
        // Cull: halo entero fuera del viewport.
        let r = h.radius as f64;
        if h.center.x + r < rect.x as f64
            || h.center.x - r > (rect.x + rect.w) as f64
            || h.center.y + r < rect.y as f64
            || h.center.y - r > (rect.y + rect.h) as f64
        {
            continue;
        }
        halos.push((h.center, h.radius, l.tint_rgb, h.alpha));
    }
    if halos.is_empty() {
        return;
    }

    // Una sola capa aditiva para todos los halos del frame.
    let full = Rect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        (rect.y + rect.h) as f64,
    );
    scene.push_layer(
        Fill::NonZero,
        BlendMode::new(Mix::Normal, Compose::Plus),
        1.0,
        Affine::IDENTITY,
        &full,
    );
    for (center, radius, tint, alpha) in halos {
        let to = |c: u8| c as f32 / 255.0;
        // Centro: tinte de la luz a `alpha`; medio y borde decaen a 0.
        // Curva con stop intermedio a 0.45 para un núcleo brillante y una
        // caída suave (no un disco plano).
        let core: Color = AlphaColor::new([to(tint.0), to(tint.1), to(tint.2), alpha]);
        let mid: Color = AlphaColor::new([to(tint.0), to(tint.1), to(tint.2), alpha * 0.35]);
        let edge: Color = AlphaColor::new([to(tint.0), to(tint.1), to(tint.2), 0.0]);
        let grad = Gradient::new_radial(center, radius)
            .with_stops([(0.0, core), (0.45, mid), (1.0, edge)].as_slice());
        let circle = Circle::new(center, radius as f64);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &grad, None, &circle);
    }
    scene.pop_layer();
}
