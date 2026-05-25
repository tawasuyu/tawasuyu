//! `supay-render-llimphi` — Fase 3.0 del proyecto supay.
//!
//! Renderer 3D que consume [`supay_scene::SceneSnapshot`] y lo pinta
//! como `View::paint_with` de Llimphi. El motor sigue corriendo a
//! 35 Hz (Fase 1) y produce snapshots (Fase 2); este renderer interpola
//! entre los últimos dos por cada frame del display y proyecta el mundo
//! con perspectiva CPU → polígonos vello que vello rasteriza en GPU.
//!
//! ## Por qué CPU + vello (no wgpu directo)
//!
//! La SDD apunta a wgpu nativo con mesh cache + TAA + RT shadows en
//! Fase 3. Hoy `llimphi-ui` sólo expone `View::paint_with(vello::Scene)`,
//! no `View::custom_pass(wgpu)`. Fase 3.0 (este crate) vive sobre vello
//! para validar la cadena `snapshot → renderer` end-to-end con el
//! surface existente. Fase 3.1+ agregará el custom_pass y migrará el
//! pipeline a wgpu directo — los tipos en `supay-scene` ya son los
//! correctos para esa transición; sólo cambia el back-end.
//!
//! ## Pipeline
//!
//! 1. **Interpolación.** Por frame, `alpha ∈ [0, 1]` se calcula de
//!    `(now - last_tick_at) / tick_period`. El renderer interpola
//!    `prev` y `next` con [`supay_scene::interpolate`].
//! 2. **Cámara.** Transformación 2D al espacio cámara: eje +X_cam =
//!    adelante (sentido `player.angle`), +Y_cam = derecha, +Z_cam =
//!    arriba (vertical mundial). `view_z = player.z + view_height`.
//! 3. **Back-face cull.** Convención Doom: el "front side" de un
//!    linedef es donde `(v2-v1) × (pt-v1) < 0` (z-comp). Si el jugador
//!    está en el back side, intercambiamos `front_sector`/`back_sector`
//!    para usar el sector del lado del jugador como `near`.
//! 4. **Slabs por linedef.** One-sided → slab `[front.floor,
//!    front.ceiling]`. Two-sided → lower (step) y upper (header) según
//!    diferencia de alturas entre near y far sector.
//! 5. **Near-clip 2D.** Linedef intersectado con el plano `X_cam = near`
//!    antes de proyectar; vértices detrás del near se sustituyen por la
//!    intersección (parametric `t = (near - x1) / (x2 - x1)`).
//! 6. **Proyección.** `screen = (cx + Y_cam·focal/X_cam,
//!    cy - Z_cam·focal/X_cam)` con `focal = h / (2·tan(fov_y/2))`.
//!    Píxeles cuadrados (mismo focal en x/y).
//! 7. **Sprites.** Billboards Y-up: rectángulo `2·half_w × height`
//!    centrado en `(x, y, sector.floor)` cara cámara — los lados
//!    izquierdo/derecho usan offset `±half_w` en Y_cam.
//! 8. **Sort.** Painter's algorithm por distancia euclidiana en cámara,
//!    bigger first. Sin BSP — funciona para Doom típico (rooms
//!    axis-aligned) y falla en casos raros de polígonos interpenetrantes
//!    (defer a Fase 3.1 cuando expongamos segs/subsectors).
//! 9. **Shading.** `shade = light_level/255 · fog_factor` con
//!    `fog_factor = max(0.2, 1 - depth/far_fog)`. Color por palette
//!    indexada por `front_sector`.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Point, Rect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::{PaintRect, View};
use supay_scene::{interpolate, SceneSnapshot, SectorSnap, SnapshotPair, SpriteSnap, WallSeg, NO_SECTOR};

// =====================================================================
// Config
// =====================================================================

/// Parámetros del renderer.
#[derive(Clone, Copy, Debug)]
pub struct RenderConfig {
    /// Field of view vertical en grados. Doom clásico ronda 60°; el
    /// default 75° da una sensación más moderna sin perder el feel.
    pub fov_y_deg: f32,
    /// Distancia near-clip en unidades Doom. Vértices con
    /// `X_cam < near` se descartan o se clipean. Más bajo = más cerca
    /// se puede ver el detalle, pero artefactos de precisión.
    pub near: f32,
    /// Distancia donde el fog alcanza la saturación máxima (shade 0.2).
    /// Más alto = menos fog, mundo más "abierto". Default 2048 ≈ 32
    /// celdas de 64 (la unidad típica de grid en Doom).
    pub far_fog: f32,
    /// Altura visual de los sprites en unidades Doom (todos los mobjs
    /// se renderizan con esta altura hasta que Fase 3.1 traiga lookup
    /// real al sprite WAD). 56 ≈ altura del Imp.
    pub sprite_height: f32,
    /// Mitad del ancho de los sprites — el billboard es
    /// `2·sprite_half_width × sprite_height`.
    pub sprite_half_width: f32,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            fov_y_deg: 75.0,
            near: 4.0,
            far_fog: 2048.0,
            sprite_height: 56.0,
            sprite_half_width: 16.0,
        }
    }
}

// =====================================================================
// API pública
// =====================================================================

/// Construye un `View<Msg>` que renderiza la escena en 3D.
///
/// El closure de paint se ejecuta por cada redraw; recomputa `alpha`
/// con `Instant::now()` en ese momento — el host **no** necesita
/// agendar redraws extra (cada `Msg::Tick` del motor reconstruye la
/// vista y eso dispara redraw).
///
/// Snapshots (`prev` y `next`) se clonan al construir el View (clone
/// es O(1) por `Arc<[T]>` en `SceneSnapshot`); el closure los mantiene
/// vivos por el lifetime del paint.
pub fn scene_view<Msg: Clone + Send + Sync + 'static>(
    pair: &SnapshotPair,
    last_tick_at: Instant,
    tick_period: Duration,
    config: RenderConfig,
) -> View<Msg> {
    let prev = pair.prev().cloned();
    let next = pair.next().cloned();
    let tick_period_secs = tick_period.as_secs_f32().max(1.0 / 1000.0);
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .clip(true)
    .paint_with(move |scene, _ts, rect: PaintRect| {
        let alpha = (last_tick_at.elapsed().as_secs_f32() / tick_period_secs).clamp(0.0, 1.0);
        let snap = make_frame(prev.as_ref(), next.as_ref(), alpha);
        render_frame(scene, rect, &snap, &config);
    })
}

fn make_frame(
    prev: Option<&SceneSnapshot>,
    next: Option<&SceneSnapshot>,
    alpha: f32,
) -> SceneSnapshot {
    match (prev, next) {
        (Some(p), Some(n)) => interpolate(p, n, alpha),
        (None, Some(n)) | (Some(n), None) => n.clone(),
        (None, None) => SceneSnapshot::empty(0),
    }
}

// =====================================================================
// Render por frame
// =====================================================================

fn render_frame(scene: &mut Scene, rect: PaintRect, snap: &SceneSnapshot, cfg: &RenderConfig) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    draw_sky_floor(scene, rect);

    let view_z = snap.player.z + snap.player.view_height;
    let cam = Camera::new(snap.player.x, snap.player.y, view_z, snap.player.angle);
    let proj = Projection::new(rect, cfg.fov_y_deg.to_radians());

    // Acumulamos polígonos en `renderables` con un depth por cada uno
    // para sort descendente (back-to-front).
    let mut renderables: Vec<Renderable> = Vec::with_capacity(snap.walls.len() * 2 + snap.sprites.len());
    for wall in snap.walls.iter() {
        gather_wall(&mut renderables, wall, snap, &cam, &proj, cfg);
    }
    for sprite in snap.sprites.iter() {
        gather_sprite(&mut renderables, sprite, snap, &cam, &proj, cfg);
    }
    renderables.sort_by(|a, b| {
        b.depth
            .partial_cmp(&a.depth)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for r in &renderables {
        scene.fill(Fill::NonZero, Affine::IDENTITY, r.color, None, &r.path);
    }
}

struct Renderable {
    /// Distancia euclidiana en espacio cámara del centroide del
    /// polígono. Bigger = más lejos = se pinta primero.
    depth: f32,
    color: Color,
    path: BezPath,
}

// =====================================================================
// Cámara + proyección
// =====================================================================

struct Camera {
    px: f32,
    py: f32,
    view_z: f32,
    cos_pa: f32,
    sin_pa: f32,
}

impl Camera {
    fn new(px: f32, py: f32, view_z: f32, angle: f32) -> Self {
        Self {
            px,
            py,
            view_z,
            cos_pa: angle.cos(),
            sin_pa: angle.sin(),
        }
    }

    /// World (x, y) → camera (X_cam = forward, Y_cam = right).
    ///
    /// Derivación: la cámara mira por `angle`, así que rotamos el
    /// mundo en `-angle` alrededor del eje Z para que la dirección de
    /// vista quede alineada con +X_cam. Convención: +Y_cam = derecha
    /// (mano derecha con +Z = arriba).
    fn to_cam_2d(&self, wx: f32, wy: f32) -> (f32, f32) {
        let dx = wx - self.px;
        let dy = wy - self.py;
        let x_cam = dx * self.cos_pa + dy * self.sin_pa;
        let y_cam = dx * self.sin_pa - dy * self.cos_pa;
        (x_cam, y_cam)
    }
}

struct Projection {
    cx: f32,
    cy: f32,
    /// `focal = h / (2·tan(fov_y/2))`. Pixels cuadrados ⇒ mismo focal
    /// en X e Y.
    focal: f32,
}

impl Projection {
    fn new(rect: PaintRect, fov_y_rad: f32) -> Self {
        let focal = rect.h * 0.5 / (fov_y_rad * 0.5).tan();
        Self {
            cx: rect.x + rect.w * 0.5,
            cy: rect.y + rect.h * 0.5,
            focal,
        }
    }

    /// `(X_cam, Y_cam, Z_cam)` → coordenada en pantalla. **Caller
    /// garantiza `x_cam > 0`** (post near-clip).
    fn project(&self, x_cam: f32, y_cam: f32, z_cam: f32) -> Point {
        let inv_d = 1.0 / x_cam;
        let sx = self.cx + y_cam * self.focal * inv_d;
        let sy = self.cy - z_cam * self.focal * inv_d;
        Point::new(sx as f64, sy as f64)
    }
}

// =====================================================================
// Walls
// =====================================================================

fn gather_wall(
    out: &mut Vec<Renderable>,
    wall: &WallSeg,
    snap: &SceneSnapshot,
    cam: &Camera,
    proj: &Projection,
    cfg: &RenderConfig,
) {
    // Determinamos en qué lado del linedef está el jugador.
    // Convención Doom (ver `P_PointOnLineSide` en chocolate-doom):
    // front = `(v2-v1) × (pt-v1)_z < 0`.
    let cross = (wall.x2 - wall.x1) * (cam.py - wall.y1)
        - (wall.y2 - wall.y1) * (cam.px - wall.x1);
    let on_front = cross < 0.0;

    let (near_idx, far_idx) = if on_front {
        (wall.front_sector, wall.back_sector)
    } else {
        (wall.back_sector, wall.front_sector)
    };

    if near_idx == NO_SECTOR {
        // El jugador está del lado del void — Doom no debería permitir
        // esto en un mapa bien formado. Defensiva: no dibujamos
        // (no hay sector desde el que mirar).
        return;
    }
    let Some(near_sec) = snap.sectors.get(near_idx as usize) else {
        return;
    };
    let far_sec = if far_idx != NO_SECTOR {
        snap.sectors.get(far_idx as usize)
    } else {
        None
    };

    // Camera-space del linedef en 2D plan view.
    let (mut x1, mut y1) = cam.to_cam_2d(wall.x1, wall.y1);
    let (mut x2, mut y2) = cam.to_cam_2d(wall.x2, wall.y2);

    // Near-clip: si los dos vértices están detrás, fuera. Si uno está
    // detrás, lo movemos al cruce con el plano `X_cam = near`.
    let near = cfg.near;
    if x1 < near && x2 < near {
        return;
    }
    if x1 < near {
        let t = (near - x1) / (x2 - x1);
        y1 += (y2 - y1) * t;
        x1 = near;
    } else if x2 < near {
        let t = (near - x2) / (x1 - x2);
        y2 += (y1 - y2) * t;
        x2 = near;
    }

    // Determinamos las slabs visibles.
    let near_floor = near_sec.floor_height;
    let near_ceiling = near_sec.ceiling_height;
    // Stack-allocated: máximo 2 slabs (lower + upper) o 1 (solid).
    let mut slabs: [(f32, f32, &SectorSnap); 2] = [
        (0.0, 0.0, near_sec),
        (0.0, 0.0, near_sec),
    ];
    let mut n_slabs = 0_usize;
    match far_sec {
        Some(far) => {
            // Lower (step up) — visible cuando el far_floor está más
            // alto que el near_floor.
            if far.floor_height > near_floor {
                slabs[n_slabs] = (near_floor, far.floor_height, near_sec);
                n_slabs += 1;
            }
            // Upper (header) — visible cuando el far_ceiling está más
            // bajo que el near_ceiling.
            if far.ceiling_height < near_ceiling {
                slabs[n_slabs] = (far.ceiling_height, near_ceiling, near_sec);
                n_slabs += 1;
            }
        }
        None => {
            slabs[0] = (near_floor, near_ceiling, near_sec);
            n_slabs = 1;
        }
    }

    if n_slabs == 0 {
        return;
    }

    // Depth para sort: distancia euclidiana del midpoint en cámara.
    let mid_x = (x1 + x2) * 0.5;
    let mid_y = (y1 + y2) * 0.5;
    let depth = (mid_x * mid_x + mid_y * mid_y).sqrt();

    for &(z_bot, z_top, sec) in &slabs[..n_slabs] {
        if z_top <= z_bot {
            continue;
        }
        let zb = z_bot - cam.view_z;
        let zt = z_top - cam.view_z;
        let bl = proj.project(x1, y1, zb);
        let tl = proj.project(x1, y1, zt);
        let tr = proj.project(x2, y2, zt);
        let br = proj.project(x2, y2, zb);
        let mut path = BezPath::new();
        path.move_to(bl);
        path.line_to(tl);
        path.line_to(tr);
        path.line_to(br);
        path.close_path();
        out.push(Renderable {
            depth,
            color: wall_color(wall, sec, depth, cfg),
            path,
        });
    }
}

// =====================================================================
// Sprites
// =====================================================================

fn gather_sprite(
    out: &mut Vec<Renderable>,
    sprite: &SpriteSnap,
    snap: &SceneSnapshot,
    cam: &Camera,
    proj: &Projection,
    cfg: &RenderConfig,
) {
    let (x_cam, y_cam) = cam.to_cam_2d(sprite.x, sprite.y);
    if x_cam < cfg.near {
        return;
    }
    let sec = snap.sectors.get(sprite.sector as usize);
    let floor = sec.map(|s| s.floor_height).unwrap_or(0.0);
    let z_bot = floor - cam.view_z;
    let z_top = z_bot + cfg.sprite_height;
    // Billboard cara cámara: lados izquierdo/derecho con offset en
    // Y_cam (=eje "derecha" en cámara). +Y_cam → derecha de la escena
    // → izquierda del billboard cuando proyectamos (porque
    // x_screen crece con +Y_cam).
    let hw = cfg.sprite_half_width;
    let bl = proj.project(x_cam, y_cam + hw, z_bot);
    let tl = proj.project(x_cam, y_cam + hw, z_top);
    let tr = proj.project(x_cam, y_cam - hw, z_top);
    let br = proj.project(x_cam, y_cam - hw, z_bot);
    let mut path = BezPath::new();
    path.move_to(bl);
    path.line_to(tl);
    path.line_to(tr);
    path.line_to(br);
    path.close_path();
    let depth = (x_cam * x_cam + y_cam * y_cam).sqrt();
    out.push(Renderable {
        depth,
        color: sprite_color(sprite, sec, depth, cfg),
        path,
    });
}

// =====================================================================
// Shading
// =====================================================================

/// Paleta indexada por `front_sector` para variedad sin texturas.
/// Fase 3.1 reemplaza por sampling de la WAD texture lump real.
const WALL_PALETTE: &[(u8, u8, u8)] = &[
    (0xB0, 0x88, 0x66),
    (0x88, 0x80, 0x70),
    (0x90, 0x70, 0x60),
    (0x60, 0x70, 0x80),
    (0xA0, 0x90, 0x70),
    (0x70, 0x68, 0x60),
];

fn wall_color(wall: &WallSeg, sec: &SectorSnap, depth: f32, cfg: &RenderConfig) -> Color {
    let (r, g, b) = WALL_PALETTE[(wall.front_sector as usize) % WALL_PALETTE.len()];
    let light = sec.light_level as f32 / 255.0;
    let fog = 1.0 - (depth / cfg.far_fog).clamp(0.0, 0.8);
    let shade = (light * fog).clamp(0.05, 1.0);
    Color::from_rgba8(
        ((r as f32) * shade) as u8,
        ((g as f32) * shade) as u8,
        ((b as f32) * shade) as u8,
        255,
    )
}

fn sprite_color(
    _sprite: &SpriteSnap,
    sec: Option<&SectorSnap>,
    depth: f32,
    cfg: &RenderConfig,
) -> Color {
    let light = sec.map(|s| s.light_level as f32 / 255.0).unwrap_or(0.8);
    let fog = 1.0 - (depth / cfg.far_fog).clamp(0.0, 0.8);
    let shade = (light * fog).clamp(0.05, 1.0);
    // Imp-red-ish hasta que Fase 3.1 traiga sprite real desde el WAD.
    Color::from_rgba8(
        (220.0 * shade) as u8,
        (90.0 * shade) as u8,
        (60.0 * shade) as u8,
        255,
    )
}

// =====================================================================
// Sky + floor backdrop
// =====================================================================

/// Banda superior de cielo + banda inferior de piso. Llenan el rect
/// completo antes de los polígonos 3D — los muros se pintan encima.
/// Sin gradiente todavía: dos colores planos para mantener el costo
/// bajo (vello rasteriza dos rects).
fn draw_sky_floor(scene: &mut Scene, rect: PaintRect) {
    let mid_y = rect.y as f64 + (rect.h as f64) * 0.5;
    let sky = Rect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        mid_y,
    );
    let floor = Rect::new(
        rect.x as f64,
        mid_y,
        (rect.x + rect.w) as f64,
        (rect.y + rect.h) as f64,
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgba8(14, 16, 24, 255),
        None,
        &sky,
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgba8(28, 26, 22, 255),
        None,
        &floor,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_identity_at_zero_angle() {
        let cam = Camera::new(0.0, 0.0, 0.0, 0.0);
        // Point in front (+X world) — should map to (+X_cam, 0).
        let (x, y) = cam.to_cam_2d(10.0, 0.0);
        assert!((x - 10.0).abs() < 1e-5);
        assert!(y.abs() < 1e-5);
    }

    #[test]
    fn camera_left_is_negative_y_cam() {
        // Player at origin, looking +X. Point to player's left in world
        // is at +Y world (right-hand rule with +Z up). En cámara,
        // derecha = +Y_cam, así que el punto izquierdo debe tener
        // Y_cam < 0.
        let cam = Camera::new(0.0, 0.0, 0.0, 0.0);
        let (_x, y) = cam.to_cam_2d(0.0, 10.0);
        assert!(y < 0.0, "left point should map to negative Y_cam, got {y}");
    }

    #[test]
    fn projection_centers_origin_at_screen_center() {
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj = Projection::new(rect, 75_f32.to_radians());
        // Punto directamente al frente — debe caer en el centro.
        let p = proj.project(100.0, 0.0, 0.0);
        assert!((p.x - 400.0).abs() < 1e-3);
        assert!((p.y - 300.0).abs() < 1e-3);
    }

    #[test]
    fn projection_right_of_camera_lands_right_of_center() {
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj = Projection::new(rect, 75_f32.to_radians());
        // Punto a 10 unidades al frente y 1 a la derecha — debe caer
        // a la derecha del centro horizontal.
        let p = proj.project(10.0, 1.0, 0.0);
        assert!(p.x > 400.0, "+Y_cam should project right of center, got {}", p.x);
    }
}
