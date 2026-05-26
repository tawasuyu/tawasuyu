//! `supay-render-llimphi` — Fase 3.3 del proyecto supay.
//!
//! Renderer 3D que consume [`supay_scene::SceneSnapshot`] y lo pinta
//! como `View::paint_with` de Llimphi. El motor sigue corriendo a
//! 35 Hz (Fase 1) y produce snapshots (Fase 2); este renderer interpola
//! entre los últimos dos por cada frame del display y proyecta el mundo
//! con perspectiva CPU → polígonos vello que vello rasteriza en GPU.
//!
//! ## Qué añade Fase 3.3 sobre 3.2
//!
//! - **Colores de piso/techo desde el WAD real**. Si `RenderConfig`
//!   trae un [`WadAtlas`] (cargado por el host con `supay-wad` desde
//!   `DOOM1.WAD`), `floor_color`/`ceiling_color` devuelven el promedio
//!   real del flat indexed por `sector.floor_pic`/`ceiling_pic` —
//!   resuelto vía `DoomEngine::flat_name(pic_idx)` → nombre del lump →
//!   `Wad::flat_average_color`. El cache vive en `WadAtlas` y se llena
//!   on-demand. Sin WAD (`atlas: None`), cae a las paletas hardcoded
//!   de 3.1 — el modo stub queda igual.
//!
//! ## Qué añade Fase 3.2 sobre 3.1
//!
//! - **Polígonos de subsector reales**. Si el snapshot trae
//!   `subsectors` y `segs` (motor real con BSP cargado), el renderer
//!   pinta el piso y el techo de cada subsector como polígono convexo
//!   proyectado con near-plane clipping Sutherland-Hodgman 2D. Esto
//!   reemplaza el "fake floor" de 3.1 que extendía cada pared a los
//!   bordes de pantalla — ahora los pisos/techos respetan la geometría
//!   real del nivel y las habitaciones se ven cerradas con la forma
//!   correcta.
//! - **Cielo detectado**. `ceiling_pic == sky_pic` (el motor expone
//!   `skyflatnum` en cada snapshot) → el subsector salta el techo
//!   sólido y deja ver el backdrop de cielo. Útil para áreas abiertas
//!   tipo E1M1 entrada exterior.
//! - **Fallback fake-floor 3.1**. Si el snapshot no trae subsectors
//!   (modo stub, mapa todavía no cargado) los walls vuelven a emitir
//!   trapezoides de piso/techo como antes — todavía se ve algo en
//!   lugar de horizonte plano.
//!
//! ## Qué añade 3.1 (todavía vigente)
//!
//! - Bandas horizontales por slab (`wall_bands=4` configurable) con
//!   shade modulado por `(linedef_idx, band_idx)` — feel de paneles
//!   sin samplear WAD.
//! - Paletas Doom-ish (`WALL_PALETTE`/`FLOOR_PALETTE`/`CEIL_PALETTE`/
//!   `SPRITE_PALETTE`) reverse-engineered del look de E1M1.
//! - Backdrop tinted con el color del sector más iluminado.
//!
//! ## Lo que NO está acá (defer a 3.3+)
//!
//! - Sampling de texturas WAD reales (lumps PNAMES/TEXTURE1/SIDEDEF).
//! - BSP front-to-back ordering correcto (3.2 sigue con painter's algo).
//! - Stencil/RT shadows, TAA, fog volumétrico real.
//! - Sprite real lookup por `sprite/frame` desde el WAD.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Point, Rect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::{PaintRect, View};
use supay_scene::{
    interpolate, NodeSnap, PlayerOverlays, SceneSnapshot, SectorSnap, SnapshotPair, SpriteSnap,
    SubsectorSnap, WallSeg, ML_DONTPEGBOTTOM, ML_DONTPEGTOP, NF_SUBSECTOR, NO_SECTOR, NO_SKY_PIC,
};

// =====================================================================
// Config
// =====================================================================

/// Atlas de assets resueltos desde el WAD para que el renderer no
/// tenga que hablar con `supay-wad` por frame. Construir con
/// [`WadAtlas::new`] una vez al inicio del host y compartir por `Arc`.
///
/// El cache de colores por nombre de flat es interno y lazy — la
/// primera vez que un flat se consulta calculamos su `flat_average_color`
/// y lo guardamos.
pub struct WadAtlas {
    wad: supay_wad::Wad,
    palette: [(u8, u8, u8); supay_wad::PALETTE_ENTRIES],
    /// Estado mutable interior — flat_names + color_cache bajo un
    /// único `Mutex` para que el host pueda registrar pic_idx nuevos
    /// (`set_flat_name`) sin tener que clonar/reconstruir el Arc
    /// compartido con el renderer.
    inner: Mutex<AtlasInner>,
}

#[derive(Default)]
struct AtlasInner {
    /// Lookup pic_idx (u16) → nombre del flat. Se llena on-demand
    /// vía `DoomEngine::flat_name(i)` la primera vez que el host ve
    /// un pic_idx en algún sector.
    flat_names: HashMap<u16, String>,
    /// Cache lazy: pic_idx → color promedio resuelto.
    color_cache: HashMap<u16, Option<(u8, u8, u8)>>,
    /// Lookup spritenum (u16) → 4-char base name del sprite (e.g.
    /// "TROO"). Llenado por el host con `DoomEngine::sprite_name(n)`
    /// la primera vez que el host ve un `SpriteSnap` con ese sprite.
    sprite_names: HashMap<u16, String>,
    /// Cache de patches decodificados por (spritenum, frame_letter,
    /// angle). `frame_letter` viene del bit 0..4 del `frame` del mobj
    /// (A..Z = 0..25); `angle` es 1..8 (Doom convention: 1=front,
    /// 5=back). Valor: `Option<(Arc<Patch>, mirror_flag)>` — mirror
    /// indica que el patch corresponde a un lump combinado tipo
    /// `TROOA2A8` y debe pintarse horizontalmente espejado.
    sprite_patches: HashMap<(u16, u8, u8), Option<(Arc<supay_wad::Patch>, bool)>>,
    /// Cache de texturas de pared compuestas por nombre. `None` para
    /// nombres que no resuelven en TEXTURE1.
    wall_textures: HashMap<String, Option<Arc<supay_wad::Texture>>>,
    /// Cache de flats expandidos a RGBA8 (64×64×4 = 16 KB) por pic_idx.
    flat_rgbas: HashMap<u16, Option<Arc<Vec<u8>>>>,
}

impl std::fmt::Debug for WadAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = self.inner.lock().map(|i| i.flat_names.len()).unwrap_or(0);
        f.debug_struct("WadAtlas")
            .field("lumps", &self.wad.len())
            .field("flat_names", &names)
            .finish()
    }
}

impl WadAtlas {
    /// Construye el atlas desde un WAD ya parseado. El mapa
    /// `pic_idx → flat_name` arranca vacío; el host lo va llenando
    /// con [`Self::set_flat_name`] conforme el motor expone los
    /// pic_idx del mapa cargado.
    pub fn new(wad: supay_wad::Wad, flat_names: HashMap<u16, String>) -> Self {
        let palette = wad.palette();
        Self {
            wad,
            palette,
            inner: Mutex::new(AtlasInner {
                flat_names,
                color_cache: HashMap::new(),
                sprite_names: HashMap::new(),
                sprite_patches: HashMap::new(),
                wall_textures: HashMap::new(),
                flat_rgbas: HashMap::new(),
            }),
        }
    }

    /// Recupera el color promedio para un `pic_idx`. Devuelve `None`
    /// si el nombre del flat no está mapeado o si el flat no existe
    /// en el WAD (e.g. el placeholder `F_SKY1` que no tiene bytes).
    pub fn flat_color(&self, pic_idx: u16) -> Option<(u8, u8, u8)> {
        let Ok(mut inner) = self.inner.lock() else {
            return None;
        };
        if let Some(&cached) = inner.color_cache.get(&pic_idx) {
            return cached;
        }
        let resolved = inner
            .flat_names
            .get(&pic_idx)
            .and_then(|n| self.wad.flat_average_color(n, &self.palette));
        inner.color_cache.insert(pic_idx, resolved);
        resolved
    }

    /// Registra (o sobreescribe) el nombre del flat para `pic_idx`.
    /// Invalida la entrada cacheada para ese índice. Toma `&self` —
    /// la interior mutability permite hacerlo desde un `Arc<Self>`
    /// compartido con el renderer.
    pub fn set_flat_name(&self, pic_idx: u16, name: String) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.flat_names.insert(pic_idx, name);
            inner.color_cache.remove(&pic_idx);
            inner.flat_rgbas.remove(&pic_idx);
        }
    }

    /// `true` si `pic_idx` ya fue registrado vía `set_flat_name`.
    pub fn has_flat_name(&self, pic_idx: u16) -> bool {
        self.inner
            .lock()
            .map(|i| i.flat_names.contains_key(&pic_idx))
            .unwrap_or(false)
    }

    /// Registra el 4-char name del sprite para un `spritenum`. Usado
    /// por el host análogo a [`Self::set_flat_name`]. Invalida los
    /// patches cacheados para ese spritenum (por si los frames
    /// dependían del nombre viejo).
    pub fn set_sprite_name(&self, spritenum: u16, name: String) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.sprite_names.insert(spritenum, name);
            inner.sprite_patches.retain(|(s, _, _), _| *s != spritenum);
        }
    }

    pub fn has_sprite_name(&self, spritenum: u16) -> bool {
        self.inner
            .lock()
            .map(|i| i.sprite_names.contains_key(&spritenum))
            .unwrap_or(false)
    }

    /// Recupera (decodificando si hace falta y cacheando) el patch
    /// RGBA para el sprite `spritenum` en `frame` (bits 0..4 = letter
    /// A..Z; bit 7 = full bright, ignorado por ahora) y `angle` (1..8).
    ///
    /// Devuelve `Some((patch, mirror))` o `None` si no se encuentra
    /// ningún lump razonable. `mirror=true` indica que el lump
    /// corresponde a un combinado tipo `TROOA2A8` y debe pintarse
    /// horizontalmente espejado.
    pub fn sprite_patch(
        &self,
        spritenum: u16,
        frame: u8,
        angle: u8,
    ) -> Option<(Arc<supay_wad::Patch>, bool)> {
        let letter = frame & 0x1F;
        let angle = angle.clamp(1, 8);
        let key = (spritenum, letter, angle);
        if let Ok(inner) = self.inner.lock() {
            if let Some(cached) = inner.sprite_patches.get(&key) {
                return cached.clone();
            }
        }
        let name = {
            let inner = self.inner.lock().ok()?;
            inner.sprite_names.get(&spritenum).cloned()?
        };
        let frame_char = (b'A' + letter) as char;
        // `sprite_lump` cubre los tres casos de naming + mirror.
        let resolved = self.wad.sprite_lump(&name, frame_char, angle);
        let decoded: Option<(Arc<supay_wad::Patch>, bool)> = resolved.and_then(|(lump_name, mirror)| {
            self.wad
                .patch_rgba(&lump_name, &self.palette)
                .map(|p| (Arc::new(p), mirror))
        });
        if let Ok(mut inner) = self.inner.lock() {
            inner.sprite_patches.insert(key, decoded.clone());
        }
        decoded
    }

    /// Recupera (decodificando + cacheando) el RGBA del flat 64×64
    /// para `pic_idx`. Devuelve `None` si el nombre del flat no está
    /// mapeado o no existe en el WAD (e.g. F_SKY1 placeholder).
    /// El renderer usa esto para texturizar pisos/techos.
    pub fn flat_rgba(&self, pic_idx: u16) -> Option<Arc<Vec<u8>>> {
        // Reusamos el color_cache para evitar duplicar lookups; lo
        // dejamos sin tocar porque el RGBA es ortogonal al color.
        // Cache propia para flats: el HashMap nuevo `flat_rgbas`.
        // De momento simplificamos: re-decodificamos por idx — son
        // 64×64=4 KB por flat resuelto, y `inner.flat_rgbas` cachea.
        if let Ok(inner) = self.inner.lock() {
            if let Some(cached) = inner.flat_rgbas.get(&pic_idx) {
                return cached.clone();
            }
        }
        let name = {
            let inner = self.inner.lock().ok()?;
            inner.flat_names.get(&pic_idx).cloned()
        }?;
        let decoded = self.wad.flat_rgba(&name, &self.palette).map(Arc::new);
        if let Ok(mut inner) = self.inner.lock() {
            inner.flat_rgbas.insert(pic_idx, decoded.clone());
        }
        decoded
    }

    /// Recupera (decodificando + cacheando) la textura de pared
    /// compuesta `name` (de TEXTURE1). Devuelve `None` si no existe
    /// o no parsea. Cache: `Some(Arc<Texture>)` o `None` para misses.
    pub fn wall_texture(&self, name: &str) -> Option<Arc<supay_wad::Texture>> {
        let key = name.to_ascii_uppercase();
        if let Ok(inner) = self.inner.lock() {
            if let Some(cached) = inner.wall_textures.get(&key) {
                return cached.clone();
            }
        }
        let decoded = self.wad.texture(&key, &self.palette).map(Arc::new);
        if let Ok(mut inner) = self.inner.lock() {
            inner.wall_textures.insert(key, decoded.clone());
        }
        decoded
    }

    /// Acceso al WAD interno (para features futuras como wall
    /// texturing samplear patches sin reabrir).
    pub fn wad(&self) -> &supay_wad::Wad {
        &self.wad
    }
}

/// Parámetros del renderer.
#[derive(Clone, Debug)]
pub struct RenderConfig {
    /// Field of view vertical en grados. Doom clásico ronda 60°; el
    /// default 75° da una sensación más moderna sin perder el feel.
    pub fov_y_deg: f32,
    /// Distancia near-clip en unidades Doom. Vértices con
    /// `X_cam < near` se descartan o se clipean.
    pub near: f32,
    /// Distancia donde el fog alcanza la saturación máxima.
    pub far_fog: f32,
    /// Altura visual de los sprites en unidades Doom.
    pub sprite_height: f32,
    /// Mitad del ancho de los sprites — billboard `2·hw × sprite_height`.
    pub sprite_half_width: f32,
    /// Cantidad de bandas horizontales por slab (subdivisión vertical).
    /// Más bandas = más detalle "panel/ladrillo" a costo de rects.
    pub wall_bands: u32,
    /// Cantidad de strips horizontales por slab texturizada. Cada
    /// strip resuelve su propia affine (image→screen) — el error de
    /// perspectiva queda reducido por factor `wall_strips`. 1 = sin
    /// subdivisión (3.6 behavior). 8 = compromiso razonable. Strips
    /// adicionales cuestan O(N) image fills.
    pub wall_strips: u32,
    /// Atlas WAD con paleta + colores de flats. Sin él, el renderer cae
    /// a las paletas hardcoded de 3.1.
    pub atlas: Option<Arc<WadAtlas>>,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            fov_y_deg: 75.0,
            near: 4.0,
            far_fog: 2048.0,
            sprite_height: 56.0,
            sprite_half_width: 16.0,
            wall_bands: 4,
            wall_strips: 8,
            atlas: None,
        }
    }
}

// =====================================================================
// API pública
// =====================================================================

pub fn scene_view<Msg: Clone + Send + Sync + 'static>(
    pair: &SnapshotPair,
    last_tick_at: Instant,
    tick_period: Duration,
    config: RenderConfig,
) -> View<Msg> {
    let prev = pair.prev().cloned();
    let next = pair.next().cloned();
    let tick_period_secs = tick_period.as_secs_f32().max(1.0 / 1000.0);
    let config = Arc::new(config);
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
    draw_backdrop(scene, rect, snap, cfg);

    let view_z = snap.player.z + snap.player.view_height;
    let cam = Camera::new(snap.player.x, snap.player.y, view_z, snap.player.angle);
    let proj = Projection::new(rect, cfg.fov_y_deg.to_radians());

    // Si el snapshot trae BSP (motor real con mapa cargado), pintamos
    // pisos/techos reales por subsector. Si no, los walls hacen
    // "fake-floor" como fallback de 3.1.
    let use_subsectors = !snap.subsectors.is_empty() && !snap.segs.is_empty();

    // Fase 3.13: si tenemos el árbol BSP, calculamos un orden
    // back-to-front desde la posición del jugador para asignar depth
    // de painter's correcto a los planos de subsector. Walls y sprites
    // siguen usando depth euclidiano (su orden relativo entre ellos no
    // depende del BSP y el ordenamiento por distancia funciona en Doom
    // para cualquier viewpoint plausible).
    //
    // `bsp_order_depths[ss_id]` = depth para los planos de ese subsector.
    // Grande = pintado primero. Vacío si no hay BSP — fallback al cálculo
    // euclidiano viejo dentro de gather_subsector_planes.
    let bsp_order_depths: Vec<Option<f32>> = if use_subsectors && !snap.nodes.is_empty() {
        compute_bsp_order_depths(snap)
    } else {
        Vec::new()
    };

    let cap = snap.walls.len() * (cfg.wall_bands as usize * 2 + 2)
        + snap.subsectors.len() * 2
        + snap.sprites.len();
    let mut renderables: Vec<Renderable> = Vec::with_capacity(cap);

    if use_subsectors {
        for (idx, sub) in snap.subsectors.iter().enumerate() {
            let bsp_depth = bsp_order_depths.get(idx).copied().flatten();
            gather_subsector_planes(
                &mut renderables,
                sub,
                snap,
                &cam,
                &proj,
                &rect,
                cfg,
                bsp_depth,
            );
        }
    }
    for (idx, wall) in snap.walls.iter().enumerate() {
        gather_wall(
            &mut renderables,
            wall,
            idx as u32,
            snap,
            &cam,
            &proj,
            &rect,
            cfg,
            use_subsectors,
        );
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
        match &r.kind {
            RenderKind::Fill => {
                scene.fill(Fill::NonZero, Affine::IDENTITY, r.color, None, &r.path);
            }
            RenderKind::Sprite { image, xform } => {
                scene.draw_image(image, *xform);
            }
            RenderKind::TexturedWall { image, brush_xform } => {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    image,
                    Some(*brush_xform),
                    &r.path,
                );
            }
        }
    }

    // Fase 3.14: overlay full-screen al final del frame (damage red,
    // pickup yellow, radsuit green, invuln white). Modernización pura
    // de la lógica de Doom de palette swapping a PLAYPAL[1..13].
    draw_player_overlays(scene, rect, &snap.player_overlays, snap.tick);
}

struct Renderable {
    depth: f32,
    color: Color,
    path: BezPath,
    kind: RenderKind,
}

enum RenderKind {
    /// Fill sólido del `path` con `color`. Walls fallback, floors,
    /// ceilings, sprites fallback.
    Fill,
    /// `scene.draw_image(image, xform)` — `path` y `color` se ignoran.
    /// Sprites texturizados desde el WAD.
    Sprite {
        image: llimphi_ui::llimphi_raster::peniko::Image,
        xform: Affine,
    },
    /// Pared texturizada: fill del `path` con la `image` (Extend::Repeat
    /// activado) usando `brush_xform` como brush_transform — vello
    /// rellena el polígono samplando el image tileado en world coords.
    /// `color` se ignora.
    TexturedWall {
        image: llimphi_ui::llimphi_raster::peniko::Image,
        brush_xform: Affine,
    },
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
    fn to_cam_2d(&self, wx: f32, wy: f32) -> (f32, f32) {
        let dx = wx - self.px;
        let dy = wy - self.py;
        let x_cam = dx * self.cos_pa + dy * self.sin_pa;
        let y_cam = dx * self.sin_pa - dy * self.cos_pa;
        (x_cam, y_cam)
    }

    /// Inverso de [`Self::to_cam_2d`]: camera (X, Y) → world (wx, wy).
    /// Útil para recuperar las coords mundo de vértices intermedios
    /// generados por el near-clip 2D (que ya están en cam space).
    fn from_cam_2d(&self, x_cam: f32, y_cam: f32) -> (f32, f32) {
        // Inversa de la rotación: rot⁻¹ = rotᵀ.
        // dx = x_cam·cos + y_cam·sin
        // dy = x_cam·sin - y_cam·cos
        let dx = x_cam * self.cos_pa + y_cam * self.sin_pa;
        let dy = x_cam * self.sin_pa - y_cam * self.cos_pa;
        (self.px + dx, self.py + dy)
    }
}

struct Projection {
    cx: f32,
    cy: f32,
    /// `focal = h / (2·tan(fov_y/2))`. Pixels cuadrados.
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

    /// `(X_cam, Y_cam, Z_cam)` → coordenada en pantalla.
    /// **Caller garantiza `x_cam > 0`** (post near-clip).
    fn project(&self, x_cam: f32, y_cam: f32, z_cam: f32) -> Point {
        let inv_d = 1.0 / x_cam;
        let sx = self.cx + y_cam * self.focal * inv_d;
        let sy = self.cy - z_cam * self.focal * inv_d;
        Point::new(sx as f64, sy as f64)
    }
}

// =====================================================================
// Walls + floor/ceiling strips
// =====================================================================

fn gather_wall(
    out: &mut Vec<Renderable>,
    wall: &WallSeg,
    wall_idx: u32,
    snap: &SceneSnapshot,
    cam: &Camera,
    proj: &Projection,
    rect: &PaintRect,
    cfg: &RenderConfig,
    skip_fake_floor: bool,
) {
    // Front/back side por convención Doom.
    let cross = (wall.x2 - wall.x1) * (cam.py - wall.y1)
        - (wall.y2 - wall.y1) * (cam.px - wall.x1);
    let on_front = cross < 0.0;

    let (near_idx, far_idx) = if on_front {
        (wall.front_sector, wall.back_sector)
    } else {
        (wall.back_sector, wall.front_sector)
    };

    if near_idx == NO_SECTOR {
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

    let (mut x1, mut y1) = cam.to_cam_2d(wall.x1, wall.y1);
    let (mut x2, mut y2) = cam.to_cam_2d(wall.x2, wall.y2);

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

    // Determinamos las slabs visibles + alturas para floor/ceiling strips.
    let near_floor = near_sec.floor_height;
    let near_ceiling = near_sec.ceiling_height;
    let mut slabs: [(f32, f32, &SectorSnap); 2] = [
        (0.0, 0.0, near_sec),
        (0.0, 0.0, near_sec),
    ];
    let mut n_slabs = 0_usize;
    let (floor_strip_z, ceiling_strip_z) = match far_sec {
        Some(far) => {
            // Lower (step up).
            if far.floor_height > near_floor {
                slabs[n_slabs] = (near_floor, far.floor_height, near_sec);
                n_slabs += 1;
            }
            // Upper (header).
            if far.ceiling_height < near_ceiling {
                slabs[n_slabs] = (far.ceiling_height, near_ceiling, near_sec);
                n_slabs += 1;
            }
            // Para floor/ceiling visibles del lado del jugador:
            // si el step sube, vemos el floor del near; si el step baja
            // (far más bajo) ya no hay slab pero el floor del far asoma.
            let visible_floor = near_floor.min(far.floor_height);
            let visible_ceil = near_ceiling.max(far.ceiling_height);
            (visible_floor, visible_ceil)
        }
        None => {
            slabs[0] = (near_floor, near_ceiling, near_sec);
            n_slabs = 1;
            (near_floor, near_ceiling)
        }
    };

    if n_slabs == 0 && far_sec.is_none() {
        return;
    }

    // Depth para sort: distancia euclidiana del midpoint en cámara.
    let mid_x = (x1 + x2) * 0.5;
    let mid_y = (y1 + y2) * 0.5;
    let depth = (mid_x * mid_x + mid_y * mid_y).sqrt();

    // -----------------------------------------------------------------
    // Floor & ceiling strips ("fake floor") — fallback de 3.1 cuando no
    // hay BSP. Si el snapshot trae subsectors, los pisos/techos los
    // dibuja `gather_subsector_planes` con polígonos reales y este
    // bloque se salta entero.
    // -----------------------------------------------------------------
    if !skip_fake_floor {
        let zf = floor_strip_z - cam.view_z;
        let zc = ceiling_strip_z - cam.view_z;
        let bl_floor = proj.project(x1, y1, zf);
        let br_floor = proj.project(x2, y2, zf);
        let bl_ceil = proj.project(x1, y1, zc);
        let br_ceil = proj.project(x2, y2, zc);

        let screen_top = rect.y as f64;
        let screen_bot = (rect.y + rect.h) as f64;

        if bl_floor.y < screen_bot || br_floor.y < screen_bot {
            let mut path = BezPath::new();
            path.move_to(Point::new(bl_floor.x, screen_bot));
            path.line_to(bl_floor);
            path.line_to(br_floor);
            path.line_to(Point::new(br_floor.x, screen_bot));
            path.close_path();
            out.push(Renderable {
                depth: depth + 0.5,
                color: floor_color(near_sec, depth, cfg),
                path,
                kind: RenderKind::Fill,
            });
        }

        if bl_ceil.y > screen_top || br_ceil.y > screen_top {
            let mut path = BezPath::new();
            path.move_to(Point::new(bl_ceil.x, screen_top));
            path.line_to(Point::new(br_ceil.x, screen_top));
            path.line_to(br_ceil);
            path.line_to(bl_ceil);
            path.close_path();
            out.push(Renderable {
                depth: depth + 0.5,
                color: ceiling_color(near_sec, depth, cfg, snap.sky_pic),
                path,
                kind: RenderKind::Fill,
            });
        }
    }

    // -----------------------------------------------------------------
    // Wall slabs: texturizadas si hay textura asignada + atlas; sino
    // fallback a bandas horizontales con shading procedural.
    // -----------------------------------------------------------------
    // Index del slab actual en `slabs`: i=0 puede ser lower o solid,
    // i=1 (si existe) es upper. `slab_kind_for(i, n_slabs, far_sec)`
    // resuelve cuál sidedef-kind aplica (0=mid, 1=upper, 2=lower).
    let bands = cfg.wall_bands.max(1);
    let wall_len = ((wall.x2 - wall.x1).powi(2) + (wall.y2 - wall.y1).powi(2)).sqrt().max(1e-3);
    for (slab_i, &(z_bot, z_top, sec)) in (&slabs[..n_slabs]).iter().enumerate() {
        if z_top <= z_bot {
            continue;
        }
        let zb = z_bot - cam.view_z;
        let zt = z_top - cam.view_z;
        let bl = proj.project(x1, y1, zb);
        let tl = proj.project(x1, y1, zt);
        let tr = proj.project(x2, y2, zt);
        let br = proj.project(x2, y2, zb);

        // ¿Hay textura asignada? Front side (0) o back side (1) según
        // qué lado del linedef ve el jugador. kind según slab_i.
        let side_idx = if on_front { 0usize } else { 1usize };
        let kind = wall_slab_kind(slab_i, n_slabs, far_sec.is_some());
        let tex_slot = wall.textures.get(side_idx * 3 + kind);
        let tex_name = tex_slot.and_then(|s| supay_scene::texture_name(s));
        let tex = tex_name.and_then(|n| cfg.atlas.as_ref().and_then(|a| a.wall_texture(n)));

        let mut path = BezPath::new();
        path.move_to(bl);
        path.line_to(tl);
        path.line_to(tr);
        path.line_to(br);
        path.close_path();

        if let Some(tex) = tex {
            // Per-strip rendering: subdividimos la pared en N strips a
            // lo largo del linedef. Cada strip se proyecta y resuelve
            // su propia affine — el error de perspectiva queda 1/N.
            use llimphi_ui::llimphi_raster::peniko::{Blob, Extend, Image, ImageFormat};
            let strips = cfg.wall_strips.max(1);
            let slab_h = (z_top - z_bot).max(1e-3);
            // Offsets de textura del sidedef + convención de pegging
            // de Doom (ML_DONTPEGTOP / ML_DONTPEGBOTTOM). v_top es la
            // coord V del image en el borde superior del slab — el
            // affine V de cada strip arranca ahí.
            let tex_x_offset = wall.tex_x_offsets[side_idx];
            let row_offset = wall.tex_y_offsets[side_idx];
            let far_floor = far_sec.map(|f| f.floor_height);
            let far_ceiling = far_sec.map(|f| f.ceiling_height);
            let v_top = wall_v_top(
                kind,
                wall.flags,
                near_floor,
                near_ceiling,
                far_floor,
                far_ceiling,
                z_top,
                tex.height as f32,
                row_offset,
            );
            let img = Image::new(
                Blob::from(tex.rgba.clone()),
                ImageFormat::Rgba8,
                tex.width as u32,
                tex.height as u32,
            )
            .with_extend(Extend::Repeat);
            // Para cada strip: lerp world a lo largo de v1→v2, proyectar
            // y emitir quad con su propio affine. Reuso el `img` clonado
            // por refcount (Blob).
            for s in 0..strips {
                let t0 = s as f32 / strips as f32;
                let t1 = (s + 1) as f32 / strips as f32;
                // World start/end del strip (después del near-clip,
                // que ya está reflejado en x1/y1/x2/y2 cam-space).
                // Trabajamos en cam space directamente: lerp entre los
                // dos extremos cam del slab.
                let cx0 = x1 + (x2 - x1) * t0;
                let cy0 = y1 + (y2 - y1) * t0;
                let cx1 = x1 + (x2 - x1) * t1;
                let cy1 = y1 + (y2 - y1) * t1;
                let zb_c = z_bot - cam.view_z;
                let zt_c = z_top - cam.view_z;
                let s_bl = proj.project(cx0, cy0, zb_c);
                let s_tl = proj.project(cx0, cy0, zt_c);
                let s_tr = proj.project(cx1, cy1, zt_c);
                let s_br = proj.project(cx1, cy1, zb_c);
                // U coord en image space del strip:
                //   [tex_x_offset + t0·wall_len, tex_x_offset + t1·wall_len].
                // V coord: [v_top, v_top + slab_h]. El affine mapea
                // image(u, v) → screen.
                let strip_w = wall_len * (t1 - t0);
                let strip_u_base = tex_x_offset + wall_len * t0;
                let step_ux = (s_tr.x - s_tl.x) / strip_w.max(1e-3) as f64;
                let step_uy = (s_tr.y - s_tl.y) / strip_w.max(1e-3) as f64;
                let step_vx = (s_bl.x - s_tl.x) / slab_h as f64;
                let step_vy = (s_bl.y - s_tl.y) / slab_h as f64;
                let xform = Affine::new([
                    step_ux,
                    step_uy,
                    step_vx,
                    step_vy,
                    s_tl.x - strip_u_base as f64 * step_ux - v_top as f64 * step_vx,
                    s_tl.y - strip_u_base as f64 * step_uy - v_top as f64 * step_vy,
                ]);
                let mut s_path = BezPath::new();
                s_path.move_to(s_bl);
                s_path.line_to(s_tl);
                s_path.line_to(s_tr);
                s_path.line_to(s_br);
                s_path.close_path();
                out.push(Renderable {
                    depth,
                    color: Color::WHITE,
                    path: s_path,
                    kind: RenderKind::TexturedWall {
                        image: img.clone(),
                        brush_xform: xform,
                    },
                });
            }
            // Overlay de shade: una sola fill sobre todo el slab —
            // no hace falta strip-per-strip porque shade es constante
            // sobre la slab al mismo depth.
            let shade = shade_for(sec.light_level, depth, cfg);
            if shade < 0.95 {
                let alpha = ((1.0 - shade) * 255.0) as u8;
                out.push(Renderable {
                    depth: depth - 0.001,
                    color: Color::from_rgba8(0, 0, 0, alpha),
                    path,
                    kind: RenderKind::Fill,
                });
            }
        } else {
            // Fallback: bandas horizontales coloreadas (3.1 behavior).
            for b in 0..bands {
                let t0 = b as f32 / bands as f32;
                let t1 = (b + 1) as f32 / bands as f32;
                let zb_b = (z_bot + (z_top - z_bot) * t0) - cam.view_z;
                let zt_b = (z_bot + (z_top - z_bot) * t1) - cam.view_z;
                let bl_b = proj.project(x1, y1, zb_b);
                let tl_b = proj.project(x1, y1, zt_b);
                let tr_b = proj.project(x2, y2, zt_b);
                let br_b = proj.project(x2, y2, zb_b);
                let mut p = BezPath::new();
                p.move_to(bl_b);
                p.line_to(tl_b);
                p.line_to(tr_b);
                p.line_to(br_b);
                p.close_path();
                out.push(Renderable {
                    depth,
                    color: wall_color(wall_idx, wall, sec, depth, b, bands, cfg),
                    path: p,
                    kind: RenderKind::Fill,
                });
            }
        }
    }
}

/// Resuelve el `kind` del sidedef (0=mid, 1=upper, 2=lower) para un
/// slab dado. Convención:
/// - Pared one-sided: hay un único slab → middle.
/// - Pared two-sided con n_slabs=1: el step expuesto → upper si
///   `far.ceiling < near.ceiling`, sino lower. (Reconstruimos del
///   orden en que `gather_wall` los emite — siempre lower primero.)
/// - Two-sided con n_slabs=2: slab_i=0 es lower, slab_i=1 es upper.
/// Coordenada V (image-space) en el borde superior del slab,
/// siguiendo la convención de pegging de Doom.
///
/// La regla general (ver `r_segs.c` de Chocolate Doom): la textura
/// queda anclada por un `v_anchor` que depende del `slab_kind` y los
/// flags `ML_DONTPEGTOP`/`ML_DONTPEGBOTTOM`. La V de un pixel a altura
/// world `z` es entonces `v(z) = v_anchor - z + rowoffset`. Acá
/// evaluamos eso en `z = z_top` — el resto del slab cae por debajo
/// con `v(z_bot) = v_top + slab_h` (1 image-pixel = 1 world-unit).
///
/// Casos:
/// - `kind=0` middle (one-sided): default → top de la textura en
///   `near_ceiling`. `DONTPEGBOTTOM` → bottom en `near_floor`.
/// - `kind=1` upper: default → top en `far_ceiling` (anclado al
///   bottom del opening); `DONTPEGTOP` → top en `near_ceiling`.
///   Esto hace que las puertas no muevan su textura al subir.
/// - `kind=2` lower: default → top en `far_floor` (el escalón);
///   `DONTPEGBOTTOM` → top en `near_ceiling` (para alinear con upper).
fn wall_v_top(
    slab_kind: usize,
    flags: u32,
    near_floor: f32,
    near_ceiling: f32,
    far_floor: Option<f32>,
    far_ceiling: Option<f32>,
    z_top: f32,
    tex_height: f32,
    row_offset: f32,
) -> f32 {
    let peg_top = (flags & ML_DONTPEGTOP) != 0;
    let peg_bot = (flags & ML_DONTPEGBOTTOM) != 0;
    let v_anchor = match slab_kind {
        0 => {
            if peg_bot {
                near_floor + tex_height
            } else {
                near_ceiling
            }
        }
        1 => {
            if peg_top {
                near_ceiling
            } else {
                far_ceiling.unwrap_or(near_ceiling) + tex_height
            }
        }
        2 => {
            if peg_bot {
                near_ceiling
            } else {
                far_floor.unwrap_or(near_floor)
            }
        }
        _ => near_ceiling,
    };
    (v_anchor - z_top) + row_offset
}

fn wall_slab_kind(slab_i: usize, n_slabs: usize, two_sided: bool) -> usize {
    if !two_sided {
        return 0; // middle
    }
    // En el path two-sided: gather_wall pushea lower primero (si visible)
    // y upper después. Sin n_slabs=1 sabemos cuál tipo. Aproximamos:
    if n_slabs == 2 {
        if slab_i == 0 { 2 } else { 1 }
    } else {
        // Un único slab two-sided: no podemos distinguir lower vs upper
        // sin más info. Default a upper (más común en mapas E1M1: techos
        // bajos sobre puertas).
        1
    }
}

// =====================================================================
// Subsector planes (floor + ceiling)
// =====================================================================

/// Pinta los polígonos de piso y techo de un subsector. El polígono se
/// construye encadenando los segs del subsector (`subsector.first_seg`,
/// `num_segs`): cada seg aporta `v1` y, el último, también su `v2`.
/// La cadena es CCW por convención BSP; cerramos directamente v2_final
/// → v1_inicial. Algunos lados pueden estar bordeados por particiones
/// BSP sin seg correspondiente y la cadena no representa el polígono
/// completo; el subsector vecino del mismo sector cubre el hueco.
/// Base sobre la que se acumula el orden BSP para los depths de planos.
/// Mucho más grande que cualquier depth euclidiano de pared o sprite
/// (los maps de Doom tienen ~3000 unidades de extensión máxima) para
/// garantizar que los planos siempre se pinten antes que walls y sprites.
const BSP_DEPTH_BASE: f32 = 1.0e6;

/// Devuelve, por cada subsector del snapshot, su depth de painter's
/// asignado por el orden back-to-front del árbol BSP — o `None` si el
/// subsector no fue alcanzado (no debería pasar en un BSP bien formado,
/// pero defendemos contra mapas con subtrees colgados).
///
/// El primer subsector visitado (más lejano) recibe el depth más grande;
/// el último visitado (donde está el jugador) recibe el depth más chico.
/// La painter's pinta de más-depth a menos-depth → orden BSP correcto.
fn compute_bsp_order_depths(snap: &SceneSnapshot) -> Vec<Option<f32>> {
    let n_subs = snap.subsectors.len();
    let mut depths: Vec<Option<f32>> = vec![None; n_subs];
    let mut traversal: Vec<u32> = Vec::with_capacity(n_subs);
    let root_child = (snap.nodes.len() - 1) as u16;
    walk_bsp(&snap.nodes, root_child, snap.player.x, snap.player.y, &mut traversal);
    let total = traversal.len();
    for (step, &ss) in traversal.iter().enumerate() {
        if let Some(slot) = depths.get_mut(ss as usize) {
            // step 0 = más lejano → depth alto; step total-1 = más cercano → depth bajo.
            *slot = Some(BSP_DEPTH_BASE + (total - step) as f32);
        }
    }
    depths
}

/// Camina el árbol BSP recursivamente desde `child`, agregando los
/// subsectores hoja a `out` en orden back-to-front respecto al viewer.
///
/// `child` codifica al estilo Doom: bit 15 set = subsector, else nodo
/// interno (ver [`NF_SUBSECTOR`]).
fn walk_bsp(nodes: &[NodeSnap], child: u16, view_x: f32, view_y: f32, out: &mut Vec<u32>) {
    if child & NF_SUBSECTOR != 0 {
        out.push((child & !NF_SUBSECTOR) as u32);
        return;
    }
    let Some(node) = nodes.get(child as usize) else {
        return;
    };
    // Convención R_PointOnSide: side = dx·(py - y) - dy·(px - x).
    // side > 0 → viewer en el lado front (children[0]); side < 0 → back.
    let side = node.partition_dx * (view_y - node.partition_y)
        - node.partition_dy * (view_x - node.partition_x);
    let (near_child, far_child) = if side > 0.0 {
        (node.children[0], node.children[1])
    } else {
        (node.children[1], node.children[0])
    };
    // Back-to-front: visitamos el subtree lejano primero.
    walk_bsp(nodes, far_child, view_x, view_y, out);
    walk_bsp(nodes, near_child, view_x, view_y, out);
}

fn gather_subsector_planes(
    out: &mut Vec<Renderable>,
    sub: &SubsectorSnap,
    snap: &SceneSnapshot,
    cam: &Camera,
    proj: &Projection,
    rect: &PaintRect,
    cfg: &RenderConfig,
    bsp_depth_override: Option<f32>,
) {
    if sub.num_segs < 2 {
        return;
    }
    let Some(sec) = snap.sectors.get(sub.sector as usize) else {
        return;
    };
    let first = sub.first_seg as usize;
    let count = sub.num_segs as usize;
    let end = first + count;
    if end > snap.segs.len() {
        return;
    }
    let seg_slice = &snap.segs[first..end];

    // Construir polígono mundial: v1 de cada seg + v2 del último.
    let mut world: Vec<(f32, f32)> = Vec::with_capacity(count + 1);
    for s in seg_slice {
        world.push((s.x1, s.y1));
    }
    // Cerrar con v2 del último seg sólo si difiere del primer v1
    // (algunos subsectores ya cierran naturalmente).
    let last_v2 = (seg_slice[count - 1].x2, seg_slice[count - 1].y2);
    let first_v1 = world[0];
    if (last_v2.0 - first_v1.0).abs() > 0.01 || (last_v2.1 - first_v1.1).abs() > 0.01 {
        world.push(last_v2);
    }

    // Transformar a cámara 2D.
    let cam_poly: Vec<(f32, f32)> = world
        .iter()
        .map(|&(x, y)| cam.to_cam_2d(x, y))
        .collect();

    // Clip contra near plane (X_cam >= near).
    let clipped = clip_near(&cam_poly, cfg.near);
    if clipped.len() < 3 {
        return;
    }

    // Necesitamos las world (x, y) en paralelo con el camera-space para
    // poder construir la affine image→screen del flat. El clip near
    // pudo introducir vértices intermedios sin world coords reales —
    // los recuperamos por inversa: world = cam.px + cos·x_cam - sin·y_cam
    //                              world_y = cam.py + sin·x_cam + cos·y_cam
    let cam_to_world = |cx: f32, cy: f32| -> (f32, f32) {
        (
            cam.px + cx * cam.cos_pa - cy * cam.sin_pa * -1.0 + cy * cam.sin_pa - cy * cam.sin_pa,
            cam.py,
        )
    };
    let _ = cam_to_world; // bypass — refactorizamos a un método del Camera.

    let world_xy: Vec<(f32, f32)> = clipped.iter().map(|&(cx, cy)| cam.from_cam_2d(cx, cy)).collect();

    // Centroide euclidiano del polígono en cámara — necesario para
    // calcular el shade (fog + light dropoff) que depende de la distancia
    // real al observador, no del BSP order.
    let shade_depth = {
        let (mut cx_sum, mut cy_sum) = (0.0_f32, 0.0_f32);
        for &(x, y) in &clipped {
            cx_sum += x;
            cy_sum += y;
        }
        let n = clipped.len() as f32;
        let cx = cx_sum / n;
        let cy = cy_sum / n;
        (cx * cx + cy * cy).sqrt()
    };
    // Depth para painter's sort:
    // - Con BSP (Fase 3.13), usamos el depth asignado por la travesía
    //   back-to-front del árbol — orden correcto Doom, elimina glitches
    //   del sort euclidiano cuando dos subsectores comparten centroide
    //   pero tienen prioridad distinta (escaleras, sectores interpenetrados).
    // - Sin BSP (stub, mapa no cargado), euclidiano como Fase 3.7+.
    let depth = bsp_depth_override.unwrap_or(shade_depth);

    let screen_x_min = rect.x as f64;
    let screen_x_max = (rect.x + rect.w) as f64;
    let screen_y_min = rect.y as f64;
    let screen_y_max = (rect.y + rect.h) as f64;

    // Proyecta todos los vértices a screen a una altura z dada y
    // devuelve `(path, screen_points)` — o `None` si está fuera de rect.
    let project_polygon = |z_world: f32| -> Option<(BezPath, Vec<Point>)> {
        let z_cam = z_world - cam.view_z;
        let pts: Vec<Point> = clipped
            .iter()
            .map(|&(x, y)| proj.project(x, y, z_cam))
            .collect();
        let all_left = pts.iter().all(|p| p.x < screen_x_min);
        let all_right = pts.iter().all(|p| p.x > screen_x_max);
        let all_above = pts.iter().all(|p| p.y < screen_y_min);
        let all_below = pts.iter().all(|p| p.y > screen_y_max);
        if all_left || all_right || all_above || all_below {
            return None;
        }
        let mut p = BezPath::new();
        p.move_to(pts[0]);
        for pt in &pts[1..] {
            p.line_to(*pt);
        }
        p.close_path();
        Some((p, pts))
    };

    // Helper común para emitir un plano (piso o techo) con o sin tex.
    let mut emit_plane = |z_world: f32, pic_idx: u16, is_floor: bool| {
        let Some((path, screen_pts)) = project_polygon(z_world) else {
            return;
        };
        // Intentar texturizar: tenemos atlas + flat resolves a RGBA.
        if let Some(atlas) = cfg.atlas.as_ref() {
            if let Some(rgba) = atlas.flat_rgba(pic_idx) {
                // Per-triangle fan: triangulamos el polígono convexo
                // del subsector desde el vértice 0 (fan(0, j, j+1)).
                // Cada triángulo individual es perspective-correct
                // porque sus 3 vértices determinan exactamente una
                // affine — sin aproximación de "spread-out 3 picks"
                // de 3.7. Subsectores son convexos por BSP definition,
                // y el clip near (Sutherland-Hodgman) preserva la
                // convexidad, así que el fan es válido. Triángulos
                // colineales o degenerados (post-clip) se saltan.
                let n_v = world_xy.len();
                if n_v >= 3 {
                    use llimphi_ui::llimphi_raster::peniko::{
                        Blob, Extend, Image, ImageFormat,
                    };
                    let img = Image::new(
                        Blob::from((*rgba).clone()),
                        ImageFormat::Rgba8,
                        supay_wad::FLAT_SIZE as u32,
                        supay_wad::FLAT_SIZE as u32,
                    )
                    .with_extend(Extend::Repeat);
                    let mut any_drawn = false;
                    for j in 1..n_v - 1 {
                        let (i0, i1, i2) = (0, j, j + 1);
                        if let Some(xform) = solve_floor_affine(
                            world_xy[i0],
                            screen_pts[i0],
                            world_xy[i1],
                            screen_pts[i1],
                            world_xy[i2],
                            screen_pts[i2],
                        ) {
                            let mut tri = BezPath::new();
                            tri.move_to(screen_pts[i0]);
                            tri.line_to(screen_pts[i1]);
                            tri.line_to(screen_pts[i2]);
                            tri.close_path();
                            out.push(Renderable {
                                depth: depth + 1.0,
                                color: Color::WHITE,
                                path: tri,
                                kind: RenderKind::TexturedWall {
                                    image: img.clone(),
                                    brush_xform: xform,
                                },
                            });
                            any_drawn = true;
                        }
                    }
                    if any_drawn {
                        // Shade overlay sobre el polígono entero
                        // (shade es constante por plano — no necesita
                        // ser per-triangle). Mismo truco que walls.
                        // Usa `shade_depth` euclidiano (no `depth` BSP-derived)
                        // porque fog/light dropoff dependen de la distancia
                        // real al jugador.
                        let shade = shade_for(sec.light_level, shade_depth, cfg)
                            * if is_floor { 0.92 } else { 0.85 };
                        if shade < 0.95 {
                            let alpha = ((1.0 - shade) * 255.0).clamp(0.0, 255.0) as u8;
                            out.push(Renderable {
                                depth: depth + 0.999,
                                color: Color::from_rgba8(0, 0, 0, alpha),
                                path,
                                kind: RenderKind::Fill,
                            });
                        }
                        return;
                    }
                }
            }
        }
        // Fallback al color promedio (3.3 behavior).
        let color = if is_floor {
            floor_color(sec, shade_depth, cfg)
        } else {
            ceiling_color(sec, shade_depth, cfg, snap.sky_pic)
        };
        out.push(Renderable {
            depth: depth + 1.0,
            color,
            path,
            kind: RenderKind::Fill,
        });
    };

    emit_plane(sec.floor_height, sec.floor_pic, true);

    let is_sky = snap.sky_pic != NO_SKY_PIC && sec.ceiling_pic == snap.sky_pic;
    if !is_sky {
        emit_plane(sec.ceiling_height, sec.ceiling_pic, false);
    }
}

/// Resuelve la affine `image (wx, wy) → screen (sx, sy)` a partir de 3
/// pares de correspondencias. Devuelve `None` si los 3 vértices están
/// near-colineales en world space (determinante ~0).
fn solve_floor_affine(
    w0: (f32, f32),
    s0: Point,
    w1: (f32, f32),
    s1: Point,
    w2: (f32, f32),
    s2: Point,
) -> Option<Affine> {
    let dw1x = (w1.0 - w0.0) as f64;
    let dw1y = (w1.1 - w0.1) as f64;
    let dw2x = (w2.0 - w0.0) as f64;
    let dw2y = (w2.1 - w0.1) as f64;
    let det_w = dw1x * dw2y - dw2x * dw1y;
    if det_w.abs() < 1e-3 {
        return None;
    }
    let ds1x = s1.x - s0.x;
    let ds1y = s1.y - s0.y;
    let ds2x = s2.x - s0.x;
    let ds2y = s2.y - s0.y;
    let a = (ds1x * dw2y - ds2x * dw1y) / det_w;
    let c = (dw1x * ds2x - ds1x * dw2x) / det_w;
    let e = s0.x - a * w0.0 as f64 - c * w0.1 as f64;
    let b = (ds1y * dw2y - ds2y * dw1y) / det_w;
    let d = (dw1x * ds2y - ds1y * dw2x) / det_w;
    let f = s0.y - b * w0.0 as f64 - d * w0.1 as f64;
    if !a.is_finite() || !b.is_finite() || !c.is_finite() || !d.is_finite() {
        return None;
    }
    Some(Affine::new([a, b, c, d, e, f]))
}

/// Sutherland-Hodgman para un único plano `X_cam >= near` en 2D
/// (paralelo al eje Y_cam). Vértices con `x < near` se descartan; las
/// aristas que cruzan el plano se intersectan parámetricamente.
fn clip_near(poly: &[(f32, f32)], near: f32) -> Vec<(f32, f32)> {
    if poly.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(poly.len() + 2);
    let n = poly.len();
    for i in 0..n {
        let curr = poly[i];
        let prev = poly[if i == 0 { n - 1 } else { i - 1 }];
        let curr_in = curr.0 >= near;
        let prev_in = prev.0 >= near;
        match (prev_in, curr_in) {
            (true, true) => out.push(curr),
            (true, false) => {
                let t = (near - prev.0) / (curr.0 - prev.0);
                let yi = prev.1 + (curr.1 - prev.1) * t;
                out.push((near, yi));
            }
            (false, true) => {
                let t = (near - prev.0) / (curr.0 - prev.0);
                let yi = prev.1 + (curr.1 - prev.1) * t;
                out.push((near, yi));
                out.push(curr);
            }
            (false, false) => {}
        }
    }
    out
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
    let depth = (x_cam * x_cam + y_cam * y_cam).sqrt();

    // ---- Camino texturizado: hay atlas + patch decodificado ----
    if let Some(atlas) = cfg.atlas.as_ref() {
        // Ángulo de display 1..8 según la convención Doom:
        // R_PointToAngle2(thing, viewer) − thing.angle, redondeado al
        // wedge de π/4 más cercano. 1 = facing camera, 5 = back,
        // 3 = right side, 7 = left.
        let display_angle = compute_display_angle(sprite.x, sprite.y, sprite.angle, cam.px, cam.py);
        if let Some((patch, mirror)) =
            atlas.sprite_patch(sprite.sprite, sprite.frame, display_angle)
        {
            let w = patch.width as f32;
            let h = patch.height as f32;
            let lo = patch.leftoffset as f32;
            let to = patch.topoffset as f32;
            let y_left = y_cam + lo;
            let y_right = y_cam + lo - w;
            let z_top = floor + to - cam.view_z;
            let z_bot = floor + to - h - cam.view_z;
            // Billboard axis-aligned → affine exacto.
            let tl = proj.project(x_cam, y_left, z_top);
            let br = proj.project(x_cam, y_right, z_bot);
            let sx = (br.x - tl.x) / w as f64;
            let sy = (br.y - tl.y) / h as f64;
            if !(sx.is_finite() && sy.is_finite() && sx > 0.01 && sy > 0.01) {
                return;
            }
            // Shading: tinte multiplicativo al RGBA cacheado, según
            // light_level del sector + fog distance. Construimos un
            // Image nuevo con la versión tinted — cada draw cuesta
            // un Vec::with_capacity + iter de width·height pixels;
            // para sprites típicos (≈2300 px) ronda 10 KB/draw,
            // ~30 sprites/frame ≈ 300 KB/frame, asumible a 60 fps.
            //
            // Full-bright (bit 7 = FF_FULLBRIGHT_BYTE): si el estado
            // del mobj tiene este flag (proyectiles, muzzle flashes,
            // frames de "fire" de imps/cacos), saltamos shade y fog —
            // el sprite se ve a luz plena como en Doom original.
            let full_bright = (sprite.frame & 0x80) != 0;
            let light = sec.map(|s| s.light_level).unwrap_or(192);
            let shade = if full_bright {
                1.0
            } else {
                shade_for(light, depth, cfg)
            };
            let img = make_tinted_sprite_image(&patch, shade);
            // Mirror = pintamos espejado: scale_x negativo + corrimiento.
            let xform = if mirror {
                Affine::translate((br.x, tl.y)) * Affine::scale_non_uniform(-sx, sy)
            } else {
                Affine::translate((tl.x, tl.y)) * Affine::scale_non_uniform(sx, sy)
            };
            out.push(Renderable {
                depth,
                color: Color::WHITE,
                path: BezPath::new(),
                kind: RenderKind::Sprite { image: img, xform },
            });
            return;
        }
    }

    // ---- Fallback 3.1: rectángulo coloreado ----
    let z_bot = floor - cam.view_z;
    let z_top = z_bot + cfg.sprite_height;
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
    out.push(Renderable {
        depth,
        color: sprite_color(sprite, sec, depth, cfg),
        path,
        kind: RenderKind::Fill,
    });
}

/// Crea un `peniko::Image` aplicando un shade multiplicativo (0..1) al
/// RGBA del patch. `shade=1.0` → idéntico; `shade<1.0` → tonos más
/// oscuros. La alpha del patch se preserva tal cual (importante: los
/// pixels transparentes siguen transparentes después del tint).
fn make_tinted_sprite_image(
    patch: &supay_wad::Patch,
    shade: f32,
) -> llimphi_ui::llimphi_raster::peniko::Image {
    use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
    let s = shade.clamp(0.05, 1.0);
    let tinted: Vec<u8> = if (s - 1.0).abs() < 1e-3 {
        // Fast path full-bright: clonamos sin transformar.
        patch.rgba.clone()
    } else {
        let mut out = Vec::with_capacity(patch.rgba.len());
        for chunk in patch.rgba.chunks_exact(4) {
            out.push(((chunk[0] as f32) * s) as u8);
            out.push(((chunk[1] as f32) * s) as u8);
            out.push(((chunk[2] as f32) * s) as u8);
            out.push(chunk[3]);
        }
        out
    };
    let blob = Blob::from(tinted);
    Image::new(blob, ImageFormat::Rgba8, patch.width as u32, patch.height as u32)
}

/// Calcula el ángulo de display 1..8 para un sprite direccional según
/// la convención Doom. `mobj_angle` = orientación facial del mobj en
/// world space (radianes desde +X, antihorario). `(viewer_x, viewer_y)`
/// = posición del jugador. Resultado: 1 si la cámara está en frente
/// del mobj, 3 a la derecha del mobj, 5 detrás, 7 a la izquierda.
fn compute_display_angle(
    mobj_x: f32,
    mobj_y: f32,
    mobj_angle: f32,
    viewer_x: f32,
    viewer_y: f32,
) -> u8 {
    use std::f32::consts::{FRAC_PI_4, TAU};
    let angle_to_viewer = (viewer_y - mobj_y).atan2(viewer_x - mobj_x);
    let rel = (angle_to_viewer - mobj_angle).rem_euclid(TAU);
    // Wedge de π/4. +π/8 = bias para que el wedge centre cada ángulo.
    let wedge = ((rel + FRAC_PI_4 / 2.0) / FRAC_PI_4).floor() as i32;
    let wedge = wedge.rem_euclid(8) as u8;
    wedge + 1
}

// =====================================================================
// Paletas — riffs sobre los flats/textures clásicos de Doom shareware
// (BROVINE/STARTAN/GRAYBIG/SLADWALL para paredes; FLAT5_5/MFLR8_1 para
// pisos; F_SKY1 para cielo). No son samples reales — son colores
// reverse-engineered del look visual de E1M1.
// =====================================================================

const WALL_PALETTE: &[(u8, u8, u8)] = &[
    (0xB0, 0x88, 0x66), // BROVINE — marrón cálido
    (0x88, 0x80, 0x70), // BLAKWAL — gris piedra
    (0x68, 0x58, 0x4C), // BROWN1  — marrón oscuro
    (0x8C, 0x74, 0x5C), // BROVINE alt
    (0x9C, 0x9C, 0x9C), // GRAYBIG — gris claro
    (0x6C, 0x6C, 0x6C), // GRAY1   — gris medio
    (0xA8, 0x84, 0x54), // STARTAN — tan UAC
    (0x74, 0x5C, 0x44), // BROWN2  — marrón quemado
    (0x84, 0x6C, 0x54), // marrón medio
    (0x5C, 0x4C, 0x40), // marrón profundo
    (0xB8, 0xA0, 0x80), // sand
    (0x4C, 0x54, 0x60), // slate
    (0x80, 0x70, 0x58), // tech tan
    (0x68, 0x64, 0x60), // dust gray
    (0x90, 0x80, 0x68), // cardboard
    (0xA0, 0x70, 0x4C), // rust
];

/// Pisos: marrones tierra, gris piedra, slime verde, marble azulado.
/// Indexed por `floor_pic % len`.
const FLOOR_PALETTE: &[(u8, u8, u8)] = &[
    (0x54, 0x44, 0x34), // FLAT5_5 — dirt
    (0x4C, 0x48, 0x44), // FLAT5_1 — stone
    (0x3C, 0x54, 0x38), // SLIME — slime green
    (0x38, 0x40, 0x50), // marble blue
    (0x5C, 0x50, 0x3C), // wood
    (0x44, 0x3C, 0x34), // tech dark
    (0x6C, 0x58, 0x40), // sand floor
    (0x40, 0x38, 0x2C), // ash
];

/// Techos: típicamente más oscuros + un blue-noche que reemplaza a F_SKY1.
const CEIL_PALETTE: &[(u8, u8, u8)] = &[
    (0x38, 0x34, 0x30), // CEIL3_1 — dark slate
    (0x44, 0x40, 0x38), // CEIL5_2 — light slate
    (0x2C, 0x28, 0x24), // RROCK04 — black rock
    (0x4C, 0x44, 0x38), // tech panel
];

/// "Cielo" en 3.2 se detecta comparando `sector.ceiling_pic` contra el
/// `sky_pic` del snapshot (el motor lo resuelve vía `skyflatnum` al
/// cargar el mapa). Cuando coincide, los pisos/techos por subsector
/// directamente NO emiten polígono y el backdrop se ve por ahí.
const SKY_BAND_TOP: Color = Color::from_rgba8(8, 10, 18, 255);
const SKY_BAND_BOT: Color = Color::from_rgba8(20, 22, 32, 255);

fn ceiling_is_sky(sec: &SectorSnap, sky_pic: u16) -> bool {
    sky_pic != NO_SKY_PIC && sec.ceiling_pic == sky_pic
}

// =====================================================================
// Shading
// =====================================================================

fn shade_for(light_level: u8, depth: f32, cfg: &RenderConfig) -> f32 {
    let light = light_level as f32 / 255.0;
    let fog = 1.0 - (depth / cfg.far_fog).clamp(0.0, 0.85);
    (light * fog).clamp(0.05, 1.0)
}

fn tint(rgb: (u8, u8, u8), shade: f32) -> Color {
    Color::from_rgba8(
        ((rgb.0 as f32) * shade) as u8,
        ((rgb.1 as f32) * shade) as u8,
        ((rgb.2 as f32) * shade) as u8,
        255,
    )
}

/// Hash determinístico ligero para variar tonos por linedef. xorshift
/// de 32 bits sembrado con el índice — la idea es que paredes adyacentes
/// no tengan exactamente el mismo color base.
fn wall_hash(wall_idx: u32) -> u32 {
    let mut x = wall_idx.wrapping_add(0x9E37_79B9);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

fn wall_color(
    wall_idx: u32,
    wall: &WallSeg,
    sec: &SectorSnap,
    depth: f32,
    band: u32,
    bands: u32,
    cfg: &RenderConfig,
) -> Color {
    // Base color por linedef hash + nudge por front_sector (para que
    // cada habitación tienda a una familia de tonos sin ser uniforme).
    let h = wall_hash(wall_idx).wrapping_add(wall.front_sector.wrapping_mul(7));
    let base = WALL_PALETTE[(h as usize) % WALL_PALETTE.len()];

    // Banda 0 = piso → más oscura. Banda top = techo → más clara. Curva
    // levemente positiva: simulación cheap de iluminación cenital.
    let band_t = if bands <= 1 {
        0.5
    } else {
        band as f32 / (bands - 1) as f32
    };
    // Factor en [0.78, 1.12] — bajo abajo, alto arriba, con sutil curva.
    let band_mul = 0.78 + 0.34 * band_t;

    // Variación pseudo-aleatoria por banda (ladrillo / panel feel).
    let band_jitter = {
        let hj = wall_hash(wall_idx ^ band.wrapping_mul(0x1234_5));
        let n = ((hj as f32) / (u32::MAX as f32)) * 2.0 - 1.0; // -1..1
        1.0 + n * 0.08 // ±8%
    };

    let base_shade = shade_for(sec.light_level, depth, cfg);
    let shade = (base_shade * band_mul * band_jitter).clamp(0.05, 1.0);
    tint(base, shade)
}

fn floor_color(sec: &SectorSnap, depth: f32, cfg: &RenderConfig) -> Color {
    let rgb = cfg
        .atlas
        .as_ref()
        .and_then(|a| a.flat_color(sec.floor_pic))
        .unwrap_or_else(|| FLOOR_PALETTE[(sec.floor_pic as usize) % FLOOR_PALETTE.len()]);
    let shade = shade_for(sec.light_level, depth, cfg) * 0.92;
    tint(rgb, shade.clamp(0.05, 1.0))
}

fn ceiling_color(sec: &SectorSnap, depth: f32, cfg: &RenderConfig, sky_pic: u16) -> Color {
    if ceiling_is_sky(sec, sky_pic) {
        return SKY_BAND_BOT;
    }
    let rgb = cfg
        .atlas
        .as_ref()
        .and_then(|a| a.flat_color(sec.ceiling_pic))
        .unwrap_or_else(|| CEIL_PALETTE[(sec.ceiling_pic as usize) % CEIL_PALETTE.len()]);
    let shade = shade_for(sec.light_level, depth, cfg) * 0.85;
    tint(rgb, shade.clamp(0.05, 1.0))
}

/// Paleta minimal por tipo de sprite. spritenum_t de Doom shareware
/// (subset): SPR_TROO=imp marrón, SPR_POSS=zombi verdoso, SPR_BAR1=barril,
/// SPR_BKEY/RKEY/YKEY=llaves, SPR_BFUG/SHOT/PLAS=armas, SPR_TLMP=lámpara.
/// Como Fase 3.1 no tiene tabla de spritenum_t expandida, usamos
/// `sprite_idx % len` directo — los colores quedan estables por tipo
/// pero no correspondem a la semántica real hasta Fase 3.2.
const SPRITE_PALETTE: &[(u8, u8, u8)] = &[
    (0xB4, 0x5C, 0x3C), // imp red-brown
    (0x6C, 0x84, 0x4C), // zombi verde
    (0x88, 0x70, 0x54), // barril marrón
    (0xC4, 0xA8, 0x4C), // amarillo (llave / munición)
    (0x5C, 0x80, 0xB4), // azul (llave azul / plasma)
    (0xB4, 0x44, 0x44), // rojo (llave roja / sangre)
    (0xD4, 0xC0, 0x88), // hueso / cráneo
    (0xE0, 0xA8, 0x4C), // antorcha cálida
    (0x9C, 0x9C, 0xA8), // armadura plateada
    (0x44, 0x6C, 0x44), // verde oscuro
    (0xC4, 0x80, 0x40), // naranja
    (0xA0, 0xA0, 0xB4), // gris claro
];

fn sprite_color(
    sprite: &SpriteSnap,
    sec: Option<&SectorSnap>,
    depth: f32,
    cfg: &RenderConfig,
) -> Color {
    let rgb = SPRITE_PALETTE[(sprite.sprite as usize) % SPRITE_PALETTE.len()];
    let full_bright = (sprite.frame & 0x80) != 0;
    let shade = if full_bright {
        1.0
    } else {
        let light = sec.map(|s| s.light_level).unwrap_or(192);
        shade_for(light, depth, cfg)
    };
    tint(rgb, shade)
}

// =====================================================================
// Backdrop (cuando paredes no cubren)
// =====================================================================

/// Pinta cielo arriba + tinte del piso del sector del jugador abajo.
/// El sector del jugador se infiere del primer sprite del jugador (el
/// snapshot no expone explícitamente sector del player en 3.1; el sprite
/// con índice 0 suele ser el avatar). Si no hay sectores, fallback gris.
fn draw_backdrop(scene: &mut Scene, rect: PaintRect, snap: &SceneSnapshot, cfg: &RenderConfig) {
    let mid_y = rect.y as f64 + (rect.h as f64) * 0.5;
    let sky_rect = Rect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        mid_y,
    );
    let floor_rect = Rect::new(
        rect.x as f64,
        mid_y,
        (rect.x + rect.w) as f64,
        (rect.y + rect.h) as f64,
    );

    // Sky con textura real si el atlas la tiene (SKY1 en E1, SKY2 en
    // E2, SKY3 en E3). Scrolling horizontal según player.angle —
    // convención Doom: 360° = 4 × sky_width = 1024 pixels en panorama.
    let sky_drawn = (|| -> bool {
        let Some(atlas) = cfg.atlas.as_ref() else {
            return false;
        };
        let Some(tex) = atlas.wall_texture("SKY1") else {
            return false;
        };
        use llimphi_ui::llimphi_raster::peniko::{Blob, Extend, Image, ImageFormat};
        let tex_w = tex.width as f64;
        let tex_h = tex.height as f64;
        let panorama_px = tex_w * 4.0; // 360° = 4 × tex.width
        let px_per_rad = panorama_px / std::f64::consts::TAU;
        // Scroll: player.angle aumenta antihorario; el sky debe
        // moverse en el sentido opuesto (cuando giro a la izquierda,
        // el sky parece moverse a la derecha en pantalla).
        let scroll_x = (-snap.player.angle as f64) * px_per_rad;
        // FOV horizontal aproximada (asumimos rect 4:3-ish, fov_y=75°).
        // pixels image por pixel pantalla en horizontal:
        // ancho de sky panorama visible = (fov_x_rad / 2π) × panorama_px
        // Aproximación: tomamos fov_x = fov_y · aspect_ratio.
        let aspect = rect.w as f64 / rect.h.max(1.0) as f64;
        let fov_x_rad = (cfg.fov_y_deg as f64).to_radians() * aspect;
        let pixels_to_show = fov_x_rad / std::f64::consts::TAU * panorama_px;
        let scale_x = pixels_to_show / rect.w as f64;
        let scale_y = tex_h / (rect.h as f64 * 0.5);
        // Affine: image(ix, iy) → screen((ix - scroll_x) / scale_x, iy / scale_y).
        // Vello forward affine a/b/c/d/e/f donde sx = a·ix + c·iy + e,
        // sy = b·ix + d·iy + f.
        let xform = Affine::new([
            1.0 / scale_x,
            0.0,
            0.0,
            1.0 / scale_y,
            -scroll_x / scale_x + rect.x as f64,
            rect.y as f64,
        ]);
        let img = Image::new(
            Blob::from(tex.rgba.clone()),
            ImageFormat::Rgba8,
            tex.width as u32,
            tex.height as u32,
        )
        .with_x_extend(Extend::Repeat)
        .with_y_extend(Extend::Pad);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &img, Some(xform), &sky_rect);
        true
    })();

    if !sky_drawn {
        scene.fill(Fill::NonZero, Affine::IDENTITY, SKY_BAND_TOP, None, &sky_rect);
    }
    let _ = SKY_BAND_BOT;

    // Floor backdrop: si tenemos al menos un sector, usá su paleta.
    // Como heurística pickeamos el sector con más light_level (la
    // habitación más iluminada — suele ser donde el jugador está
    // cuando arranca el nivel). No es exacto pero quita el "gris muerto"
    // de la 3.0 cuando mirás al vacío.
    let brightest = snap.sectors.iter().max_by_key(|s| s.light_level);
    let floor_rgb = brightest
        .and_then(|s| {
            cfg.atlas
                .as_ref()
                .and_then(|a| a.flat_color(s.floor_pic))
                .or_else(|| Some(FLOOR_PALETTE[(s.floor_pic as usize) % FLOOR_PALETTE.len()]))
        })
        .unwrap_or((0x32, 0x2E, 0x28));
    let backdrop_shade = 0.45;
    let bg = Color::from_rgba8(
        ((floor_rgb.0 as f32) * backdrop_shade) as u8,
        ((floor_rgb.1 as f32) * backdrop_shade) as u8,
        ((floor_rgb.2 as f32) * backdrop_shade) as u8,
        255,
    );
    scene.fill(Fill::NonZero, Affine::IDENTITY, bg, None, &floor_rect);
}

// =====================================================================
// Player overlays (Fase 3.14)
// =====================================================================
//
// Doom intercala PLAYPAL[1..13] cuando algo le pasa al jugador:
//   - [1..8]   = damage red flash (intensidad ∝ damagecount)
//   - [9..12]  = bonus yellow flash (intensidad ∝ bonuscount)
//   - [13]     = radiation suit green tint
//   - invuln   = inversión de colores via colormap (más caro de emular)
//
// Como sampleamos siempre con PLAYPAL[0] desde el renderer 3D, los
// overlays no aparecen "gratis" — los pintamos como rect full-screen
// semi-transparente al final del frame.

/// Pinta el overlay del jugador (damage/pickup/radsuit/invuln) sobre
/// todo el viewport. No-op si no hay overlays activos.
fn draw_player_overlays(scene: &mut Scene, rect: PaintRect, ov: &PlayerOverlays, tick: u64) {
    let Some((r, g, b, a)) = overlay_rgba(ov, tick) else {
        return;
    };
    let path = Rect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        (rect.y + rect.h) as f64,
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgba8(r, g, b, a),
        None,
        &path,
    );
}

/// Resuelve el overlay activo + su color RGBA. Prioridad Doom:
///   damage > bonus > radsuit. Invuln se superpone con tinte propio.
///
/// Acepta `tick` para el blink de los últimos 4 segundos (32 tics =
/// ~0.9 s a 35 Hz) de invuln/radsuit — bit 3 del tick controla on/off.
fn overlay_rgba(ov: &PlayerOverlays, tick: u64) -> Option<(u8, u8, u8, u8)> {
    use PlayerOverlays as O;
    let _ = std::mem::size_of::<O>();
    // Invulnerability: blink 32 tics finales, blanco brillante.
    let invuln_active = ov.power_invuln > 0
        && (ov.power_invuln > 4 * 32 || (tick & 0x8) != 0);
    if invuln_active {
        // Blanco semi-translúcido — aproximación cheap del invert colors
        // de Doom. Subir alpha hace que la escena "se desature".
        return Some((220, 220, 232, 110));
    }
    // Damage: red flash 8 niveles, alpha cada 8 pts de damagecount.
    if ov.damage_count > 0 {
        // Doom: (dc + 7) >> 3 → niveles 1..8. NUMREDPALS=8.
        let level = (((ov.damage_count + 7) >> 3).min(8)) as u8;
        // Alpha ramp 40..200 sobre los 8 niveles (más fuerte = más opaco).
        let alpha = 24 + level * 24;
        return Some((220, 30, 30, alpha));
    }
    // Bonus pickup: yellow flash 4 niveles.
    if ov.bonus_count > 0 {
        // Doom: (bc + 7) >> 3, NUMBONUSPALS=4.
        let level = (((ov.bonus_count + 7) >> 3).min(4)) as u8;
        let alpha = 24 + level * 18;
        return Some((215, 180, 70, alpha));
    }
    // Radsuit: green tint constante mientras el power > 4*32 (≈3.6 s),
    // luego blinkea con bit 3 del tick.
    if ov.power_radsuit > 0 {
        let active = ov.power_radsuit > 4 * 32 || (tick & 0x8) != 0;
        if active {
            return Some((45, 140, 60, 64));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_identity_at_zero_angle() {
        let cam = Camera::new(0.0, 0.0, 0.0, 0.0);
        let (x, y) = cam.to_cam_2d(10.0, 0.0);
        assert!((x - 10.0).abs() < 1e-5);
        assert!(y.abs() < 1e-5);
    }

    #[test]
    fn camera_left_is_negative_y_cam() {
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
        let p = proj.project(10.0, 1.0, 0.0);
        assert!(p.x > 400.0, "+Y_cam should project right of center, got {}", p.x);
    }

    #[test]
    fn wall_bands_vary_shade_monotonic_lighter_up() {
        // Misma pared, misma profundidad, distintas bandas — la banda
        // de arriba debe quedar más clara que la de abajo (multiplicador
        // 0.78..1.12 con t creciente).
        let sec = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 200,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let wall = WallSeg {
            x1: 0.0,
            y1: 0.0,
            x2: 64.0,
            y2: 0.0,
            front_sector: 0,
            back_sector: NO_SECTOR,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        let cfg = RenderConfig::default();
        let c_bot = wall_color(7, &wall, &sec, 100.0, 0, 4, &cfg);
        let c_top = wall_color(7, &wall, &sec, 100.0, 3, 4, &cfg);
        let comps = |c: Color| {
            let [r, g, b, _a] = c.to_rgba8().to_u8_array();
            r as u32 + g as u32 + b as u32
        };
        assert!(
            comps(c_top) > comps(c_bot),
            "top band ({:?}) should be lighter than bottom ({:?})",
            c_top.to_rgba8().to_u8_array(),
            c_bot.to_rgba8().to_u8_array()
        );
    }

    #[test]
    fn clip_near_keeps_polygon_fully_in_front() {
        // Cuadrado a X_cam = 100..200, Y ±50. Todo delante del near=4.
        let poly = vec![(100.0, -50.0), (200.0, -50.0), (200.0, 50.0), (100.0, 50.0)];
        let clipped = clip_near(&poly, 4.0);
        assert_eq!(clipped.len(), 4);
        assert_eq!(clipped, poly);
    }

    #[test]
    fn clip_near_drops_polygon_fully_behind() {
        // Cuadrado a X_cam = -100..-50. Todo detrás.
        let poly = vec![(-100.0, -50.0), (-50.0, -50.0), (-50.0, 50.0), (-100.0, 50.0)];
        let clipped = clip_near(&poly, 4.0);
        assert!(clipped.is_empty(), "behind-camera poly should be empty, got {clipped:?}");
    }

    #[test]
    fn clip_near_clips_polygon_crossing_plane() {
        // Triángulo con un vértice atrás (X=-10) y dos adelante (X=20).
        // Las dos aristas que cruzan deben generar intersecciones a X=near.
        let near = 4.0;
        let poly = vec![(-10.0, 0.0), (20.0, -10.0), (20.0, 10.0)];
        let clipped = clip_near(&poly, near);
        // Resultado esperado: 4 vértices — los 2 frontales + 2 intersecciones.
        assert_eq!(clipped.len(), 4, "expected 4 verts, got {clipped:?}");
        // Todas las X >= near.
        for &(x, _) in &clipped {
            assert!(x >= near - 1e-4, "vertex x={x} < near={near}");
        }
        // Las dos intersecciones deben estar en x = near.
        let on_plane = clipped.iter().filter(|&&(x, _)| (x - near).abs() < 1e-4).count();
        assert_eq!(on_plane, 2, "expected 2 vertices on plane, got {clipped:?}");
    }

    #[test]
    fn ceiling_sky_detection_matches_pic() {
        let sky_sec = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 256.0,
            light_level: 255,
            floor_pic: 0,
            ceiling_pic: 42,
        };
        assert!(ceiling_is_sky(&sky_sec, 42));
        assert!(!ceiling_is_sky(&sky_sec, 41));
        // Sentinel NO_SKY_PIC nunca debe matchear, aunque ceiling_pic
        // por casualidad sea 0xFFFF (mapa raro).
        let weird = SectorSnap {
            ceiling_pic: NO_SKY_PIC,
            ..sky_sec.clone()
        };
        assert!(!ceiling_is_sky(&weird, NO_SKY_PIC));
    }

    #[test]
    fn camera_to_from_round_trip() {
        let cam = Camera::new(100.0, 200.0, 41.0, 0.75);
        for (wx, wy) in [(150.0, 220.0), (50.0, 80.0), (100.0, 200.0), (-20.0, 999.0)] {
            let (cx, cy) = cam.to_cam_2d(wx, wy);
            let (rx, ry) = cam.from_cam_2d(cx, cy);
            assert!((rx - wx).abs() < 1e-3, "wx round-trip: {wx} → {rx}");
            assert!((ry - wy).abs() < 1e-3, "wy round-trip: {wy} → {ry}");
        }
    }

    #[test]
    fn solve_floor_affine_recovers_identity_when_world_equals_screen() {
        // Si world == screen para 3 puntos, la affine resuelta es la
        // identidad (a=1, b=0, c=0, d=1, e=0, f=0).
        let a = solve_floor_affine(
            (0.0, 0.0), Point::new(0.0, 0.0),
            (10.0, 0.0), Point::new(10.0, 0.0),
            (0.0, 10.0), Point::new(0.0, 10.0),
        ).expect("solve");
        let coeffs = a.as_coeffs();
        assert!((coeffs[0] - 1.0).abs() < 1e-6, "a={}", coeffs[0]);
        assert!(coeffs[1].abs() < 1e-6, "b={}", coeffs[1]);
        assert!(coeffs[2].abs() < 1e-6, "c={}", coeffs[2]);
        assert!((coeffs[3] - 1.0).abs() < 1e-6, "d={}", coeffs[3]);
    }

    #[test]
    fn solve_floor_affine_rejects_collinear() {
        // 3 vértices sobre una línea horizontal → det_w = 0 → None.
        let a = solve_floor_affine(
            (0.0, 0.0), Point::new(0.0, 0.0),
            (10.0, 0.0), Point::new(10.0, 0.0),
            (20.0, 0.0), Point::new(20.0, 0.0),
        );
        assert!(a.is_none());
    }

    #[test]
    fn display_angle_facing_camera_is_1() {
        // Mobj en (10, 0) facing -X (hacia el jugador en origen).
        // mobj_angle = π, viewer = (0,0). atan2(0-0, 0-10) = π.
        // rel = π - π = 0 → wedge 0 → display 1.
        let a = compute_display_angle(10.0, 0.0, std::f32::consts::PI, 0.0, 0.0);
        assert_eq!(a, 1, "expected front (1), got {a}");
    }

    #[test]
    fn display_angle_back_is_5() {
        // Mobj en (10, 0) facing +X (de espaldas al jugador en origen).
        // mobj_angle = 0, atan2(0-0, 0-10) = π. rel = π - 0 = π.
        // π / (π/4) = 4 → wedge 4 → display 5.
        let a = compute_display_angle(10.0, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(a, 5, "expected back (5), got {a}");
    }

    #[test]
    fn display_angle_right_side_is_3() {
        // Mobj en origen facing +X (su derecha = -Y world). Jugador
        // sobre el lado derecho del mobj → en -Y.
        // mobj_angle=0, viewer=(0,-10). atan2(-10-0, 0-0) = -π/2.
        // rel = (-π/2 - 0) mod 2π = 3π/2. 3π/2 / (π/4) = 6 → display 7.
        // (lado IZQUIERDO según convención Doom mirror; 3 sería al
        //  otro lado). Verificamos consistencia: si viewer está a +Y,
        //  debería ser 3.
        let a = compute_display_angle(0.0, 0.0, 0.0, 0.0, 10.0);
        // mobj_angle=0, viewer=(0,+10). atan2(+10, 0) = +π/2.
        // rel = π/2. π/2 / (π/4) = 2 → display 3.
        assert_eq!(a, 3, "expected right (3) for viewer on +Y of mobj facing +X, got {a}");
    }

    #[test]
    fn floor_color_uses_atlas_when_available() {
        // Sintetiza un WAD mínimo en memoria con un flat "F_T1" cuyo
        // promedio es conocido (todo índice 42 → palette[42]).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"IWAD");
        bytes.extend_from_slice(&2u32.to_le_bytes());
        let dir_off_placeholder = bytes.len();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        // PLAYPAL grayscale.
        let p1 = bytes.len();
        let playpal: Vec<u8> = (0..supay_wad::PALETTE_ENTRIES)
            .flat_map(|i| {
                let v = i as u8;
                [v, v, v]
            })
            .collect();
        bytes.extend_from_slice(&playpal);
        // F_T1 = todo 42.
        let p2 = bytes.len();
        bytes.extend(std::iter::repeat(42u8).take(supay_wad::FLAT_BYTES));
        let dir_off = bytes.len() as u32;
        bytes.extend_from_slice(&(p1 as u32).to_le_bytes());
        bytes.extend_from_slice(&(playpal.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"PLAYPAL\0");
        bytes.extend_from_slice(&(p2 as u32).to_le_bytes());
        bytes.extend_from_slice(&(supay_wad::FLAT_BYTES as u32).to_le_bytes());
        bytes.extend_from_slice(b"F_T1\0\0\0\0");
        bytes[dir_off_placeholder..dir_off_placeholder + 4]
            .copy_from_slice(&dir_off.to_le_bytes());

        let wad = supay_wad::Wad::parse(bytes).unwrap();
        let atlas = Arc::new(WadAtlas::new(wad, HashMap::new()));
        // Antes de registrar el nombre, flat_color devuelve None y el
        // floor_color cae a FLOOR_PALETTE.
        let sec = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 255,
            floor_pic: 7,
            ceiling_pic: 0,
        };
        let cfg_no_name = RenderConfig {
            atlas: Some(atlas.clone()),
            ..RenderConfig::default()
        };
        let c_fallback = floor_color(&sec, 0.0, &cfg_no_name);
        // Color del fallback: FLOOR_PALETTE[7 % 8] = ash (0x40,0x38,0x2C)
        // multiplicado por shade ≈ 0.92.
        let fb = c_fallback.to_rgba8().to_u8_array();
        assert!(fb[0] < 80, "fallback red should be muted, got {fb:?}");

        // Registrar nombre del flat → ahora flat_color devuelve (42,42,42).
        atlas.set_flat_name(7, "F_T1".to_string());
        let c_real = floor_color(&sec, 0.0, &cfg_no_name);
        let rc = c_real.to_rgba8().to_u8_array();
        // Expected: (42,42,42) tinted con light=255, depth=0 → shade≈0.92
        // → 42*0.92 ≈ 38 en cada canal.
        assert!((rc[0] as i32 - 38).abs() <= 2, "expected ≈38, got {rc:?}");
        assert_eq!(rc[0], rc[1]);
        assert_eq!(rc[1], rc[2]);
    }

    #[test]
    fn wall_v_top_middle_default_pegs_top_to_ceiling() {
        // Middle, no flags: la textura ancla su TOP al techo del near.
        // En z_top (= ceiling), V = 0.
        let v = wall_v_top(0, 0, 0.0, 128.0, None, None, 128.0, 64.0, 0.0);
        assert!(v.abs() < 1e-4, "expected v_top=0, got {v}");
    }

    #[test]
    fn wall_v_top_middle_dontpegbottom_pegs_bottom_to_floor() {
        // Middle + DONTPEGBOTTOM: bottom de la textura en near_floor.
        // En z_top (= ceiling=128), V = floor + tex_h - z_top = -64
        // (lo cual con Extend::Repeat tilea correctamente).
        let v = wall_v_top(0, ML_DONTPEGBOTTOM, 0.0, 128.0, None, None, 128.0, 64.0, 0.0);
        assert!((v - (-64.0)).abs() < 1e-4, "expected -64, got {v}");
    }

    #[test]
    fn wall_v_top_upper_default_pegs_to_back_ceiling() {
        // Upper sin flag: top de la textura al far_ceiling. La pared
        // "header" va de far_ceiling (= 96) a near_ceiling (= 128).
        // V(z_top = 128) = far_ceiling + tex_h - z_top = 96 + 64 - 128 = 32.
        let v = wall_v_top(1, 0, 0.0, 128.0, Some(0.0), Some(96.0), 128.0, 64.0, 0.0);
        assert!((v - 32.0).abs() < 1e-4, "expected 32, got {v}");
    }

    #[test]
    fn wall_v_top_upper_dontpegtop_pegs_to_front_ceiling() {
        // Upper + DONTPEGTOP: top alineado al near_ceiling — doors.
        // V(z_top = 128) = near_ceiling - z_top = 0.
        let v = wall_v_top(1, ML_DONTPEGTOP, 0.0, 128.0, Some(0.0), Some(96.0), 128.0, 64.0, 0.0);
        assert!(v.abs() < 1e-4, "expected 0, got {v}");
    }

    #[test]
    fn wall_v_top_lower_default_pegs_to_back_floor() {
        // Lower sin flag: top de la textura al far_floor. La pared
        // "step" va de near_floor (= 0) a far_floor (= 32).
        // V(z_top = 32) = far_floor - z_top = 0.
        let v = wall_v_top(2, 0, 0.0, 128.0, Some(32.0), Some(128.0), 32.0, 64.0, 0.0);
        assert!(v.abs() < 1e-4, "expected 0, got {v}");
    }

    #[test]
    fn wall_v_top_lower_dontpegbottom_pegs_to_near_ceiling() {
        // Lower + DONTPEGBOTTOM: top alineado al near_ceiling (= 128)
        // — alinea con la textura "main" del techo.
        // V(z_top = 32) = near_ceiling - z_top = 96.
        let v = wall_v_top(2, ML_DONTPEGBOTTOM, 0.0, 128.0, Some(32.0), Some(128.0), 32.0, 64.0, 0.0);
        assert!((v - 96.0).abs() < 1e-4, "expected 96, got {v}");
    }

    #[test]
    fn sprite_color_full_bright_bypasses_shading() {
        // Sin full-bright el sprite oscurece con light_level bajo + fog.
        // Con bit 7 set, sale a luz plena (shade=1.0).
        let sec = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 80, // oscuro
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let cfg = RenderConfig::default();
        let dim_sprite = SpriteSnap {
            x: 0.0, y: 0.0, z: 0.0, angle: 0.0,
            sprite: 0, frame: 0, sector: 0,
        };
        let bright_sprite = SpriteSnap {
            frame: 0x80, // bit 7 set
            ..dim_sprite.clone()
        };
        // depth=500 → fog atenúa visible
        let dim = sprite_color(&dim_sprite, Some(&sec), 500.0, &cfg).to_rgba8().to_u8_array();
        let bright = sprite_color(&bright_sprite, Some(&sec), 500.0, &cfg).to_rgba8().to_u8_array();
        let dim_sum = dim[0] as u32 + dim[1] as u32 + dim[2] as u32;
        let bright_sum = bright[0] as u32 + bright[1] as u32 + bright[2] as u32;
        assert!(
            bright_sum > dim_sum + 40,
            "full-bright should be much brighter than dim shaded: bright={bright:?} dim={dim:?}"
        );
    }

    #[test]
    fn wall_v_top_rowoffset_is_added() {
        // rowoffset shift directo del V_top — útil para alinear
        // texturas entre paredes adyacentes.
        let v0 = wall_v_top(0, 0, 0.0, 128.0, None, None, 128.0, 64.0, 0.0);
        let v8 = wall_v_top(0, 0, 0.0, 128.0, None, None, 128.0, 64.0, 8.0);
        assert!((v8 - v0 - 8.0).abs() < 1e-4, "expected +8 shift, got {} vs {}", v8, v0);
    }

    #[test]
    fn floor_and_ceiling_palettes_indexed_by_pic() {
        // Distintos floor_pic deben dar colores distintos cuando el módulo
        // los separa.
        let a = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 255,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let b = SectorSnap {
            floor_pic: 1,
            ..a.clone()
        };
        let cfg = RenderConfig::default();
        let ca = floor_color(&a, 0.0, &cfg);
        let cb = floor_color(&b, 0.0, &cfg);
        assert_ne!(ca.to_rgba8().to_u8_array(), cb.to_rgba8().to_u8_array());
    }

    // -----------------------------------------------------------------
    // Fase 3.13: BSP back-to-front traversal
    // -----------------------------------------------------------------

    /// Construye un BSP de 2 hojas con partición a X=0 y dx=0, dy=1
    /// (línea vertical). Front (children[0]) = subsector 0 (lado +X).
    /// Back (children[1]) = subsector 1 (lado -X).
    fn simple_two_leaf_bsp() -> Vec<NodeSnap> {
        vec![NodeSnap {
            partition_x: 0.0,
            partition_y: 0.0,
            partition_dx: 0.0,
            partition_dy: 1.0,
            children: [NF_SUBSECTOR | 0, NF_SUBSECTOR | 1],
        }]
    }

    #[test]
    fn bsp_walk_viewer_on_front_visits_back_first() {
        // Partición vertical x=0, dy=1. side = dx·(py - y) - dy·(px - x).
        // Para viewer en (+10, 0): side = 0·(0) - 1·(10) = -10 < 0 → back.
        // ¡Pero los hijos en Doom convention son [front, back] respecto a
        // R_PointOnSide, que dice `side > 0 = back` en su implementación
        // ¡pero usamos el signo opuesto! Verifiquemos lo que walk_bsp hace
        // realmente con esta config.
        // Implementación actual: side > 0 → near = children[0] (front lit).
        // side < 0 → near = children[1].
        // Para viewer en (+10, 0): side = -10 < 0 → near = children[1] = ss1,
        // far = children[0] = ss0. Visita ss0 primero (back-to-front).
        let nodes = simple_two_leaf_bsp();
        let mut out = Vec::new();
        walk_bsp(&nodes, (nodes.len() - 1) as u16, 10.0, 0.0, &mut out);
        assert_eq!(out, vec![0, 1], "viewer al +X visita ss0 (far) primero");
    }

    #[test]
    fn bsp_walk_viewer_on_back_visits_front_first() {
        // Para viewer en (-10, 0): side = -1·(-10) = +10 > 0 → near = children[0] = ss0,
        // far = children[1] = ss1. Visita ss1 primero (back-to-front).
        let nodes = simple_two_leaf_bsp();
        let mut out = Vec::new();
        walk_bsp(&nodes, (nodes.len() - 1) as u16, -10.0, 0.0, &mut out);
        assert_eq!(out, vec![1, 0], "viewer al -X visita ss1 (far) primero");
    }

    // -----------------------------------------------------------------
    // Fase 3.14: player overlays
    // -----------------------------------------------------------------

    #[test]
    fn overlay_none_when_all_counters_zero() {
        let ov = PlayerOverlays::default();
        assert!(overlay_rgba(&ov, 0).is_none());
    }

    #[test]
    fn overlay_damage_red_priority_over_bonus() {
        // damagecount tiene prioridad sobre bonuscount.
        let ov = PlayerOverlays {
            damage_count: 16,
            bonus_count: 16,
            ..Default::default()
        };
        let (r, g, b, _a) = overlay_rgba(&ov, 0).expect("overlay activo");
        // Es rojizo: r >> g, r >> b.
        assert!(r > g && r > b, "expected red dominant, got ({r}, {g}, {b})");
    }

    #[test]
    fn overlay_damage_alpha_scales_with_count() {
        let low = PlayerOverlays {
            damage_count: 4,
            ..Default::default()
        };
        let hi = PlayerOverlays {
            damage_count: 80,
            ..Default::default()
        };
        let (_, _, _, a_lo) = overlay_rgba(&low, 0).expect("low");
        let (_, _, _, a_hi) = overlay_rgba(&hi, 0).expect("hi");
        assert!(a_hi > a_lo, "alpha más grande con más daño: lo={a_lo} hi={a_hi}");
    }

    #[test]
    fn overlay_radsuit_blinks_in_last_seconds() {
        // power_radsuit < 4*32 (= 128): blinkea por bit 3 del tick.
        let ov = PlayerOverlays {
            power_radsuit: 50,
            ..Default::default()
        };
        // tick con bit 3 set (8, 9, 10, ...) → overlay activo (green).
        let on = overlay_rgba(&ov, 8);
        // tick con bit 3 limpio (0..7) → sin overlay.
        let off = overlay_rgba(&ov, 0);
        assert!(on.is_some(), "blink-on tick debe pintar verde");
        assert!(off.is_none(), "blink-off tick no debe pintar");
    }

    #[test]
    fn overlay_invuln_dominates_damage() {
        // Si hay invuln activo + damage, gana invuln (blanco, no rojo).
        let ov = PlayerOverlays {
            damage_count: 80,
            power_invuln: 200,
            ..Default::default()
        };
        let (r, g, b, _a) = overlay_rgba(&ov, 0).expect("overlay activo");
        // Blanco: r ~ g ~ b, todos altos.
        assert!(r > 180 && g > 180 && b > 180, "expected white-ish, got ({r}, {g}, {b})");
    }

    #[test]
    fn bsp_compute_depths_assigns_decreasing_values() {
        // Snapshot con 2 subsectors y el árbol simple. Compute_depths debe
        // asignar al subsector visitado primero (más lejano) el depth más
        // grande.
        let mut snap = SceneSnapshot::empty(0);
        snap.player.x = 10.0;
        snap.player.y = 0.0;
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());
        let depths = compute_bsp_order_depths(&snap);
        // ss0 visitado primero → depth grande. ss1 segundo → depth chico.
        let d0 = depths[0].expect("ss0 reached");
        let d1 = depths[1].expect("ss1 reached");
        assert!(d0 > d1, "ss0 (far) {d0} debe ser > ss1 (near) {d1}");
        // Ambos depths están sobre BSP_DEPTH_BASE para estar siempre detrás
        // de walls/sprites con depths euclidianos.
        assert!(d0 > BSP_DEPTH_BASE);
        assert!(d1 > BSP_DEPTH_BASE);
    }
}
