//! Fondo «rive» **vivo** del greeter: reproduce un proyecto de
//! `llimphi-anim-studio` (`Doc` + `RigDoc`) deformando su malla con la **misma
//! deriva idle** ([`RigDoc::idle_at`]) que usa el bake de `mirada-fondo`, así el
//! mismo `.ron` se ve igual en las tres superficies. El greeter sí tiene vello,
//! así que lo pinta en vivo (sin cache de frames).
//!
//! (Hoy una deriva idle procedural; el día que el studio serialice pistas de
//! animación por keyframe, se reproducirán esas — el resto del cableado no cambia.)
//!
//! Patrón **snapshot** como [`crate::bg_physics`]/[`crate::alleycat`]: el `view`
//! toma una foto barata (posiciones ya deformadas + `Arc` de la malla) y la mueve
//! al closure de pintura `'static`.

use std::sync::Arc;

use llimphi_anim::skel::Mesh;
use llimphi_anim_studio::rig::RigDoc;
use llimphi_anim_studio::Project;
use llimphi_mesh::{fit_transform, paint_solid, paint_textured};
use llimphi_ui::llimphi_raster::kurbo::{Point, Rect};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::PaintRect;

/// Duración del loop idle (segundos) — igual que el default del bake.
const LOOP_SECS: f32 = 6.0;

/// Fondo rive cargado: el rig base + su malla (compartida por `Arc`) + textura.
pub struct RiveBg {
    base: RigDoc,
    mesh: Arc<Mesh>,
    /// Bounds de reposo ya expandidos para que el vaivén no se corte.
    bounds: Rect,
    texture: Option<llimphi_image::Image>,
}

/// Foto de un frame: posiciones deformadas + lo necesario para pintarlas, todo
/// `'static` para moverlo al closure.
pub struct RiveSnapshot {
    mesh: Arc<Mesh>,
    positions: Vec<Point>,
    bounds: Rect,
    texture: Option<llimphi_image::Image>,
    accent: (u8, u8, u8),
}

impl RiveBg {
    /// Carga un proyecto `.ron`. `None` (con log) si no parsea o el rig/malla
    /// quedan vacíos — el greeter cae al fondo siguiente en precedencia.
    pub fn load(path: &str) -> Option<Self> {
        let project = Project::load(path)
            .map_err(|e| eprintln!("greeter · rive «{path}»: {e}"))
            .ok()?;
        let base = project.rig;
        if base.bones.is_empty() {
            eprintln!("greeter · rive «{path}»: el rig no tiene huesos");
            return None;
        }
        let mesh = base.mesh();
        if mesh.vertices.is_empty() {
            eprintln!("greeter · rive «{path}»: la malla quedó vacía");
            return None;
        }
        let texture = base.texture_path.as_deref().and_then(|p| {
            llimphi_image::load_path(std::path::Path::new(p), 64 * 1024 * 1024)
                .map_err(|e| eprintln!("greeter · rive textura «{p}»: {e:?}"))
                .ok()
        });
        let bounds = pad(llimphi_mesh::rest_bounds(&mesh), 0.35);
        Some(RiveBg { base, mesh: Arc::new(mesh), bounds, texture })
    }

    /// Foto del frame en el instante `t` (segundos), con `accent` para la malla
    /// sin textura.
    pub fn snapshot(&self, t: f32, accent: (u8, u8, u8)) -> RiveSnapshot {
        let phase = (t / LOOP_SECS).rem_euclid(1.0) as f64;
        let rig = self.base.idle_at(phase);
        let positions = self.mesh.deform(&rig.skeleton());
        RiveSnapshot {
            mesh: self.mesh.clone(),
            positions,
            bounds: self.bounds,
            texture: self.texture.clone(),
            accent,
        }
    }
}

/// Pinta una foto del fondo rive, encajando la malla deformada en `rect`.
pub fn paint_snapshot(snap: &RiveSnapshot, scene: &mut Scene, rect: PaintRect) {
    let xform = fit_transform(snap.bounds, rect);
    match &snap.texture {
        Some(tex) => paint_textured(scene, &snap.mesh, &snap.positions, xform, tex),
        None => {
            let (r, g, b) = snap.accent;
            paint_solid(
                scene,
                &snap.mesh,
                &snap.positions,
                xform,
                Color::from_rgba8(r, g, b, 255),
            )
        }
    }
}

/// Expande un `Rect` por una fracción de su tamaño en cada lado.
fn pad(r: Rect, frac: f64) -> Rect {
    let dx = r.width() * frac;
    let dy = r.height() * frac;
    Rect::new(r.x0 - dx, r.y0 - dy, r.x1 + dx, r.y1 + dy)
}
