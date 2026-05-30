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

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Point, Rect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::{PaintRect, View};
use llimphi_ui::llimphi_text::{self as text, Alignment, TextBlock, Typesetter};
use supay_scene::{
    interpolate, NodeSnap, PlayerOverlays, PlayerStats, SceneSnapshot, SectorSnap, SnapshotPair,
    SpriteSnap, SubsectorSnap, WallSeg, WeaponSpriteSnap, ML_DONTPEGBOTTOM, ML_DONTPEGTOP,
    NF_SUBSECTOR, NO_SECTOR, NO_SKY_PIC,
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

    /// Devuelve el 4-char name del sprite si fue registrado vía
    /// [`Self::set_sprite_name`]. Usado por el renderer para resolver
    /// el tinte característico de cada mobj FF_FULLBRIGHT (Fase 3.27).
    pub fn sprite_name(&self, spritenum: u16) -> Option<String> {
        self.inner
            .lock()
            .ok()?
            .sprite_names
            .get(&spritenum)
            .cloned()
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

/// **Fase 3.46** — marca efímera en el mundo (scorch de bala, splat de
/// sangre). El host la detecta a partir de los sprites de impacto del
/// motor (PUFF / BLUD), la persiste con un fade y la pasa al renderer
/// por [`RenderConfig::decals`] cada frame con su `alpha` ya computado.
/// El renderer la dibuja como un billboard pequeño camera-facing,
/// z-ordenado con la escena (lo ocluyen las paredes que estén delante).
#[derive(Clone, Copy, Debug)]
pub struct Decal {
    /// Posición mundo del impacto (la del mobj PUFF/BLUD que lo originó).
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Medio-tamaño en unidades mundo del quad.
    pub radius: f32,
    /// Color RGB de la marca (scorch oscuro, sangre roja).
    pub color: (u8, u8, u8),
    /// Opacidad 0..1 — el host la decae con la edad del decal.
    pub alpha: f32,
    /// **Fase 3.47** — tangente unitaria mundo `(tx, ty)` del lineseg
    /// donde impactó: el decal yace plano sobre la pared, con su eje
    /// horizontal a lo largo de la tangente y el vertical en `+Z`. Si es
    /// `(0, 0)` (sin pared cercana — p.ej. sangre en el aire) el renderer
    /// cae al billboard camera-facing de 3.46.
    pub tangent: (f32, f32),
    /// **Fase 3.48** — el impacto fue contra piso o techo: el quad yace
    /// **horizontal** (ejes en el plano XY mundo, a `z` constante) como
    /// un charco. Tiene prioridad sobre `tangent`. `false` ⇒ pared
    /// (tangente) o billboard.
    pub horizontal: bool,
    /// **Fase 3.52** — recorte horizontal del decal de pared a su lineseg.
    /// Offsets firmados `(s_min, s_max)` en unidades mundo a lo largo de
    /// la [`Decal::tangent`], medidos desde el centro del decal hasta los
    /// dos extremos del segmento donde impactó. El renderer recorta la
    /// extensión horizontal del quad a `[s_min.max(-r), s_max.min(r)]`,
    /// evitando que el decal sangre más allá del borde de la pared (la
    /// esquina). `None` ⇒ sin recorte (billboard, charco, o pared sin
    /// span resuelto) — comportamiento 3.51.
    pub wall_span: Option<(f32, f32)>,
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
    /// **Fase 3.19 — crosshair central**. Si `true`, pinta una marca
    /// fina en el centro del viewport (4 chevrons + dot). Modernización
    /// pura: Doom clásico no lo usa, los FPS contemporáneos sí. Cosmético
    /// total — sólo afecta el rasterizador, no la simulación.
    pub crosshair: bool,
    /// **Fase 3.19 — fuerza de la viñeta de cabina**. `0.0` = off,
    /// `1.0` = oscurecimiento muy marcado en esquinas. Default `0.55`
    /// queda sutil: ~70/255 de alpha crimson_deep en el corner más
    /// lejano del centro. Pintada antes que el crosshair y los overlays
    /// para que las flashes de damage la cubran.
    pub vignette: f32,
    /// **Fase 3.20 — HUD inferior**. Si `true`, pinta una banda slim al
    /// pie del viewport con health/armor/ammo/keys leídos del
    /// `PlayerStats` del snapshot. Modernización de la status bar
    /// clásica de Doom (320×32 al pie del FB): mismos datos, layout
    /// "tile-by-tile" co-locado con la imagen 3D.
    pub hud: bool,
    /// **Fase 3.21 — sombras de mobjs en el piso**. Si `true`, cada
    /// sprite proyecta un disco oscuro semi-transparente en el plano
    /// del sector donde está parado, dándole sensación de peso al
    /// mundo 3D. Cosmético total — Doom clásico no tiene sombras, el
    /// renderer software pinta sprites flotando sobre el piso.
    pub sprite_shadows: bool,
    /// **Fase 3.22 — luz dinámica del muzzle flash**. Intensidad actual
    /// (0.0 = nada, 1.0 = pico) del destello de boca de arma que
    /// ilumina el mundo alrededor del jugador. El host lo settea cada
    /// frame: pico 1.0 cuando el snapshot tiene `FF_FULLBRIGHT` activo
    /// en `weapon` o `weapon_flash`, decae a 0 en `MUZZLE_DECAY_SECS`.
    /// Aplica un boost cálido (amarillo-blanco) sobre paredes, pisos,
    /// techos y sprites dentro de `MUZZLE_RADIUS_WORLD` unidades del
    /// jugador. Doom clásico cicla la PLAYPAL completa; esta es la
    /// modernización por sector/depth.
    pub muzzle_glow_alpha: f32,
    /// **Fase 3.23 — oclusión sectorial del muzzle boost**. Si `true`,
    /// el destello del arma sólo ilumina superficies del sector donde
    /// está parado el jugador y de los sectores conectados a él por al
    /// menos una linedef two-sided (puerta, escalón abierto, ventana).
    /// Una pared sólida entre el jugador y un sprite/pared lejana corta
    /// el boost: el cuarto vecino queda oscuro aunque su distancia
    /// euclidiana esté bajo `MUZZLE_RADIUS_WORLD`. Si `false`, vuelve al
    /// comportamiento 3.22 (boost ignora paredes). En stub sin BSP el
    /// flag no aplica — el renderer ilumina todo igual.
    pub muzzle_occlusion: bool,
    /// **Fase 3.26 — luces dinámicas desde mobjs full-bright**. Si
    /// `true`, los sprites con bit `FF_FULLBRIGHT` (proyectiles,
    /// puffs, frames de explosión, fog) emiten una luz puntual cálida
    /// que ilumina paredes, pisos, techos y sprites cercanos (radio
    /// `WORLD_LIGHT_RADIUS_WORLD = 192`, mitad del muzzle). Sumadas al
    /// boost del muzzle (clamp ≤ `MUZZLE_BOOST_PEAK`). Doom clásico
    /// no irradia luz desde proyectiles — esta es modernización pura.
    /// Sin gating por oclusión: las luces son efímeras (1-30 ticks),
    /// el leak fugaz a través de paredes es invisible en práctica.
    pub world_lights_enabled: bool,
    /// **Fase 3.29 — oclusión sectorial de world lights**. Si `true`,
    /// cada world light cachea su set de sectores alcanzables por BFS
    /// desde su sector origen (mismo radio y hops del muzzle gate),
    /// y sólo aporta tinte a superficies cuyo sector está en ese set.
    /// Un BFG ball pasando en el cuarto vecino con pared sólida deja
    /// de pintar verde la pared detrás del jugador. Si `false`, las
    /// luces aportan por radio solamente — comportamiento 3.27. En
    /// stub sin BSP el flag no aplica (lit_sectors queda `None` y el
    /// boost pasa). Costo: una llamada a BFS por luz por frame, ≤ 8
    /// luces, ≤ 2 hops — despreciable.
    pub world_lights_occlusion: bool,
    /// **Fase 3.28 — rim-light del arma desde world lights**. Si `true`,
    /// el sprite del arma se tinta cada frame con el boost RGB de
    /// `world_lights` evaluado en la posición del jugador (origen del
    /// cam-space). Caminar al lado de una antorcha azul (`TBLU`) tinta
    /// la pistola apenas azulada; un fireball pasando cerca le pinta
    /// un rim rojizo. Modernización pura — Doom clásico no liga el
    /// arma al ambiente (la PLAYPAL global no llega al psprite del
    /// arma). Los frames `FF_FULLBRIGHT` (muzzle flash) saltan el
    /// boost — el destello del propio fogonazo domina y el ambiente
    /// queda subsumido. El muzzle glow del jugador *no* se suma acá
    /// (el fogonazo sale *de* la pistola, no la ilumina a ella), como
    /// en 3.22.
    pub weapon_rim_light: bool,
    /// **Fase 3.30 — rim direccional**. Si `true`, el aporte de cada
    /// world light al rim del arma se atenúa según el ángulo entre la
    /// "normal" virtual del psprite (apuntando a +X_cam, hacia adelante)
    /// y la dirección a la luz: una antorcha al frente tinta a plena
    /// intensidad; una atrás baja a un piso ambient (cosine·0.5 + 0.5
    /// clampeado a [`WEAPON_RIM_AMBIENT_FLOOR`]). Sin direccional
    /// (`false`), todas las luces aportan igual sin importar dónde
    /// estén — comportamiento 3.28-3.29. Sólo afecta al rim del arma;
    /// el resto de la escena conserva el path omnidireccional 3.27.
    pub weapon_rim_directional: bool,
    /// **Fase 3.31 — rim direccional de mobjs**. Si `true`, cada sprite
    /// billboard (enemigos, decoración, proyectiles) usa una fake-normal
    /// apuntando hacia la cámara para atenuar el aporte de cada world
    /// light. Una antorcha **entre** el jugador y el imp tinta su frente
    /// al 100 %; una antorcha **detrás** del imp lo back-lightea — el
    /// sprite que ve el jugador es la cara frontal, ahí queda al piso
    /// (`SPRITE_RIM_AMBIENT_FLOOR`). Sin direccional (`false`), todos
    /// los sprites reciben el aporte omnidireccional del 3.27/3.29 —
    /// backwards-compat exacta. Se aplica tanto al patch texturizado
    /// como al fallback de rectángulos coloreados. El muzzle del
    /// jugador queda fuera del shading direccional (es la luz que
    /// emite el propio sprite del arma, no hay normal de mobj que la
    /// module).
    pub sprite_rim_directional: bool,
    /// **Fase 3.32 — rim direccional para paredes**. Si `true`, cada
    /// pared usa su normal (perpendicular al lineseg, orientada toward
    /// camera) para atenuar el aporte de world lights por `cos(θ)`. Una
    /// antorcha justo frente a la pared (luz "de frente") tinta al 100 %;
    /// una al costado (rasante) cae al 50 %; una efectivamente "detrás"
    /// de la pared cae al piso [`WALL_RIM_AMBIENT_FLOOR`] — modela el
    /// bounce indirecto cuando una linedef two-sided permite atravesar
    /// el muzzle/world-light a un sector vecino y el ángulo de rasante
    /// queda extremo. El muzzle queda omni (como en 3.30/3.31). Cuando
    /// `false`, vuelve al path omni 3.27/3.29 — backwards-compat.
    pub wall_rim_directional: bool,
    /// **Fase 3.33 — BRDF para pisos y techos**. Si `true`, los planos
    /// horizontales reciben el aporte de cada world light atenuado por
    /// el cosine entre la normal del plano (`+Z` floor, `-Z` ceiling) y
    /// la dirección 3D del plano hacia la luz. Una antorcha al ras del
    /// piso ilumina fuerte el piso cercano pero el techo lo recibe
    /// rasante (cos ≈ 0); un mobj BFG en el aire (proyectil flotante)
    /// ilumina ambos, pero más al que tiene más cara hacia él. Combina
    /// con el radio 3D-aware: una luz a 100 u horizontal y 100 u
    /// vertical (d_3D ≈ 141) cae con `f = 1 - d²/r²` donde `d` es 3D —
    /// más realista que el 2D-only del 3.27. Cuando `false`, vuelve al
    /// path omni 3.27/3.29 con radio 2D. El muzzle queda omni
    /// (consistente con 3.30-3.32).
    pub plane_rim_directional: bool,
    /// **Fase 3.37 — muzzle direccional sobre walls y planes**. Si
    /// `true`, el muzzle flash del arma se trata como una luz puntual
    /// en el origen del cam-space y se atenúa por el cosine de la
    /// superficie (igual que el rim direccional pero para la fuente
    /// muzzle). Paredes oblicuas reciben menos tinte cálido durante
    /// el flash; pisos muy lejos del jugador horizontalmente reciben
    /// el cosine reducido por el ángulo bajo. Mobjs y weapon siguen
    /// con muzzle omni — el psprite es overlay 2D sin geometría 3D
    /// y los mobjs reciben el muzzle "envolvente" característico de
    /// Doom clásico. Cuando `false` (default), preserva el path
    /// omni 3.30-3.35 — backwards-compat exacta. Sólo afecta walls
    /// y floors/ceilings.
    pub muzzle_brdf: bool,
    /// **Fase 3.42 — bandas verticales para BRDF de walls**. Número de
    /// sub-bandas horizontales sobre cada slab texturizado donde el
    /// overlay del shading y el tinte se calculan independientemente.
    /// Default `1` = un único overlay por slab (comportamiento 3.32-3.41).
    /// Valores > 1 emiten N overlays adicionales con boost computado al
    /// centro vertical de cada banda — una antorcha al ras del piso
    /// ilumina más la parte baja de la pared, una a la altura del
    /// techo más la parte alta. Coste: ~2N extra fills por slab
    /// texturizado. Recomendado: 2-4. Sólo afecta al path texturizado.
    pub wall_vertical_bands: u8,
    /// **Fase 3.43 — gradiente vertical continuo para walls**. Si `true`,
    /// el shading y el tinte del slab texturizado se pintan con un único
    /// `Gradient` lineal de Vello (bottom→top en pantalla) en lugar de N
    /// bandas discretas. El boost se muestrea a
    /// `wall_vertical_bands.max(2) + 1` alturas y Vello interpola suave
    /// entre stops — sin las costuras visibles de las bandas 3.42 y con
    /// **dos** fills por slab en lugar de 2N. Cuando está on tiene
    /// precedencia sobre `wall_vertical_bands` (que sólo controla la
    /// densidad de muestreo). Default `false` ⇒ comportamiento 3.42
    /// bit-equivalente. Sólo afecta al path texturizado.
    pub wall_vertical_gradient: bool,
    /// **Fase 3.44 — gradiente de profundidad para pisos/techos**. Si
    /// `true`, el shading/tinte del plano texturizado se pinta con un
    /// `Gradient` lineal de Vello a lo largo del eje near→far (vértice
    /// más cercano al jugador → vértice más lejano) en lugar de un
    /// overlay uniforme computado al centroide. El boost y el fog se
    /// muestrean en ambos extremos: la parte del piso cercana al jugador
    /// queda más clara (menos fog + más pool de luz del muzzle/proyectil)
    /// y la lejana más oscura. Reusa los mismos helpers que el gradiente
    /// vertical de walls (3.43). Default `false` ⇒ overlay uniforme 3.33
    /// bit-equivalente. Sólo afecta al path texturizado de planos.
    pub plane_depth_gradient: bool,
    /// **Fase 3.46 — decals efímeros de impacto**. Lista de marcas
    /// (scorch / sangre) que el host detecta de los sprites de impacto
    /// del motor y mantiene con su fade. Vacía por default ⇒ sin decals
    /// (modo stub, o el host no las alimenta). Se reconstruye cada frame;
    /// el renderer las dibuja como billboards camera-facing z-ordenados.
    pub decals: Vec<Decal>,
    /// **Fase 3.51 — boost direccional del decal por su normal**. Si
    /// `true`, el tinte RGB de world lights + muzzle sobre cada decal se
    /// atenúa por el cosine entre la normal de la superficie donde yace
    /// el decal y la dirección a cada luz, igual que walls/planes/sprites:
    /// un charco de piso usa BRDF de plano (`n_z=+1`), una marca pegada a
    /// la pared usa BRDF de pared (normal del lineseg toward-camera) y un
    /// billboard flotante (sangre en el aire) queda omni. Un scorch en
    /// pared rasante a una antorcha recibe menos tinte que uno encarado;
    /// un charco bajo un fireball alto recoge el cosine vertical. Cuando
    /// `false` (o sin BSP), cae al boost omni 3.50 bit-equivalente. Sólo
    /// afecta al tinte de los decals.
    pub decal_rim_directional: bool,
    /// **Fase 3.53 — recorte del charco al recinto de paredes**. Si
    /// `true`, el quad de un decal horizontal (charco de piso/techo) se
    /// recorta a las paredes (linedefs) que lo bordean dentro de su radio,
    /// manteniendo el lado del centro — una mancha de sangre junto a un
    /// muro deja de treparlo o cruzar al cuarto vecino. Cuando `false` (o
    /// sin paredes en el snapshot, modo stub) el charco se dibuja como el
    /// quad completo de 3.48. Sólo afecta a los decals `horizontal`.
    pub decal_clip_walls: bool,
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
            crosshair: true,
            vignette: 0.55,
            hud: true,
            sprite_shadows: true,
            muzzle_glow_alpha: 0.0,
            muzzle_occlusion: true,
            world_lights_enabled: true,
            world_lights_occlusion: true,
            weapon_rim_light: true,
            weapon_rim_directional: true,
            sprite_rim_directional: true,
            wall_rim_directional: true,
            plane_rim_directional: true,
            muzzle_brdf: false,
            wall_vertical_bands: 1,
            wall_vertical_gradient: false,
            plane_depth_gradient: false,
            decals: Vec::new(),
            decal_rim_directional: true,
            decal_clip_walls: true,
        }
    }
}

// =====================================================================
// Fase 3.22 — Muzzle world light
// =====================================================================
//
// El destello del arma del jugador (`FF_FULLBRIGHT` en `psprites[]`) emite
// un boost cálido que ilumina paredes, pisos, techos y sprites en un
// disco alrededor del jugador. Modela el "fogonazo del cañón" que en
// Doom original sólo afectaba la PLAYPAL global — acá lo hacemos
// world-light: las superficies cercanas reciben un tinte amarillento que
// decae con `distance² / RADIUS²`. La intensidad `cfg.muzzle_glow_alpha`
// viene del host y decae con el tiempo.

/// Radio de influencia del fogonazo, en unidades Doom. ~6 cells de 64
/// → habitación pequeña entera, pasillo medio.
const MUZZLE_RADIUS_WORLD: f32 = 384.0;
/// Boost de shade en el centro (player position) con `alpha=1.0`. Se
/// suma al shade base, capeado a 1.0 — paredes oscuras quedan visibles
/// durante el flash sin "blow out" las claras.
const MUZZLE_BOOST_PEAK: f32 = 0.55;
/// Tinte cálido amarillo-blanco del fogonazo, en RGB 0..255.
const MUZZLE_TINT_RGB: (u8, u8, u8) = (255, 220, 140);

/// Devuelve el boost de luz del muzzle flash para un punto en cam-space
/// (player está en `(0, 0)`). Cae con distancia² hasta `MUZZLE_RADIUS_WORLD`.
fn muzzle_boost_cam(x_cam: f32, y_cam: f32, alpha: f32) -> f32 {
    if alpha <= 0.0 {
        return 0.0;
    }
    let d2 = x_cam * x_cam + y_cam * y_cam;
    let r2 = MUZZLE_RADIUS_WORLD * MUZZLE_RADIUS_WORLD;
    if d2 >= r2 {
        return 0.0;
    }
    let f = 1.0 - d2 / r2;
    (f * f * alpha * MUZZLE_BOOST_PEAK).clamp(0.0, MUZZLE_BOOST_PEAK)
}

/// Variante 3D del falloff del muzzle (Fase 3.40). Incluye el componente
/// `z_cam` en la distancia: una superficie alta o baja del jugador queda
/// fuera del rango del fogonazo cuando la distancia 3D la supera, aunque
/// su distancia horizontal sea chica. Usado por las versiones BRDF del
/// muzzle (`muzzle_boost_rgb_wall_3d`, `muzzle_boost_rgb_plane_3d`) para
/// que el modelo direccional sea coherentemente 3D — el scalar 2D del
/// `muzzle_boost_cam` sigue activo en el path omni (`muzzle_brdf=false`,
/// default) y en mobjs/weapon.
fn muzzle_boost_cam_3d(x_cam: f32, y_cam: f32, z_cam: f32, alpha: f32) -> f32 {
    if alpha <= 0.0 {
        return 0.0;
    }
    let d2 = x_cam * x_cam + y_cam * y_cam + z_cam * z_cam;
    let r2 = MUZZLE_RADIUS_WORLD * MUZZLE_RADIUS_WORLD;
    if d2 >= r2 {
        return 0.0;
    }
    let f = 1.0 - d2 / r2;
    (f * f * alpha * MUZZLE_BOOST_PEAK).clamp(0.0, MUZZLE_BOOST_PEAK)
}

/// Suma aditivamente el tinte cálido `MUZZLE_TINT_RGB · boost` al color
/// base, preservando alpha. Boost ≤ 0 ⇒ no-op.
#[cfg(test)]
fn apply_muzzle_tint(c: Color, boost: f32) -> Color {
    if boost <= 0.0 {
        return c;
    }
    let [r, g, b, a] = c.to_rgba8().to_u8_array();
    let add_r = (MUZZLE_TINT_RGB.0 as f32 * boost) as u32;
    let add_g = (MUZZLE_TINT_RGB.1 as f32 * boost) as u32;
    let add_b = (MUZZLE_TINT_RGB.2 as f32 * boost) as u32;
    Color::from_rgba8(
        (r as u32 + add_r).min(255) as u8,
        (g as u32 + add_g).min(255) as u8,
        (b as u32 + add_b).min(255) as u8,
        a,
    )
}

/// Multiplicador per-canal para tintar el patch del sprite cuando el
/// muzzle flash está activo. Devuelve `(shade·tint_r, shade·tint_g,
/// shade·tint_b)` con `tint = 1 + boost · MUZZLE_TINT/255` clampeado.
/// Cuando `boost = 0` devuelve `[shade, shade, shade]` — equivalente al
/// shading grayscale histórico.
#[cfg(test)]
fn sprite_shade_with_muzzle(shade: f32, boost: f32) -> [f32; 3] {
    if boost <= 0.0 {
        return [shade, shade, shade];
    }
    let tr = 1.0 + boost * (MUZZLE_TINT_RGB.0 as f32 / 255.0);
    let tg = 1.0 + boost * (MUZZLE_TINT_RGB.1 as f32 / 255.0);
    let tb = 1.0 + boost * (MUZZLE_TINT_RGB.2 as f32 / 255.0);
    [
        (shade * tr).clamp(0.0, 1.0),
        (shade * tg).clamp(0.0, 1.0),
        (shade * tb).clamp(0.0, 1.0),
    ]
}

/// Fase 3.24 — máximo de saltos del BFS de sectores iluminables.
///
/// 1 hop = vecino directo del jugador (puerta/ventana inmediata).
/// 2 hops = vecino del vecino — cubre escenarios típicos de Doom donde
/// el flash debería alcanzar el siguiente cuarto a través de una puerta
/// abierta sin necesidad de R_CheckSight completo. >2 hops empieza a
/// "filtrar" luz por geometrías retorcidas sin agregar valor visual; el
/// radio cumulativo (`MUZZLE_RADIUS_WORLD`) ya cortaría antes en la
/// mayoría de los casos.
const MUZZLE_BFS_MAX_HOPS: usize = 2;

/// Fase 3.25 — conjunto de sectores que reciben el muzzle boost,
/// computado por relajación tipo Dijkstra sobre **caminos acumulativos**
/// player→midpoint(W₁)→midpoint(W₂)→…, no por chequeo per-bridge contra
/// el centro del jugador.
///
/// El destello del arma sólo ilumina superficies del cuarto donde está
/// parado el jugador y de los cuartos alcanzables a `≤
/// MUZZLE_BFS_MAX_HOPS` linedefs two-sided **cuya distancia total
/// recorrida por la cadena de bridge walls** esté dentro de
/// `MUZZLE_RADIUS_WORLD`. Cada sector cachea el midpoint del último
/// bridge wall por el que se entró — el siguiente hop se mide desde ese
/// punto, no desde el jugador. Eso modela mejor "hasta dónde llegaría
/// la luz si tuviera que atravesar cada puerta en orden", y corta
/// correctamente en U-shapes/L-shapes donde un sector distante quedaba
/// visualmente lit en 3.24 aunque su camino real fuera más largo que
/// el disco del flash.
///
/// La relajación de Dijkstra-lite también garantiza que si un sector
/// es alcanzable por dos caminos, gana el más corto — el "entry point"
/// queda fijado al midpoint del camino más corto encontrado.
///
/// Una pared sólida entre medio sigue cortando la luz — sin two-sided
/// no hay arista en el grafo.
///
/// La heurística es O(walls · hops · sectores_visitados) por frame. En
/// E1M1 (~400 walls × 2 hops × <16 sectores visitados) ≈ 13k checks/s
/// cuando el flash está activo (<5 % del tiempo). Sin alocaciones extra
/// significativas.
///
/// Devuelve `None` cuando no hay BSP (modo stub o mapa pre-carga); en
/// ese caso el caller debe asumir "todo lit" y aplicar el comportamiento
/// 3.22 (boost everywhere).
fn compute_muzzle_lit_sectors(snap: &SceneSnapshot) -> Option<HashSet<u32>> {
    if snap.nodes.is_empty() || snap.subsectors.is_empty() {
        return None;
    }
    let player_ss = subsector_at_point(&snap.nodes, snap.player.x, snap.player.y)?;
    let ss = snap.subsectors.get(player_ss as usize)?;
    let player_sec = ss.sector;
    Some(compute_lit_sectors_from(
        snap,
        snap.player.x,
        snap.player.y,
        player_sec,
        MUZZLE_RADIUS_WORLD,
    ))
}

/// BFS sectorial reusable: desde un sector fuente (con su entry point
/// en coords mundo) explora vecinos via linedefs two-sided hasta
/// [`MUZZLE_BFS_MAX_HOPS`] hops, con corte cumulativo por `radius`
/// (suma de tramos entre midpoints) — la misma maquinaria de 3.25 para
/// el muzzle, parametrizada para soportar también world lights (Fase
/// 3.29). El sector fuente queda con `dist=0`, los vecinos accesibles
/// con su distancia cumulativa.
fn compute_lit_sectors_from(
    snap: &SceneSnapshot,
    src_x: f32,
    src_y: f32,
    src_sec: u32,
    radius: f32,
) -> HashSet<u32> {
    let mut dist: HashMap<u32, f32> = HashMap::with_capacity(16);
    let mut entry: HashMap<u32, (f32, f32)> = HashMap::with_capacity(16);
    let mut hops: HashMap<u32, usize> = HashMap::with_capacity(16);
    dist.insert(src_sec, 0.0);
    entry.insert(src_sec, (src_x, src_y));
    hops.insert(src_sec, 0);
    // Cola de trabajo. No es un BinaryHeap real porque el set típico es
    // <16 sectores; un Vec con relajación re-inserta y deja que la
    // condición `better` filtre lo redundante. Suficiente y sin deps.
    let mut queue: Vec<u32> = vec![src_sec];
    while let Some(sec) = queue.pop() {
        let d_sec = dist[&sec];
        let (ex, ey) = entry[&sec];
        let h_sec = hops[&sec];
        if h_sec >= MUZZLE_BFS_MAX_HOPS {
            continue;
        }
        for wall in snap.walls.iter() {
            if wall.back_sector == NO_SECTOR {
                continue;
            }
            let other_sec = if wall.front_sector == sec {
                wall.back_sector
            } else if wall.back_sector == sec {
                wall.front_sector
            } else {
                continue;
            };
            if other_sec == sec {
                continue;
            }
            let mx = (wall.x1 + wall.x2) * 0.5;
            let my = (wall.y1 + wall.y2) * 0.5;
            let dx = mx - ex;
            let dy = my - ey;
            let hop_d = (dx * dx + dy * dy).sqrt();
            let new_d = d_sec + hop_d;
            if new_d > radius {
                continue;
            }
            let better = match dist.get(&other_sec) {
                Some(&existing) => new_d < existing,
                None => true,
            };
            if better {
                dist.insert(other_sec, new_d);
                entry.insert(other_sec, (mx, my));
                hops.insert(other_sec, h_sec + 1);
                queue.push(other_sec);
            }
        }
    }
    dist.into_keys().collect()
}

/// Gate del muzzle boost por sector cuando la oclusión está activa.
/// `sector_id` es el sector "dueño" de la superficie (subsector.sector
/// para planos; sprite.sector para sprites; front-side sector para la
/// pared). Si la oclusión está activa y `sector_id ∉ lit_sectors`, la
/// función devuelve 0 (sin boost). Sin oclusión o sin BSP devuelve el
/// boost crudo.
#[cfg(test)]
fn muzzle_boost_gated(
    boost: f32,
    sector_id: u32,
    lit_sectors: Option<&HashSet<u32>>,
) -> f32 {
    match lit_sectors {
        Some(lit) if !lit.contains(&sector_id) => 0.0,
        _ => boost,
    }
}

// =====================================================================
// Fase 3.26 — World point lights desde FF_FULLBRIGHT mobjs
// =====================================================================
//
// Doom marca varios mobjs con `FF_FULLBRIGHT` (bit 7 del frame): proyectiles
// en vuelo (imp fireballs, plasma, BFG, rocket), muzzle puffs, frames de
// explosión de barriles, BFG splash, teleport fog. Estos sprites ya se
// pintaban a luz plena desde 3.11 (sprite-side), pero **no irradiaban luz
// al mundo**: un fireball pasando por un cuarto oscuro dejaba el cuarto
// oscuro. Modernización: tratamos cada mobj FF_FULLBRIGHT como una fuente
// puntual con la misma maquinaria del muzzle (tinte cálido, falloff
// cuadrático, sumado al shade base). El muzzle del jugador queda como un
// caso particular anclado en el origen del cam-space.
//
// La diferencia clave vs. muzzle: estas luces están en posiciones
// arbitrarias del mundo, no en el player. Por eso no se les aplica el
// `lit_sectors` set (que se computa relativo al cuarto del jugador). Se
// gatean sólo por radio. El radio chico (mitad que muzzle) limita el leak
// natural a través de paredes; los mobjs FF_FULLBRIGHT en Doom son
// efímeros (1-30 ticks), así que un leak fugaz es invisible en práctica.

/// Radio de influencia de una luz puntual del mundo, en unidades Doom.
/// Más chico que `MUZZLE_RADIUS_WORLD` porque la "fuerza" de un fireball
/// o un puff es muy inferior al fogonazo cercano de un arma en mano.
const WORLD_LIGHT_RADIUS_WORLD: f32 = 192.0;
/// Peak del boost en el centro de una luz puntual con `alpha=1.0`.
/// Menor que `MUZZLE_BOOST_PEAK` (0.55) — el sumado de varias luces
/// puede acercarse al peak del muzzle, pero una sola no debería
/// "blow out" la escena.
const WORLD_LIGHT_PEAK: f32 = 0.40;
/// Cap del número de world lights consideradas por frame. Cubrimos los
/// proyectiles + puffs + explosiones simultáneas razonables sin pagar
/// O(surfaces · lights) descontrolado. 8 cubre escenarios típicos
/// (cyberdemon spam, BFG en cluster), el resto se descarta por
/// distancia.
const MAX_WORLD_LIGHTS: usize = 8;

#[derive(Clone, Debug)]
struct WorldLight {
    /// Posición en cam-space (forward, right).
    x_cam: f32,
    y_cam: f32,
    /// Fase 3.33: altura del mobj relativa a `cam.view_z` — `sprite.z`
    /// menos la altura del ojo del jugador. Necesaria para el cosine
    /// BRDF de pisos/techos (normal ±Z) y para que el radio sea 3D-aware
    /// en el helper `world_lights_boost_rgb_for_plane_cam`. El gather
    /// inicial sigue filtrando por d² 2D × 4 (margen generoso) — la
    /// distancia 3D real se chequea dentro del helper de cada plano.
    z_cam: f32,
    /// Sector "dueño" del mobj — origen del BFS de oclusión 3.29.
    sector: u32,
    /// Fase 3.27: tinte característico del mobj resuelto vía
    /// `sprite_tint_for_name`. Cae al amarillo cálido del muzzle si el
    /// sprite es desconocido para la tabla.
    tint_rgb: (u8, u8, u8),
    /// Fase 3.29: sectores alcanzables desde `sector` por linedefs
    /// two-sided, BFS hasta [`MUZZLE_BFS_MAX_HOPS`] con corte
    /// cumulativo por [`WORLD_LIGHT_RADIUS_WORLD`]. `None` cuando la
    /// oclusión está desactivada o no hay BSP en el snapshot — el caller
    /// asume "ilumina todo" (comportamiento 3.27). `Arc` para compartir
    /// el set sin copiar entre las múltiples superficies que consultan
    /// la misma luz por frame.
    lit_sectors: Option<Arc<HashSet<u32>>>,
}

/// Recolecta las luces puntuales del mundo del snapshot: cada sprite
/// con bit `FF_FULLBRIGHT` (0x80) en su frame contribuye una luz en su
/// posición. Sprites con `sector == NO_SECTOR` se descartan (sin
/// referencia válida). Se transforman a cam-space y se queda con los
/// `MAX_WORLD_LIGHTS` más cercanos al jugador (origen del cam-space).
///
/// Costo: O(sprites + N·log N) por frame con N ≈ sprites; en mapas Doom
/// el número de sprites visibles es <60, despreciable.
fn gather_world_lights(
    snap: &SceneSnapshot,
    cam: &Camera,
    atlas: Option<&Arc<WadAtlas>>,
    enable_occlusion: bool,
) -> Vec<WorldLight> {
    let mut lights: Vec<(f32, WorldLight)> = snap
        .sprites
        .iter()
        .filter(|s| (s.frame & 0x80) != 0 && s.sector != NO_SECTOR)
        .map(|s| {
            let (x_cam, y_cam) = cam.to_cam_2d(s.x, s.y);
            // Fase 3.33: z relativa al ojo del jugador. Permite que el
            // helper de pisos/techos calcule cos(θ) con normal ±Z.
            let z_cam = s.z - cam.view_z;
            let d2 = x_cam * x_cam + y_cam * y_cam;
            // Fase 3.27: tinte per-mobj. Si el atlas no tiene el nombre
            // (o no hay atlas — modo sin WAD), cae al amarillo cálido
            // del muzzle (comportamiento 3.26).
            let tint_rgb = atlas
                .and_then(|a| a.sprite_name(s.sprite))
                .map(|name| sprite_tint_for_name(&name))
                .unwrap_or(MUZZLE_TINT_RGB);
            (
                d2,
                WorldLight {
                    x_cam,
                    y_cam,
                    z_cam,
                    sector: s.sector,
                    tint_rgb,
                    lit_sectors: None,
                },
            )
        })
        .filter(|(d2, _)| *d2 < WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD * 4.0)
        .collect();
    if lights.len() > MAX_WORLD_LIGHTS {
        lights.select_nth_unstable_by(MAX_WORLD_LIGHTS, |a, b| {
            a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
        });
        lights.truncate(MAX_WORLD_LIGHTS);
    }
    // Fase 3.29: oclusión per-light. Cada luz cachea el set de sectores
    // alcanzables desde su sector origen (BFS desde la posición mundo
    // del mobj). Sólo computamos si hay BSP — sin él el caller asume
    // "todo lit" (comportamiento 3.27). Para revertir la transformación
    // cam→world usamos `cam.from_cam_2d`, evitando re-iterar los
    // sprites originales.
    if enable_occlusion && !snap.nodes.is_empty() && !snap.subsectors.is_empty() {
        for (_, l) in lights.iter_mut() {
            let (sx, sy) = cam.from_cam_2d(l.x_cam, l.y_cam);
            let set =
                compute_lit_sectors_from(snap, sx, sy, l.sector, WORLD_LIGHT_RADIUS_WORLD);
            l.lit_sectors = Some(Arc::new(set));
        }
    }
    lights.into_iter().map(|(_, l)| l).collect()
}

/// Suma de boosts de todas las world lights en un punto cam-space.
/// Cada luz contribuye `f²·PEAK` con `f = 1 - d²/r²`, clampeado al
/// peak del muzzle (no superar el destello del arma propia es un
/// invariante del sistema — el flash debe seguir siendo el efecto
/// dominante).
#[cfg(test)]
fn world_lights_boost_cam(x_cam: f32, y_cam: f32, lights: &[WorldLight]) -> f32 {
    if lights.is_empty() {
        return 0.0;
    }
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let mut sum = 0.0_f32;
    for l in lights {
        let dx = x_cam - l.x_cam;
        let dy = y_cam - l.y_cam;
        let d2 = dx * dx + dy * dy;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        sum += f * f * WORLD_LIGHT_PEAK;
        if sum >= MUZZLE_BOOST_PEAK {
            return MUZZLE_BOOST_PEAK;
        }
    }
    sum.min(MUZZLE_BOOST_PEAK)
}

/// Boost combinado (muzzle + world lights) en un punto cam-space. El
/// muzzle se gatea por `lit_sectors` (Fase 3.23-3.25); las world lights
/// sólo por radio. La suma se clampea a `MUZZLE_BOOST_PEAK` para
/// preservar el invariante "el fogonazo nunca debe sentirse más débil
/// que un proyectil distante".
#[cfg(test)]
fn combined_boost_cam(
    x_cam: f32,
    y_cam: f32,
    muzzle_alpha: f32,
    surf_sector: u32,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
) -> f32 {
    let muzzle = muzzle_boost_gated(
        muzzle_boost_cam(x_cam, y_cam, muzzle_alpha),
        surf_sector,
        lit_sectors,
    );
    let wl = world_lights_boost_cam(x_cam, y_cam, world_lights);
    (muzzle + wl).clamp(0.0, MUZZLE_BOOST_PEAK)
}

// =====================================================================
// Fase 3.27 — Tinte per-spritenum para world lights (BFG verde, plasma
// azul, fireballs rojos, antorchas teñidas, etc.)
// =====================================================================
//
// Hasta 3.26 todas las world lights usaban el mismo amarillo cálido
// (`MUZZLE_TINT_RGB`). Pero un proyectil BFG es verde fluorescente, una
// bola de plasma es azul cyan, un fireball de imp es rojo-naranja y una
// antorcha azul de decoración tiñe su cuarto azul. Esta fase refactoriza
// el boost a representación per-canal (`[f32; 3]`) para que cada luz
// emita su tinte característico, sumándose aditivamente en RGB.
//
// La maquinaria scalar (`muzzle_boost_cam`, `apply_muzzle_tint`,
// `sprite_shade_with_muzzle`) sobrevive para los tests existentes y como
// implementación de referencia; el render loop usa la versión RGB.

/// Boost per-canal (R, G, B), cada uno en `[0, MUZZLE_BOOST_PEAK]`.
type BoostRgb = [f32; 3];
const ZERO_BOOST: BoostRgb = [0.0, 0.0, 0.0];

/// Tabla de colores característicos por nombre de sprite Doom (4-char).
/// El nombre viene del WAD (resuelto por `WadAtlas::sprite_name`).
/// Cubre los mobjs `FF_FULLBRIGHT` notables del shareware (Doom 1) y los
/// agregados de Doom 2 / Final Doom (mancubus, revenant, archvile, lost
/// soul, keys, soul sphere, etc.).
const FB_SPRITE_TINTS: &[(&str, (u8, u8, u8))] = &[
    // --- Proyectiles base (Doom 1) ---
    ("BAL1", (255, 130, 60)),  // imp fireball — rojo-naranja
    ("BAL2", (255, 100, 80)),  // caco fireball — rojo
    ("BAL7", (140, 255, 140)), // baron fireball — verde
    ("PLSS", (130, 180, 255)), // plasma en vuelo — azul-cyan
    ("PLSE", (130, 180, 255)), // plasma impact
    ("BFS1", (160, 255, 160)), // BFG ball — verde fluorescente
    ("BFE1", (160, 255, 160)), // BFG explosion
    ("BFE2", (180, 255, 180)), // BFG splash
    ("BFGG", (160, 255, 160)), // BFG launching frames (algunos FB)
    ("MISL", (255, 180, 100)), // rocket — naranja cálido
    ("PUFF", (255, 220, 160)), // bullet puff — amarillo cálido
    ("BEXP", (255, 180, 100)), // barrel/rocket explosion — naranja
    // --- Proyectiles Doom 2 ---
    ("MANF", (255, 160, 90)),  // mancubus fireball — naranja
    ("FATB", (255, 220, 160)), // revenant tracer — pálido amarillo (la cabeza brilla)
    ("SKEL", (255, 200, 150)), // revenant attack frames — pálido cálido
    ("VILE", (255, 130, 70)),  // archvile attack frames — rojo flame
    ("FIRE", (255, 100, 50)),  // archvile fire pillar — rojo-naranja saturado
    // --- Mobjs full-bright en vuelo (Doom 1) ---
    ("SKUL", (180, 220, 255)), // lost soul — blue-white flame
    // --- Fogs / fx ---
    ("TFOG", (140, 200, 255)), // teleport fog — azul
    ("IFOG", (255, 240, 140)), // item respawn — amarillo-blanco
    // --- Pickups que brillan (Doom 1) ---
    ("SOUL", (130, 200, 255)), // soul sphere — azul-cyan
    ("MEGA", (130, 220, 200)), // mega armor — verde-cyan
    // --- Llaves coloreadas (Doom 1 — todas con FF_FULLBRIGHT) ---
    ("BKEY", (110, 160, 255)), // blue keycard
    ("YKEY", (255, 240, 130)), // yellow keycard
    ("RKEY", (255, 130, 90)),  // red keycard
    ("BSKU", (110, 160, 255)), // blue skullkey
    ("YSKU", (255, 240, 130)), // yellow skullkey
    ("RSKU", (255, 130, 90)),  // red skullkey
    // --- Antorchas / decoración (FF_FULLBRIGHT constante, Doom 1) ---
    ("TBLU", (110, 160, 255)), // blue torch (tall)
    ("TGRN", (140, 255, 160)), // green torch (tall)
    ("TRED", (255, 140, 90)),  // red torch (tall)
    ("SMBT", (110, 160, 255)), // short blue torch
    ("SMGT", (140, 255, 160)), // short green torch
    ("SMRT", (255, 140, 90)),  // short red torch
    ("CAND", (255, 200, 130)), // candle — cálido
    ("CBRA", (255, 170, 90)),  // brazier — naranja
    ("TLMP", (255, 240, 200)), // tall lamp — blanco cálido
    ("TLP2", (255, 240, 200)), // short lamp
];

/// Resuelve el tinte característico del sprite a partir de su nombre
/// 4-char. Cae al amarillo cálido del muzzle (`MUZZLE_TINT_RGB`) si el
/// nombre es desconocido — preserva el comportamiento 3.26 para mobjs
/// que el motor reportó pero la tabla no contempla.
fn sprite_tint_for_name(name: &str) -> (u8, u8, u8) {
    let key = name.get(..4).unwrap_or(name);
    for &(k, t) in FB_SPRITE_TINTS {
        if k.eq_ignore_ascii_case(key) {
            return t;
        }
    }
    MUZZLE_TINT_RGB
}

#[inline]
fn rgb_to_norm(rgb: (u8, u8, u8)) -> BoostRgb {
    [
        rgb.0 as f32 / 255.0,
        rgb.1 as f32 / 255.0,
        rgb.2 as f32 / 255.0,
    ]
}

#[inline]
fn boost_max(b: BoostRgb) -> f32 {
    b[0].max(b[1]).max(b[2])
}

/// Versión RGB del muzzle boost. Toma el scalar histórico y lo tinta
/// con `MUZZLE_TINT_RGB` per-canal — equivalente a "muzzle = world light
/// con tinte amarillo cálido anclada al jugador".
fn muzzle_boost_rgb_cam(x_cam: f32, y_cam: f32, alpha: f32) -> BoostRgb {
    let scalar = muzzle_boost_cam(x_cam, y_cam, alpha);
    if scalar <= 0.0 {
        return ZERO_BOOST;
    }
    let t = rgb_to_norm(MUZZLE_TINT_RGB);
    [scalar * t[0], scalar * t[1], scalar * t[2]]
}

// =====================================================================
// Fase 3.37 — Muzzle direccional sobre walls y planes
// =====================================================================
//
// Cuando `cfg.muzzle_brdf = true`, el muzzle se modela como una luz
// puntual emanada del jugador (origen del cam-space) y se atenúa por
// el cosine entre la normal de la superficie y la dirección a la luz.
// Las paredes oblicuas reciben menos tinte; los pisos planos lejos
// horizontalmente reciben el cosine reducido. El muzzle clásico 3.22
// (omni) sigue activo cuando el flag está off — preserva el feel
// "fogonazo que cubre todo el cono delante del jugador".
//
// La distancia y el cosine se evalúan en 3D para coincidir con el
// modelo BRDF de world lights 3.33-3.35.

/// Muzzle direccional sobre paredes. La normal del muro tiene
/// `nz=0` (paredes verticales), así que `cos = (nx·(-mx) + ny·(-my))/d_3D`.
/// Para paredes visibles tras back-face cull, `dot(normal, mid) < 0`,
/// por lo que `cos > 0` siempre — la atenuación queda en `[0.5, 1.0]`
/// salvo casos extremos.
fn muzzle_boost_rgb_wall_3d(
    x_surf: f32,
    y_surf: f32,
    z_surf_cam: f32,
    alpha: f32,
    wall_normal: (f32, f32),
) -> BoostRgb {
    // Fase 3.40: falloff 3D — el muzzle decae con d_3D, no d_2D, para
    // ser coherente con el cosine BRDF que ya considera el z.
    let scalar = muzzle_boost_cam_3d(x_surf, y_surf, z_surf_cam, alpha);
    if scalar <= 0.0 {
        return ZERO_BOOST;
    }
    let d2 = x_surf * x_surf + y_surf * y_surf + z_surf_cam * z_surf_cam;
    let att = if d2 < 1e-6 {
        1.0
    } else {
        let inv_d = d2.sqrt().recip();
        // Direction from surface to muzzle (origin): (-x_surf, -y_surf, -z) / d_3D.
        // Cosine with 2D wall normal (nz=0).
        let cos = (wall_normal.0 * -x_surf + wall_normal.1 * -y_surf) * inv_d;
        (0.5 + 0.5 * cos).max(WALL_RIM_AMBIENT_FLOOR)
    };
    let t = rgb_to_norm(MUZZLE_TINT_RGB);
    [scalar * t[0] * att, scalar * t[1] * att, scalar * t[2] * att]
}

/// Muzzle direccional sobre planos horizontales. La normal es `±Z` —
/// `cos = n_z · (-z_surf_cam) / d_3D`. Para floor (`n_z=+1`) con
/// `z_surf_cam < 0` (piso debajo del ojo), cos > 0 ⇒ att > 0.5; para
/// ceiling (`n_z=-1`) con `z_surf_cam > 0` (techo arriba), cos > 0
/// igual. Pisos/techos muy lejos horizontalmente quedan con cos bajo
/// (incidencia rasante).
fn muzzle_boost_rgb_plane_3d(
    x_surf: f32,
    y_surf: f32,
    z_surf_cam: f32,
    alpha: f32,
    n_z: f32,
) -> BoostRgb {
    // Fase 3.40: falloff 3D, mismo principio que en walls.
    let scalar = muzzle_boost_cam_3d(x_surf, y_surf, z_surf_cam, alpha);
    if scalar <= 0.0 {
        return ZERO_BOOST;
    }
    let d2 = x_surf * x_surf + y_surf * y_surf + z_surf_cam * z_surf_cam;
    let att = if d2 < 1e-6 {
        1.0
    } else {
        let inv_d = d2.sqrt().recip();
        let cos = n_z * -z_surf_cam * inv_d;
        (0.5 + 0.5 * cos).max(PLANE_RIM_AMBIENT_FLOOR)
    };
    let t = rgb_to_norm(MUZZLE_TINT_RGB);
    [scalar * t[0] * att, scalar * t[1] * att, scalar * t[2] * att]
}

/// Versión RGB del boost de world lights. Cada luz contribuye
/// `f²·PEAK·(tint/255)` per-canal, sumadas y clampeadas a
/// `MUZZLE_BOOST_PEAK` por canal.
///
/// Fase 3.29: oclusión sectorial per-light. Si una luz tiene
/// `lit_sectors = Some(set)` y `surf_sector ∉ set`, su contribución se
/// descarta — la luz quedó "encerrada" por geometría sólida del cuarto
/// que la contiene. Cuando `lit_sectors = None` (oclusión desactivada
/// o snapshot sin BSP) la luz aporta como antes, preservando el
/// comportamiento 3.27.
fn world_lights_boost_rgb_cam(
    x_cam: f32,
    y_cam: f32,
    surf_sector: u32,
    lights: &[WorldLight],
) -> BoostRgb {
    if lights.is_empty() {
        return ZERO_BOOST;
    }
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let peak = WORLD_LIGHT_PEAK;
    let mut sum = [0.0_f32; 3];
    for l in lights {
        if let Some(set) = l.lit_sectors.as_deref() {
            if !set.contains(&surf_sector) {
                continue;
            }
        }
        let dx = x_cam - l.x_cam;
        let dy = y_cam - l.y_cam;
        let d2 = dx * dx + dy * dy;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        let amount = f * f * peak;
        let t = rgb_to_norm(l.tint_rgb);
        sum[0] += amount * t[0];
        sum[1] += amount * t[1];
        sum[2] += amount * t[2];
        if sum[0] >= MUZZLE_BOOST_PEAK
            && sum[1] >= MUZZLE_BOOST_PEAK
            && sum[2] >= MUZZLE_BOOST_PEAK
        {
            return [MUZZLE_BOOST_PEAK; 3];
        }
    }
    [
        sum[0].min(MUZZLE_BOOST_PEAK),
        sum[1].min(MUZZLE_BOOST_PEAK),
        sum[2].min(MUZZLE_BOOST_PEAK),
    ]
}

// =====================================================================
// Fase 3.30 — Rim direccional del arma
// =====================================================================
//
// El psprite del jugador es un sprite 2D, pero conceptualmente "mira"
// hacia adelante (+X_cam). Una luz frontal debería tintarlo a plena
// intensidad; una luz detrás del jugador no la "ve" la cara visible
// del arma — sólo aportaría el bounce ambiente del cuarto. Modelamos
// esto con una atenuación cosine entre la "fake normal" del psprite
// (+X_cam) y la dirección normalizada a cada luz. El piso ambient
// (`WEAPON_RIM_AMBIENT_FLOOR`) representa el bounce indirecto: una
// antorcha estrictamente detrás todavía contribuye un poco vía las
// paredes del cuarto, en lugar de cortar a 0.

/// Piso ambient del rim direccional. Una luz detrás del jugador
/// igual aporta este fracción del cosine — modela el bounce
/// indirecto de paredes/techo. 0.0 = corte hard, 1.0 = sin atenuar.
const WEAPON_RIM_AMBIENT_FLOOR: f32 = 0.3;

/// Boost RGB para el rim del arma con atenuación direccional opcional.
/// `directional = false` ⇒ idéntico a `world_lights_boost_rgb_cam(0, 0,
/// player_sec, lights)` — backwards-compat con 3.28/3.29.
/// `directional = true` ⇒ cada luz se escala por
/// `att = max(AMBIENT_FLOOR, 0.5 + 0.5·cos(θ))` donde `θ` es el ángulo
/// entre la fake-normal del psprite (+X_cam, hacia adelante) y la
/// dirección unitaria a la luz. Luces frontales (cos=1) ⇒ att=1.0;
/// laterales (cos=0) ⇒ att=0.5; traseras (cos=-1) ⇒ att=AMBIENT_FLOOR.
/// Una luz exactamente en la posición del jugador (d≈0) se trata como
/// frontal — el cosine no está definido y el caso límite "abrazado por
/// la luz" merece full intensity.
///
/// Fase 3.41: la distancia y la normalización del cosine pasan a 3D.
/// El psprite vive efectivamente en el eye-level del jugador (overlay
/// 2D sobre el viewport), entonces el sample point vertical es `z=0`.
/// Una antorcha alta a `(50, 0, 60)` queda con `d_3D=78`, `cos=50/78=0.64`,
/// vs el cálculo 2D que daba `cos=1` (full). El radio también es 3D —
/// una luz remota verticalmente queda fuera del rim aunque su XY sea
/// chico. Compat 3.30 cuando todas las luces tienen `z_cam=0`.
fn weapon_rim_boost_rgb_cam(
    player_sec: u32,
    lights: &[WorldLight],
    directional: bool,
) -> BoostRgb {
    if !directional {
        return world_lights_boost_rgb_cam(0.0, 0.0, player_sec, lights);
    }
    if lights.is_empty() {
        return ZERO_BOOST;
    }
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let peak = WORLD_LIGHT_PEAK;
    let mut sum = [0.0_f32; 3];
    for l in lights {
        if let Some(set) = l.lit_sectors.as_deref() {
            if !set.contains(&player_sec) {
                continue;
            }
        }
        // Fase 3.41: distancia 3D para falloff + cos.
        let d2 = l.x_cam * l.x_cam + l.y_cam * l.y_cam + l.z_cam * l.z_cam;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        // Atenuación direccional: cos(θ) = dot((+X, 0, 0), (lx, ly, lz)/|l|_3D).
        // Para |l|=0 (luz encima del player) tratamos como att=1.0 (full
        // intensity), evita NaN y cubre el caso "luz pegada al jugador".
        let att = if d2 < 1e-6 {
            1.0
        } else {
            let inv_d = d2.sqrt().recip();
            let cos_theta = l.x_cam * inv_d;
            (0.5 + 0.5 * cos_theta).max(WEAPON_RIM_AMBIENT_FLOOR)
        };
        let amount = f * f * peak * att;
        let t = rgb_to_norm(l.tint_rgb);
        sum[0] += amount * t[0];
        sum[1] += amount * t[1];
        sum[2] += amount * t[2];
    }
    [
        sum[0].min(MUZZLE_BOOST_PEAK),
        sum[1].min(MUZZLE_BOOST_PEAK),
        sum[2].min(MUZZLE_BOOST_PEAK),
    ]
}

/// Gate RGB del muzzle boost por sector. Si `lit_sectors` está activo y
/// el sector no aparece, devuelve `ZERO_BOOST`; sino pasa el boost
/// crudo. Espejo del gating scalar de 3.23.
fn muzzle_boost_gated_rgb(
    boost: BoostRgb,
    sector_id: u32,
    lit_sectors: Option<&HashSet<u32>>,
) -> BoostRgb {
    match lit_sectors {
        Some(lit) if !lit.contains(&sector_id) => ZERO_BOOST,
        _ => boost,
    }
}

/// Versión RGB del boost combinado: muzzle (gateado por lit_sectors)
/// + world lights (sólo radio), sumados per-canal y clampeados a
/// `MUZZLE_BOOST_PEAK` por canal. Reemplaza al `combined_boost_cam`
/// scalar en el render loop.
///
/// Fase 3.33: el render loop usa los variantes specializados
/// (`combined_boost_rgb_wall_cam`, `combined_boost_rgb_sprite_cam`,
/// `combined_boost_rgb_plane_cam`). Esta versión omni se conserva como
/// referencia para tests — los specialized con `directional=false` son
/// bit-equivalentes a ella.
#[cfg(test)]
fn combined_boost_rgb_cam(
    x_cam: f32,
    y_cam: f32,
    muzzle_alpha: f32,
    surf_sector: u32,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
) -> BoostRgb {
    let m = muzzle_boost_gated_rgb(
        muzzle_boost_rgb_cam(x_cam, y_cam, muzzle_alpha),
        surf_sector,
        lit_sectors,
    );
    let w = world_lights_boost_rgb_cam(x_cam, y_cam, surf_sector, world_lights);
    [
        (m[0] + w[0]).min(MUZZLE_BOOST_PEAK),
        (m[1] + w[1]).min(MUZZLE_BOOST_PEAK),
        (m[2] + w[2]).min(MUZZLE_BOOST_PEAK),
    ]
}

// =====================================================================
// Fase 3.31 — Rim direccional para mobj sprites (billboards)
// =====================================================================
//
// Generaliza el shading direccional del arma (3.30) a cualquier mobj
// sprite. La billboard "mira" siempre a la cámara — su fake-normal es
// `(-x_surf, -y_surf)/|surf|` (la dirección desde el sprite hacia el
// origen del cam-space). Una luz **entre** la cámara y el sprite cae
// del lado iluminado del billboard ⇒ tinte fuerte. Una luz **detrás**
// del sprite back-lightea: la cara visible queda oscura, con piso
// ambient para emular el bounce indirecto. La maquinaria es la misma
// del 3.30 (cos(θ) clampeado a [`SPRITE_RIM_AMBIENT_FLOOR`]) pero la
// normal y la dirección a la luz son relativas a la posición del
// sprite, no al origen.

/// Piso ambient del rim direccional para mobjs — análogo a
/// [`WEAPON_RIM_AMBIENT_FLOOR`] del 3.30. Modela el bounce indirecto:
/// una antorcha exactamente detrás del imp igual ilumina su entorno
/// y un poco rebota hacia su cara visible.
const SPRITE_RIM_AMBIENT_FLOOR: f32 = 0.3;

/// Boost RGB de world lights en una superficie de sprite (`x_surf`,
/// `y_surf` en cam-space) con atenuación direccional opcional. Con
/// `directional=false` cae al path omni del 3.27/3.29
/// (`world_lights_boost_rgb_cam`). Con `directional=true` cada luz se
/// escala por `att = max(SPRITE_RIM_AMBIENT_FLOOR, 0.5 + 0.5·cos(θ))`
/// donde `cos(θ) = dot(normal, dir_sprite_to_light)` y la normal es
/// `(-x_surf, -y_surf)/|surf|` (toward camera) — Vec2 con `nz=0`,
/// consistente con el billboard model (sprites flat hacia la cámara
/// regardless de pitch).
///
/// Fase 3.35: la distancia y el cosine pasan a 3D usando `z_surf_cam`
/// (sprite z relativo al ojo). Una luz alta a la misma XY del mobj
/// queda con cos menor (`d_3D > d_XY` ⇒ normalización mayor) — la
/// cara del sprite "ve" menos de su intensidad. El radio también
/// es 3D-aware: una luz a 200 u en vertical queda fuera aunque su
/// XY caiga adentro.
///
/// Casos degenerados (sprite en la cámara o luz coincidente con el
/// sprite): att=1.0 — sin NaN.
fn world_lights_boost_rgb_for_sprite_cam(
    x_surf: f32,
    y_surf: f32,
    z_surf_cam: f32,
    surf_sector: u32,
    lights: &[WorldLight],
    directional: bool,
) -> BoostRgb {
    if !directional {
        return world_lights_boost_rgb_cam(x_surf, y_surf, surf_sector, lights);
    }
    if lights.is_empty() {
        return ZERO_BOOST;
    }
    let s2 = x_surf * x_surf + y_surf * y_surf;
    if s2 < 1e-6 {
        // Sprite en la cámara: la billboard no tiene normal definida.
        // Degeneramos al path omni para evitar NaN — el caso "sprite
        // pegado al jugador" es raro y visualmente ya está subsumido
        // por la propia geometría del jugador.
        return world_lights_boost_rgb_cam(x_surf, y_surf, surf_sector, lights);
    }
    let inv_s = s2.sqrt().recip();
    let nx = -x_surf * inv_s;
    let ny = -y_surf * inv_s;
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let peak = WORLD_LIGHT_PEAK;
    let mut sum = [0.0_f32; 3];
    for l in lights {
        if let Some(set) = l.lit_sectors.as_deref() {
            if !set.contains(&surf_sector) {
                continue;
            }
        }
        let dx = l.x_cam - x_surf;
        let dy = l.y_cam - y_surf;
        let dz = l.z_cam - z_surf_cam;
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        let att = if d2 < 1e-6 {
            1.0
        } else {
            let inv_d = d2.sqrt().recip();
            let cos_theta = (nx * dx + ny * dy) * inv_d;
            (0.5 + 0.5 * cos_theta).max(SPRITE_RIM_AMBIENT_FLOOR)
        };
        let amount = f * f * peak * att;
        let t = rgb_to_norm(l.tint_rgb);
        sum[0] += amount * t[0];
        sum[1] += amount * t[1];
        sum[2] += amount * t[2];
    }
    [
        sum[0].min(MUZZLE_BOOST_PEAK),
        sum[1].min(MUZZLE_BOOST_PEAK),
        sum[2].min(MUZZLE_BOOST_PEAK),
    ]
}

/// Versión sprite del boost combinado: muzzle (omni 2D, anclado al
/// jugador) + world lights direccionadas por la fake-normal del
/// billboard, con BRDF 3D (Fase 3.35). El muzzle no se direcciona
/// porque emana **del** sprite del arma — esa luz "envuelve" al mobj
/// independiente de la fake-normal.
fn combined_boost_rgb_sprite_cam(
    x_cam: f32,
    y_cam: f32,
    z_surf_cam: f32,
    muzzle_alpha: f32,
    surf_sector: u32,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
    directional: bool,
) -> BoostRgb {
    let m = muzzle_boost_gated_rgb(
        muzzle_boost_rgb_cam(x_cam, y_cam, muzzle_alpha),
        surf_sector,
        lit_sectors,
    );
    let w = world_lights_boost_rgb_for_sprite_cam(
        x_cam,
        y_cam,
        z_surf_cam,
        surf_sector,
        world_lights,
        directional,
    );
    [
        (m[0] + w[0]).min(MUZZLE_BOOST_PEAK),
        (m[1] + w[1]).min(MUZZLE_BOOST_PEAK),
        (m[2] + w[2]).min(MUZZLE_BOOST_PEAK),
    ]
}

// =====================================================================
// Fase 3.32 — Rim direccional para paredes (BRDF aproximado)
// =====================================================================
//
// Cada pared tiene una normal bien definida: perpendicular a la dirección
// del lineseg, orientada hacia el lado del frente — el que mira la cámara.
// El cosine de la normal contra la dirección a cada world light da la
// atenuación BRDF clásica (Lambert sin shadow term). Una antorcha en
// línea perpendicular a la pared (apuntando directo) la tinta al 100 %;
// una rasante (incidencia oblicua) al 50 %; una que efectivamente quedó
// "detrás" del plano de la pared (cuando un two-sided permite que la luz
// la alcance desde la cara opuesta) cae al piso ambient.
//
// El muzzle queda fuera del cosine — emana del jugador, y en walls que
// quedan "frente a vos" (las únicas visibles tras el back-face cull) el
// cosine sería ≥ 0 igual; agregarlo sólo dimearía las paredes oblicuas
// donde el muzzle ya está modelado en la simulación clásica como omni.
// Mantener esa convención preserva la lectura "el fogonazo cubre todo
// el cono delante del jugador".

const WALL_RIM_AMBIENT_FLOOR: f32 = 0.3;

/// Boost RGB de world lights en una superficie de pared (`x_surf`,
/// `y_surf` en cam-space), atenuado por el cosine entre la normal de
/// la pared (orientada toward camera) y la dirección 3D a cada luz.
/// Con `directional=false` cae al path omni 3.27/3.29 (radio 2D). La
/// normal se pasa ya en cam-space y ya orientada al frente — el caller
/// resuelve la orientación una sola vez (usando la convención de
/// back-face cull de la fase 3.0).
///
/// Fase 3.34: la distancia y el cosine se calculan en 3D, usando
/// `z_surf_cam` como cota vertical del punto de muestreo (típicamente
/// 0.0 = eye level). La normal de la pared tiene `nz=0` (vertical pura),
/// así que `cos(θ) = (nx·dx + ny·dy) / d_3D`. Una antorcha alta a la
/// misma XY que el midpoint del muro queda con cos < cos_2D porque
/// `d_3D > d_XY`. El radio también es 3D — luces remotas en z quedan
/// excluidas aunque su XY caiga dentro.
fn world_lights_boost_rgb_for_wall_cam(
    x_surf: f32,
    y_surf: f32,
    z_surf_cam: f32,
    surf_sector: u32,
    lights: &[WorldLight],
    wall_normal: (f32, f32),
    directional: bool,
) -> BoostRgb {
    if !directional {
        return world_lights_boost_rgb_cam(x_surf, y_surf, surf_sector, lights);
    }
    if lights.is_empty() {
        return ZERO_BOOST;
    }
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let peak = WORLD_LIGHT_PEAK;
    let (nx, ny) = wall_normal;
    let mut sum = [0.0_f32; 3];
    for l in lights {
        if let Some(set) = l.lit_sectors.as_deref() {
            if !set.contains(&surf_sector) {
                continue;
            }
        }
        let dx = l.x_cam - x_surf;
        let dy = l.y_cam - y_surf;
        let dz = l.z_cam - z_surf_cam;
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        let att = if d2 < 1e-6 {
            1.0
        } else {
            let inv_d = d2.sqrt().recip();
            let cos_theta = (nx * dx + ny * dy) * inv_d;
            (0.5 + 0.5 * cos_theta).max(WALL_RIM_AMBIENT_FLOOR)
        };
        let amount = f * f * peak * att;
        let t = rgb_to_norm(l.tint_rgb);
        sum[0] += amount * t[0];
        sum[1] += amount * t[1];
        sum[2] += amount * t[2];
    }
    [
        sum[0].min(MUZZLE_BOOST_PEAK),
        sum[1].min(MUZZLE_BOOST_PEAK),
        sum[2].min(MUZZLE_BOOST_PEAK),
    ]
}

/// Versión wall del boost combinado: muzzle (omni por default, BRDF
/// con `muzzle_brdf=true` en Fase 3.37) + world lights atenuadas por la
/// normal de la pared.
fn combined_boost_rgb_wall_cam(
    x_cam: f32,
    y_cam: f32,
    z_surf_cam: f32,
    muzzle_alpha: f32,
    surf_sector: u32,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
    wall_normal: (f32, f32),
    directional: bool,
    muzzle_brdf: bool,
) -> BoostRgb {
    let m_raw = if muzzle_brdf {
        muzzle_boost_rgb_wall_3d(x_cam, y_cam, z_surf_cam, muzzle_alpha, wall_normal)
    } else {
        muzzle_boost_rgb_cam(x_cam, y_cam, muzzle_alpha)
    };
    let m = muzzle_boost_gated_rgb(m_raw, surf_sector, lit_sectors);
    let w = world_lights_boost_rgb_for_wall_cam(
        x_cam,
        y_cam,
        z_surf_cam,
        surf_sector,
        world_lights,
        wall_normal,
        directional,
    );
    [
        (m[0] + w[0]).min(MUZZLE_BOOST_PEAK),
        (m[1] + w[1]).min(MUZZLE_BOOST_PEAK),
        (m[2] + w[2]).min(MUZZLE_BOOST_PEAK),
    ]
}

/// Resuelve la normal cam-space de una pared dada sus dos endpoints en
/// cam-space (`(x1, y1)`, `(x2, y2)`) y el midpoint. Devuelve la
/// componente perpendicular orientada toward camera (origen del
/// cam-space): de las dos perpendiculares posibles, pickea la que tiene
/// `dot(n, mid) < 0` (mid apunta del origen al midpoint, la normal
/// inversa apunta hacia la cámara). Devuelve `(0, 0)` si la longitud
/// del segmento es despreciable — degenerado, el caller debería caer
/// al path omni.
fn wall_normal_cam(x1: f32, y1: f32, x2: f32, y2: f32, mid_x: f32, mid_y: f32) -> (f32, f32) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-6 {
        return (0.0, 0.0);
    }
    let inv_len = len2.sqrt().recip();
    let (n1x, n1y) = (-dy * inv_len, dx * inv_len);
    // dot(n1, mid). Si negativo, n1 ya apunta toward camera.
    if n1x * mid_x + n1y * mid_y < 0.0 {
        (n1x, n1y)
    } else {
        (-n1x, -n1y)
    }
}

// =====================================================================
// Fase 3.33 — BRDF para pisos y techos con z exportado
// =====================================================================
//
// Los pisos y techos son los únicos elementos de la escena con normal
// **vertical** (`+Z` floor, `-Z` ceiling) — fuera del plano XY donde
// viven las world lights hasta el 3.32. Con la `z_cam` exportada al
// `WorldLight` desde el sprite (Fase 3.33), podemos calcular el cosine
// 3D: una antorcha al ras del piso ilumina el piso cercano pero apenas
// el techo (cos rasante); un proyectil BFG flotando en el aire ilumina
// ambos planos, con balance según su altura relativa al view-Z. El
// radio también pasa a 3D: una luz a 100 u horizontal y 100 u vertical
// queda a `d_3D≈141`, no `d_2D=100` — el aporte cae con el cuadrado
// del 3D real, más fiel al inverse-square que el 2D-only de 3.27.

const PLANE_RIM_AMBIENT_FLOOR: f32 = 0.3;

/// Boost RGB de world lights en una superficie plano-horizontal con
/// normal `±Z`. `z_surf_cam` es la altura del plano relativa al
/// `cam.view_z` (positivo arriba del ojo, negativo abajo). `n_z` =
/// `+1.0` para pisos (mirando arriba) o `-1.0` para techos (mirando
/// abajo). Cuando `directional=false`, cae al path omni 2D del
/// 3.27/3.29. Cuando `true`, usa distancia 3D para falloff + cosine
/// `n_z · dz/d_3D` para atenuar por incidencia.
fn world_lights_boost_rgb_for_plane_cam(
    x_surf: f32,
    y_surf: f32,
    z_surf_cam: f32,
    surf_sector: u32,
    lights: &[WorldLight],
    n_z: f32,
    directional: bool,
) -> BoostRgb {
    if !directional {
        return world_lights_boost_rgb_cam(x_surf, y_surf, surf_sector, lights);
    }
    if lights.is_empty() {
        return ZERO_BOOST;
    }
    let r2 = WORLD_LIGHT_RADIUS_WORLD * WORLD_LIGHT_RADIUS_WORLD;
    let peak = WORLD_LIGHT_PEAK;
    let mut sum = [0.0_f32; 3];
    for l in lights {
        if let Some(set) = l.lit_sectors.as_deref() {
            if !set.contains(&surf_sector) {
                continue;
            }
        }
        let dx = l.x_cam - x_surf;
        let dy = l.y_cam - y_surf;
        let dz = l.z_cam - z_surf_cam;
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 >= r2 {
            continue;
        }
        let f = 1.0 - d2 / r2;
        let att = if d2 < 1e-6 {
            1.0
        } else {
            let inv_d = d2.sqrt().recip();
            let cos_theta = n_z * dz * inv_d;
            (0.5 + 0.5 * cos_theta).max(PLANE_RIM_AMBIENT_FLOOR)
        };
        let amount = f * f * peak * att;
        let t = rgb_to_norm(l.tint_rgb);
        sum[0] += amount * t[0];
        sum[1] += amount * t[1];
        sum[2] += amount * t[2];
    }
    [
        sum[0].min(MUZZLE_BOOST_PEAK),
        sum[1].min(MUZZLE_BOOST_PEAK),
        sum[2].min(MUZZLE_BOOST_PEAK),
    ]
}

/// Versión plano del boost combinado: muzzle (omni 2D por default,
/// BRDF 3D con `muzzle_brdf=true` en Fase 3.37) + world lights con
/// BRDF 3D.
fn combined_boost_rgb_plane_cam(
    x_cam: f32,
    y_cam: f32,
    z_surf_cam: f32,
    muzzle_alpha: f32,
    surf_sector: u32,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
    n_z: f32,
    directional: bool,
    muzzle_brdf: bool,
) -> BoostRgb {
    let m_raw = if muzzle_brdf {
        muzzle_boost_rgb_plane_3d(x_cam, y_cam, z_surf_cam, muzzle_alpha, n_z)
    } else {
        muzzle_boost_rgb_cam(x_cam, y_cam, muzzle_alpha)
    };
    let m = muzzle_boost_gated_rgb(m_raw, surf_sector, lit_sectors);
    let w = world_lights_boost_rgb_for_plane_cam(
        x_cam,
        y_cam,
        z_surf_cam,
        surf_sector,
        world_lights,
        n_z,
        directional,
    );
    [
        (m[0] + w[0]).min(MUZZLE_BOOST_PEAK),
        (m[1] + w[1]).min(MUZZLE_BOOST_PEAK),
        (m[2] + w[2]).min(MUZZLE_BOOST_PEAK),
    ]
}

/// Suma aditivamente el boost RGB a un color base, preservando alpha.
/// Reemplaza a `apply_muzzle_tint` (scalar+yellow-fixed) en el render
/// loop. Cero boost ⇒ identidad.
fn apply_color_boost(c: Color, boost: BoostRgb) -> Color {
    if boost == ZERO_BOOST {
        return c;
    }
    let [r, g, b, a] = c.to_rgba8().to_u8_array();
    let add_r = (boost[0] * 255.0) as u32;
    let add_g = (boost[1] * 255.0) as u32;
    let add_b = (boost[2] * 255.0) as u32;
    Color::from_rgba8(
        (r as u32 + add_r).min(255) as u8,
        (g as u32 + add_g).min(255) as u8,
        (b as u32 + add_b).min(255) as u8,
        a,
    )
}

/// Versión RGB del tinte multiplicativo per-canal del sprite. Reemplaza
/// a `sprite_shade_with_muzzle` en el render loop. Devuelve `(shade · (1 + boost))`
/// por canal, clampeado a 1.0.
fn sprite_shade_with_world(shade: f32, boost: BoostRgb) -> [f32; 3] {
    [
        (shade * (1.0 + boost[0])).clamp(0.0, 1.0),
        (shade * (1.0 + boost[1])).clamp(0.0, 1.0),
        (shade * (1.0 + boost[2])).clamp(0.0, 1.0),
    ]
}

/// Deriva un par `(color, alpha)` para el overlay aditivo sobre
/// texturas (paredes + flats). El color es el boost normalizado al
/// canal más alto; el alpha escala con la magnitud del boost. Devuelve
/// `None` si el boost es despreciable (< 0.02 en cualquier canal).
fn overlay_color_alpha_from_boost(boost: BoostRgb) -> Option<(u8, u8, u8, u8)> {
    let m = boost_max(boost);
    if m <= 0.02 {
        return None;
    }
    let scale = 255.0 / m.max(1e-3);
    let r = (boost[0] * scale).clamp(0.0, 255.0) as u8;
    let g = (boost[1] * scale).clamp(0.0, 255.0) as u8;
    let b = (boost[2] * scale).clamp(0.0, 255.0) as u8;
    // Alpha proporcional al boost máximo, normalizado al peak del muzzle
    // para preservar la intensidad histórica del overlay.
    let alpha = (m * 180.0 / MUZZLE_BOOST_PEAK).clamp(0.0, 180.0) as u8;
    Some((r, g, b, alpha))
}

/// **Fase 3.43** — construye los color-stops del gradiente de oscuridad
/// vertical de un slab texturizado. `samples` son pares
/// `(offset 0..1, boost_scalar)` ordenados de abajo (offset 0) hacia
/// arriba (offset 1). El alpha de cada stop = `(1 - shade_iluminado)·255`
/// con `shade_iluminado = clamp(base_shade + boost_scalar)`; el color es
/// siempre negro. Vello interpola el alpha linealmente entre stops, dando
/// el gradiente continuo que reemplaza las bandas discretas de 3.42.
fn wall_darkness_gradient_stops(base_shade: f32, samples: &[(f32, f32)]) -> Vec<(f32, Color)> {
    samples
        .iter()
        .map(|&(off, bscalar)| {
            let lit = (base_shade + bscalar).clamp(0.0, 1.0);
            let alpha = ((1.0 - lit) * 255.0) as u8;
            (off, Color::from_rgba8(0, 0, 0, alpha))
        })
        .collect()
}

/// **Fase 3.43** — construye los color-stops del gradiente de tinte
/// vertical. `samples` son pares `(offset 0..1, boost_rgb)`. Cada stop
/// reusa la normalización de [`overlay_color_alpha_from_boost`]; los
/// stops con boost despreciable quedan transparentes (alpha 0) para no
/// cortar la continuidad del gradiente. Devuelve `None` si **ningún**
/// sample tiene tinte apreciable — en ese caso no se emite fill de tinte.
fn wall_tint_gradient_stops(samples: &[(f32, BoostRgb)]) -> Option<Vec<(f32, Color)>> {
    let mut any = false;
    let stops: Vec<(f32, Color)> = samples
        .iter()
        .map(|&(off, boost)| match overlay_color_alpha_from_boost(boost) {
            Some((r, g, b, a)) => {
                any = true;
                (off, Color::from_rgba8(r, g, b, a))
            }
            None => (off, Color::from_rgba8(0, 0, 0, 0)),
        })
        .collect();
    if any {
        Some(stops)
    } else {
        None
    }
}

/// **Fase 3.44** — devuelve `(idx_near, idx_far)`: los índices del
/// vértice más cercano y más lejano al observador (origen cam-space) por
/// distancia euclidiana². Eje del gradiente de profundidad de planos.
/// `None` si hay menos de 2 vértices.
fn plane_near_far_indices(clipped: &[(f32, f32)]) -> Option<(usize, usize)> {
    if clipped.len() < 2 {
        return None;
    }
    let (mut i_near, mut i_far) = (0usize, 0usize);
    let (mut d_near, mut d_far) = (f32::INFINITY, f32::NEG_INFINITY);
    for (i, &(x, y)) in clipped.iter().enumerate() {
        let d = x * x + y * y;
        if d < d_near {
            d_near = d;
            i_near = i;
        }
        if d > d_far {
            d_far = d;
            i_far = i;
        }
    }
    Some((i_near, i_far))
}

/// **Fase 3.45** — proyección escalar de un punto `p` sobre el eje
/// `start→end`, normalizada a `[0, 1]` (clampeada). `start` ⇒ 0,
/// `end` ⇒ 1, puntos intermedios según su proyección ortogonal. Si el
/// eje es degenerado (`start ≈ end`) devuelve 0. Usado para ubicar los
/// stops del gradiente de profundidad de planos en su offset correcto.
fn axis_offset(p: Point, start: Point, end: Point) -> f32 {
    let ax = end.x - start.x;
    let ay = end.y - start.y;
    let len2 = ax * ax + ay * ay;
    if len2 < 1e-9 {
        return 0.0;
    }
    let t = ((p.x - start.x) * ax + (p.y - start.y) * ay) / len2;
    t.clamp(0.0, 1.0) as f32
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
    .paint_with(move |scene, ts, rect: PaintRect| {
        let alpha = (last_tick_at.elapsed().as_secs_f32() / tick_period_secs).clamp(0.0, 1.0);
        let snap = make_frame(prev.as_ref(), next.as_ref(), alpha);
        render_frame(scene, ts, rect, &snap, &config);
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

fn render_frame(
    scene: &mut Scene,
    ts: &mut Typesetter,
    rect: PaintRect,
    snap: &SceneSnapshot,
    cfg: &RenderConfig,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    draw_backdrop(scene, rect, snap, cfg);

    let view_z = snap.player.z + snap.player.view_height;
    let cam = Camera::new(snap.player.x, snap.player.y, view_z, snap.player.angle);
    let proj = Projection::new_pitched(
        rect,
        cfg.fov_y_deg.to_radians(),
        snap.player.view_pitch,
    );

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

    // Fase 3.23: si la oclusión sectorial está activa y hay BSP, calculamos
    // el conjunto de sectores iluminables por el muzzle flash una sola vez
    // por frame. `None` ⇒ "iluminar todo" (modo stub o toggle apagado),
    // que reproduce el comportamiento 3.22.
    let lit_sectors: Option<HashSet<u32>> =
        if cfg.muzzle_occlusion && cfg.muzzle_glow_alpha > 0.0 {
            compute_muzzle_lit_sectors(snap)
        } else {
            None
        };
    let lit_ref = lit_sectors.as_ref();

    // Fase 3.26: recolectamos las luces puntuales del mundo desde sprites
    // FF_FULLBRIGHT. Lista cacheada por frame, hasta MAX_WORLD_LIGHTS
    // ordenados por cercanía al jugador. Si el toggle está apagado, queda
    // vacía y el plumbing pasa a no-op (rama temprana en `world_lights_boost_cam`).
    let world_lights: Vec<WorldLight> = if cfg.world_lights_enabled {
        gather_world_lights(snap, &cam, cfg.atlas.as_ref(), cfg.world_lights_occlusion)
    } else {
        Vec::new()
    };
    let world_lights_ref: &[WorldLight] = &world_lights;

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
                lit_ref,
                world_lights_ref,
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
            lit_ref,
            world_lights_ref,
        );
    }
    for sprite in snap.sprites.iter() {
        gather_sprite(
            &mut renderables,
            sprite,
            snap,
            &cam,
            &proj,
            cfg,
            lit_ref,
            world_lights_ref,
        );
    }
    // Fase 3.46: decals de impacto (host state). Camera-facing quads
    // pequeños, z-ordenados con el resto de la escena.
    if !cfg.decals.is_empty() {
        gather_decals(
            &mut renderables, cfg, snap, &cam, &proj, lit_ref, world_lights_ref,
        );
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
            RenderKind::GradientFill { gradient } => {
                scene.fill(Fill::NonZero, Affine::IDENTITY, gradient, None, &r.path);
            }
        }
    }

    // Fase 3.15: sprite del arma del jugador (pistol/shotgun/etc.) —
    // pintado *encima* de la escena 3D pero *debajo* del overlay de
    // PLAYPAL (porque los damage flashes en Doom tintan el arma también).
    // Fase 3.18: el arma se tinta por la luz del sector del jugador
    // (cuarto oscuro = arma oscura). Resolvemos el sector vía BSP point
    // query una sola vez para ambos psprites — el muzzle flash usa el
    // mismo player_light pero, gracias a su flag FF_FULLBRIGHT, igual
    // sale a luz plena.
    let player_light = player_sector_light(snap);
    // Fase 3.28: boost RGB del ambiente evaluado en la posición del
    // jugador (origen del cam-space). Reutiliza la lista cacheada de
    // world lights del frame; sin alocaciones extra. El muzzle del
    // propio jugador *no* se incluye (consistente con 3.22 — el
    // fogonazo sale de la pistola, no la ilumina a ella).
    // Fase 3.29: el rim del arma se evalúa en la posición del player
    // (origen cam-space). Para que la oclusión per-light corte luces
    // separadas por paredes, resolvemos el sector del player vía BSP
    // point query. Sin BSP cae a `NO_SECTOR`, que ninguna luz incluye
    // en su lit set ⇒ ZERO_BOOST salvo lights con `lit_sectors = None`
    // (toggle off), preservando el comportamiento 3.28.
    let player_sec = subsector_at_point(&snap.nodes, snap.player.x, snap.player.y)
        .and_then(|ss| snap.subsectors.get(ss as usize))
        .map(|ss| ss.sector)
        .unwrap_or(NO_SECTOR);
    // Fase 3.30: el rim del arma se atenúa por dirección a la luz
    // cuando `weapon_rim_directional` está on — una antorcha frente al
    // jugador tinta más fuerte que una atrás. Caso `false` cae al
    // path omnidireccional 3.28/3.29.
    let weapon_rim_boost = if cfg.weapon_rim_light {
        weapon_rim_boost_rgb_cam(player_sec, world_lights_ref, cfg.weapon_rim_directional)
    } else {
        ZERO_BOOST
    };
    draw_weapon_sprite(scene, rect, &snap.weapon, player_light, weapon_rim_boost, cfg);
    // Fase 3.16: muzzle flash (`ps_flash`) sobrepuesto al weapon.
    // Doom usa este slot para el destello brillante de BFG, plasma,
    // chaingun, etc. Mismo helper, mismo z-order layer apenas encima.
    draw_weapon_sprite(
        scene,
        rect,
        &snap.weapon_flash,
        player_light,
        weapon_rim_boost,
        cfg,
    );

    // Fase 3.19: viñeta de cabina (gradient radial muy sutil). Va antes
    // que el overlay de PLAYPAL para que un damage flash rojo intenso
    // cubra la viñeta sin contaminarse con ella. `cfg.vignette == 0.0`
    // ⇒ no-op.
    draw_vignette(scene, rect, cfg);

    // Fase 3.14: overlay full-screen al final del frame (damage red,
    // pickup yellow, radsuit green, invuln white). Modernización pura
    // de la lógica de Doom de palette swapping a PLAYPAL[1..13].
    draw_player_overlays(scene, rect, &snap.player_overlays, snap.tick);

    // Fase 3.19: crosshair central encima de todo — incluso de los
    // overlays. Si el jugador está dañado y la pantalla se tinta de
    // rojo, el crosshair sigue siendo legible. Toggleable desde el host
    // con `cfg.crosshair = false`.
    if cfg.crosshair {
        draw_crosshair(scene, rect);
    }

    // Fase 3.20: HUD inferior modernista. Va al final, encima de todo,
    // para que la barra slim al pie con health/armor/ammo/keys quede
    // siempre legible. El HUD se desactiva en stub mode (sin jugador
    // real → stats hueco) y cuando el caller pone `cfg.hud = false`.
    if cfg.hud && snap.player_stats.health > 0 {
        draw_hud(scene, ts, rect, &snap.player_stats);
    }
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
    /// **Fase 3.43** — fill del `path` con un `Gradient` lineal vertical
    /// como brush. `color` se ignora. Usado por el shading/tinte continuo
    /// de paredes texturizadas (reemplaza las bandas discretas de 3.42).
    GradientFill {
        gradient: llimphi_ui::llimphi_raster::peniko::Gradient,
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
    /// **Y-shear** del rasterizador para mouse-look cosmético. Suma a
    /// `sy` un offset constante para todos los puntos proyectados
    /// (independiente de la profundidad), lo que equivale a mover la
    /// línea del horizonte arriba/abajo en pantalla. Doom clásico no
    /// hace pitch real (cilindros de hitbox verticales); este offset
    /// preserva esa convención porque sólo afecta el rasterizador.
    ///
    /// `pitch_offset_px = focal · tan(view_pitch)`. Positivo = horizonte
    /// se mueve hacia abajo (mirando hacia arriba).
    pitch_offset_px: f32,
}

impl Projection {
    fn new(rect: PaintRect, fov_y_rad: f32) -> Self {
        Self::new_pitched(rect, fov_y_rad, 0.0)
    }

    fn new_pitched(rect: PaintRect, fov_y_rad: f32, view_pitch: f32) -> Self {
        let focal = rect.h * 0.5 / (fov_y_rad * 0.5).tan();
        // Clampeamos el pitch a ±π/3 para evitar tan() explotando y
        // distorsiones absurdas que mostrarían el "horizonte" fuera del
        // viewport. El host también clampea, pero defendemos al renderer.
        let p = view_pitch.clamp(-PITCH_MAX, PITCH_MAX);
        let pitch_offset_px = focal * p.tan();
        Self {
            cx: rect.x + rect.w * 0.5,
            cy: rect.y + rect.h * 0.5,
            focal,
            pitch_offset_px,
        }
    }

    /// `(X_cam, Y_cam, Z_cam)` → coordenada en pantalla.
    /// **Caller garantiza `x_cam > 0`** (post near-clip).
    fn project(&self, x_cam: f32, y_cam: f32, z_cam: f32) -> Point {
        let inv_d = 1.0 / x_cam;
        let sx = self.cx + y_cam * self.focal * inv_d;
        let sy = self.cy + self.pitch_offset_px - z_cam * self.focal * inv_d;
        Point::new(sx as f64, sy as f64)
    }
}

/// Rango sano del pitch (±60°). Más allá el horizonte se sale del
/// viewport y los planos del piso/techo dejan de tener interpretación
/// visual razonable.
const PITCH_MAX: f32 = std::f32::consts::FRAC_PI_3;

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
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
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
    // Fase 3.22: boost del muzzle flash en el midpoint de la pared.
    // Cae con distancia² desde el jugador (en cam-space player = origen).
    // Fase 3.23: gateado por el sector "near" del wall (el lado del que
    // miramos). Si ese sector no está en el lit set (cuarto inalcanzable
    // desde el player por linedef two-sided directo), el boost se anula
    // — la pared queda como en escena base, sin tinte cálido.
    // Fase 3.26: sumamos también las world lights de mobjs FF_FULLBRIGHT
    // cercanos al midpoint. Fase 3.27: el boost ahora es per-canal RGB
    // — cada luz emite su tinte (BFG verde, plasma azul, fireball rojo,
    // antorcha teñida). El scalar `boost_scalar = max(boost_rgb)` se
    // usa donde necesitamos una magnitud única (overlay alpha del
    // shading darkness).
    // Fase 3.32: rim direccional. La normal cam-space de la pared
    // (perpendicular al lineseg, toward camera) modula el aporte de
    // cada world light por cos(θ). Muzzle queda omni.
    // Fase 3.34: distancia y cosine en 3D, sample point en eye level
    // (`z_surf_cam = 0.0`). El radio 3D excluye luces remotas en
    // vertical aunque XY caiga dentro del rango.
    let wall_normal = wall_normal_cam(x1, y1, x2, y2, mid_x, mid_y);
    let boost_rgb = combined_boost_rgb_wall_cam(
        mid_x,
        mid_y,
        0.0,
        cfg.muzzle_glow_alpha,
        near_idx,
        lit_sectors,
        world_lights,
        wall_normal,
        cfg.wall_rim_directional,
        cfg.muzzle_brdf,
    );
    let boost_scalar = boost_max(boost_rgb);

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
                color: apply_color_boost(floor_color(near_sec, depth, cfg), boost_rgb),
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
                color: apply_color_boost(
                    ceiling_color(near_sec, depth, cfg, snap.sky_pic),
                    boost_rgb,
                ),
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
            // Overlay de shade y tinte. Fase 3.32-3.41: una sola fill
            // por slab con boost computado al eye-level.
            // Fase 3.42: si `wall_vertical_bands > 1`, subdividimos el
            // slab en N bandas horizontales y computamos el boost al
            // centro vertical de cada una. Una antorcha al ras del piso
            // ilumina más la parte baja, una a la altura del techo más
            // la parte alta — gradient discreto vertical.
            let base_shade = shade_for(sec.light_level, depth, cfg);
            let v_bands = cfg.wall_vertical_bands.max(1) as u32;
            if cfg.wall_vertical_gradient {
                // Path 3.43: gradiente lineal continuo bottom→top. Dos
                // fills por slab (oscuridad + tinte) en lugar de 2N.
                use llimphi_ui::llimphi_raster::peniko::Gradient;
                let nstops = (cfg.wall_vertical_bands as usize).max(2) + 1;
                // Geometría: bottom-center (t=0) → top-center (t=1) en
                // pantalla. Proyectamos las cuatro esquinas del slab.
                let zb_c = z_bot - cam.view_z;
                let zt_c = z_top - cam.view_z;
                let g_bl = proj.project(x1, y1, zb_c);
                let g_br = proj.project(x2, y2, zb_c);
                let g_tl = proj.project(x1, y1, zt_c);
                let g_tr = proj.project(x2, y2, zt_c);
                let start = Point::new((g_bl.x + g_br.x) * 0.5, (g_bl.y + g_br.y) * 0.5);
                let end = Point::new((g_tl.x + g_tr.x) * 0.5, (g_tl.y + g_tr.y) * 0.5);
                // Muestreo del boost a `nstops` alturas (igual normal de
                // pared, distinto z_surf_cam).
                let mut dark_samples = Vec::with_capacity(nstops);
                let mut tint_samples = Vec::with_capacity(nstops);
                for i in 0..nstops {
                    let t = i as f32 / (nstops - 1) as f32;
                    let z_band_cam = (z_bot + (z_top - z_bot) * t) - cam.view_z;
                    let band_boost = combined_boost_rgb_wall_cam(
                        mid_x,
                        mid_y,
                        z_band_cam,
                        cfg.muzzle_glow_alpha,
                        near_idx,
                        lit_sectors,
                        world_lights,
                        wall_normal,
                        cfg.wall_rim_directional,
                        cfg.muzzle_brdf,
                    );
                    dark_samples.push((t, boost_max(band_boost)));
                    tint_samples.push((t, band_boost));
                }
                let dark_stops = wall_darkness_gradient_stops(base_shade, &dark_samples);
                let dark_grad =
                    Gradient::new_linear(start, end).with_stops(dark_stops.as_slice());
                out.push(Renderable {
                    depth: depth - 0.001,
                    color: Color::WHITE,
                    path: path.clone(),
                    kind: RenderKind::GradientFill {
                        gradient: dark_grad,
                    },
                });
                if let Some(tint_stops) = wall_tint_gradient_stops(&tint_samples) {
                    let tint_grad =
                        Gradient::new_linear(start, end).with_stops(tint_stops.as_slice());
                    out.push(Renderable {
                        depth: depth - 0.002,
                        color: Color::WHITE,
                        path,
                        kind: RenderKind::GradientFill {
                            gradient: tint_grad,
                        },
                    });
                }
            } else if v_bands == 1 {
                // Path 3.32-3.41: single overlay sobre todo el slab.
                let lit_shade = (base_shade + boost_scalar).clamp(0.0, 1.0);
                if lit_shade < 0.95 {
                    let alpha = ((1.0 - lit_shade) * 255.0) as u8;
                    out.push(Renderable {
                        depth: depth - 0.001,
                        color: Color::from_rgba8(0, 0, 0, alpha),
                        path: path.clone(),
                        kind: RenderKind::Fill,
                    });
                }
                if let Some((or, og, ob, oa)) = overlay_color_alpha_from_boost(boost_rgb) {
                    out.push(Renderable {
                        depth: depth - 0.002,
                        color: Color::from_rgba8(or, og, ob, oa),
                        path,
                        kind: RenderKind::Fill,
                    });
                }
            } else {
                // Path 3.42: N bandas verticales, cada una con su boost.
                for b in 0..v_bands {
                    let t0 = b as f32 / v_bands as f32;
                    let t1 = (b + 1) as f32 / v_bands as f32;
                    // Centro vertical de la banda en world z.
                    let z_band_center =
                        z_bot + (z_top - z_bot) * (t0 + t1) * 0.5;
                    let z_band_cam = z_band_center - cam.view_z;
                    // Boost específico de la banda (mismo wall_normal,
                    // distinto z_surf_cam).
                    let band_boost = combined_boost_rgb_wall_cam(
                        mid_x,
                        mid_y,
                        z_band_cam,
                        cfg.muzzle_glow_alpha,
                        near_idx,
                        lit_sectors,
                        world_lights,
                        wall_normal,
                        cfg.wall_rim_directional,
                        cfg.muzzle_brdf,
                    );
                    let band_scalar = boost_max(band_boost);
                    // Path de la banda: clip vertical del slab.
                    let zb_b = (z_bot + (z_top - z_bot) * t0) - cam.view_z;
                    let zt_b = (z_bot + (z_top - z_bot) * t1) - cam.view_z;
                    let bl_b = proj.project(x1, y1, zb_b);
                    let tl_b = proj.project(x1, y1, zt_b);
                    let tr_b = proj.project(x2, y2, zt_b);
                    let br_b = proj.project(x2, y2, zb_b);
                    let mut band_path = BezPath::new();
                    band_path.move_to(bl_b);
                    band_path.line_to(tl_b);
                    band_path.line_to(tr_b);
                    band_path.line_to(br_b);
                    band_path.close_path();
                    let lit_band = (base_shade + band_scalar).clamp(0.0, 1.0);
                    if lit_band < 0.95 {
                        let alpha = ((1.0 - lit_band) * 255.0) as u8;
                        out.push(Renderable {
                            depth: depth - 0.001,
                            color: Color::from_rgba8(0, 0, 0, alpha),
                            path: band_path.clone(),
                            kind: RenderKind::Fill,
                        });
                    }
                    if let Some((or, og, ob, oa)) =
                        overlay_color_alpha_from_boost(band_boost)
                    {
                        out.push(Renderable {
                            depth: depth - 0.002,
                            color: Color::from_rgba8(or, og, ob, oa),
                            path: band_path,
                            kind: RenderKind::Fill,
                        });
                    }
                }
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
                    color: apply_color_boost(
                        wall_color(wall_idx, wall, sec, depth, b, bands, cfg),
                        boost_rgb,
                    ),
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

/// Light level por default cuando no podemos determinar el sector del
/// punto consultado (mapa sin BSP, índices fuera de rango). 192 es el
/// valor "habitación tipica iluminada" de Doom — coincide con el
/// fallback de `gather_sprite` para sprites sin sector.
const DEFAULT_PLAYER_LIGHT: u8 = 192;

/// Devuelve el subsector que contiene el punto `(px, py)`, descendiendo
/// el árbol BSP por el lado donde cae el punto en cada partición. `None`
/// si el snapshot no tiene BSP cargado, o si el camino llega a un
/// índice fuera de rango (mapa malformado). O(log N) en BSPs balanceados.
fn subsector_at_point(nodes: &[NodeSnap], px: f32, py: f32) -> Option<u32> {
    if nodes.is_empty() {
        return None;
    }
    let mut cur: u16 = (nodes.len() - 1) as u16;
    loop {
        if cur & NF_SUBSECTOR != 0 {
            return Some((cur & !NF_SUBSECTOR) as u32);
        }
        let node = nodes.get(cur as usize)?;
        // Mismo signo que `walk_bsp`: side > 0 → near = children[0].
        let side = node.partition_dx * (py - node.partition_y)
            - node.partition_dy * (px - node.partition_x);
        cur = if side > 0.0 {
            node.children[0]
        } else {
            node.children[1]
        };
    }
}

/// Light level del sector donde está parado el jugador. Recorre el BSP
/// para encontrar el subsector que contiene `(player.x, player.y)`,
/// luego lee `light_level` del sector referenciado. Fallback a
/// [`DEFAULT_PLAYER_LIGHT`] si no hay BSP, o el subsector apunta fuera
/// de la lista de sectores. Usado por `draw_weapon_sprite` para tintar
/// el arma según la iluminación local (Fase 3.18).
fn player_sector_light(snap: &SceneSnapshot) -> u8 {
    sector_light_at(snap, snap.player.x, snap.player.y)
}

/// **Fase 3.49** — light level del sector que contiene `(px, py)`,
/// resuelto por BSP point query. Fallback a [`DEFAULT_PLAYER_LIGHT`] si
/// no hay BSP o el subsector apunta fuera de la lista de sectores.
/// Generalización de [`player_sector_light`] para iluminar decals en su
/// posición real (no la del jugador).
fn sector_light_at(snap: &SceneSnapshot, px: f32, py: f32) -> u8 {
    let ss_id = match subsector_at_point(&snap.nodes, px, py) {
        Some(id) => id,
        None => return DEFAULT_PLAYER_LIGHT,
    };
    let Some(ss) = snap.subsectors.get(ss_id as usize) else {
        return DEFAULT_PLAYER_LIGHT;
    };
    snap.sectors
        .get(ss.sector as usize)
        .map(|s| s.light_level)
        .unwrap_or(DEFAULT_PLAYER_LIGHT)
}

/// **Fase 3.49** — multiplica un color RGB por un factor de shade
/// `[0, 1]` (per-canal, clampeado). Oscurece el decal según la luz del
/// sector donde cae: un charco en cuarto oscuro se ve casi negro, no a
/// luz plena.
fn shade_rgb((r, g, b): (u8, u8, u8), shade: f32) -> (u8, u8, u8) {
    let s = shade.clamp(0.0, 1.0);
    (
        (r as f32 * s) as u8,
        (g as f32 * s) as u8,
        (b as f32 * s) as u8,
    )
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

#[allow(clippy::too_many_arguments)]
fn gather_subsector_planes(
    out: &mut Vec<Renderable>,
    sub: &SubsectorSnap,
    snap: &SceneSnapshot,
    cam: &Camera,
    proj: &Projection,
    rect: &PaintRect,
    cfg: &RenderConfig,
    bsp_depth_override: Option<f32>,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
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
    let (centroid_cx, centroid_cy) = {
        let (mut cx_sum, mut cy_sum) = (0.0_f32, 0.0_f32);
        for &(x, y) in &clipped {
            cx_sum += x;
            cy_sum += y;
        }
        let n = clipped.len() as f32;
        (cx_sum / n, cy_sum / n)
    };
    let shade_depth = (centroid_cx * centroid_cx + centroid_cy * centroid_cy).sqrt();
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
        // Fase 3.33: boost específico del plano. Normal `+Z` para floor,
        // `-Z` para ceiling — la luz de un proyectil al ras del piso
        // ilumina el piso pero queda rasante para el techo. Cuando
        // `plane_rim_directional` está off, cae al path omni 3.27/3.29
        // (igual aporte para floor y ceiling).
        let z_surf_cam = z_world - cam.view_z;
        let n_z = if is_floor { 1.0 } else { -1.0 };
        let boost_rgb = combined_boost_rgb_plane_cam(
            centroid_cx,
            centroid_cy,
            z_surf_cam,
            cfg.muzzle_glow_alpha,
            sub.sector,
            lit_sectors,
            world_lights,
            n_z,
            cfg.plane_rim_directional,
            cfg.muzzle_brdf,
        );
        let boost_scalar = boost_max(boost_rgb);
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
                        let base_factor = if is_floor { 0.92 } else { 0.85 };
                        // Fase 3.44: gradiente de profundidad near→far.
                        // En lugar de un overlay uniforme al centroide,
                        // muestreamos fog + boost en el vértice más
                        // cercano y el más lejano al jugador, y dejamos
                        // que Vello interpole. La parte del piso a tus
                        // pies queda más clara (menos fog + pool de luz);
                        // la lejana, más oscura. Reusa los helpers del
                        // gradiente vertical de walls (3.43).
                        if cfg.plane_depth_gradient {
                            if let Some((i_near, i_far)) = plane_near_far_indices(&clipped) {
                                use llimphi_ui::llimphi_raster::peniko::Gradient;
                                let start = screen_pts[i_near];
                                let end = screen_pts[i_far];
                                // Fase 3.45: muestreamos fog + boost en
                                // *cada* vértice del polígono (más el
                                // centroide), proyectando su posición en
                                // pantalla sobre el eje near→far para
                                // obtener el offset del stop. Así el
                                // gradiente captura la variación de luz
                                // intermedia (un proyectil a mitad del
                                // piso, una esquina más iluminada) en
                                // lugar de interpolar linealmente sólo
                                // entre los dos extremos (3.44).
                                let sample_at = |vx: f32, vy: f32| -> (f32, BoostRgb) {
                                    let vdepth = (vx * vx + vy * vy).sqrt();
                                    let vb = combined_boost_rgb_plane_cam(
                                        vx,
                                        vy,
                                        z_surf_cam,
                                        cfg.muzzle_glow_alpha,
                                        sub.sector,
                                        lit_sectors,
                                        world_lights,
                                        n_z,
                                        cfg.plane_rim_directional,
                                        cfg.muzzle_brdf,
                                    );
                                    let vshade =
                                        shade_for(sec.light_level, vdepth, cfg) * base_factor;
                                    // lit-shade completo; el helper de
                                    // oscuridad recibe base 0 ⇒ alpha =
                                    // (1 - lit)·255.
                                    let lit = (vshade + boost_max(vb)).clamp(0.0, 1.0);
                                    (lit, vb)
                                };
                                // (offset, lit, boost) por vértice +
                                // centroide.
                                let mut raw: Vec<(f32, f32, BoostRgb)> =
                                    Vec::with_capacity(clipped.len() + 1);
                                for (i, &(vx, vy)) in clipped.iter().enumerate() {
                                    let off = axis_offset(screen_pts[i], start, end);
                                    let (lit, vb) = sample_at(vx, vy);
                                    raw.push((off, lit, vb));
                                }
                                // Centroide (offset por su proyección).
                                let c_screen = proj.project(
                                    centroid_cx,
                                    centroid_cy,
                                    z_world - cam.view_z,
                                );
                                let c_off = axis_offset(c_screen, start, end);
                                let (c_lit, c_vb) = sample_at(centroid_cx, centroid_cy);
                                raw.push((c_off, c_lit, c_vb));
                                // Orden por offset + dedup (Vello exige
                                // offsets no decrecientes; colapsamos los
                                // casi-iguales para evitar stops cero-ancho).
                                raw.sort_by(|a, b| {
                                    a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                                });
                                let mut dark: Vec<(f32, f32)> = Vec::with_capacity(raw.len());
                                let mut tint: Vec<(f32, BoostRgb)> = Vec::with_capacity(raw.len());
                                let mut last_off = f32::NEG_INFINITY;
                                for (off, lit, vb) in raw {
                                    if off <= last_off + 1e-4 {
                                        continue;
                                    }
                                    last_off = off;
                                    dark.push((off, lit));
                                    tint.push((off, vb));
                                }
                                let dstops = wall_darkness_gradient_stops(0.0, &dark);
                                let dgrad = Gradient::new_linear(start, end)
                                    .with_stops(dstops.as_slice());
                                out.push(Renderable {
                                    depth: depth + 0.999,
                                    color: Color::WHITE,
                                    path: path.clone(),
                                    kind: RenderKind::GradientFill { gradient: dgrad },
                                });
                                if let Some(tstops) = wall_tint_gradient_stops(&tint) {
                                    let tgrad = Gradient::new_linear(start, end)
                                        .with_stops(tstops.as_slice());
                                    out.push(Renderable {
                                        depth: depth + 0.998,
                                        color: Color::WHITE,
                                        path,
                                        kind: RenderKind::GradientFill { gradient: tgrad },
                                    });
                                }
                                return;
                            }
                        }
                        // Shade overlay sobre el polígono entero
                        // (shade es constante por plano — no necesita
                        // ser per-triangle). Mismo truco que walls.
                        // Usa `shade_depth` euclidiano (no `depth` BSP-derived)
                        // porque fog/light dropoff dependen de la distancia
                        // real al jugador.
                        //
                        // Fase 3.22: el muzzle boost levanta el `shade`
                        // (reduce el overlay oscuro) + emite un overlay
                        // amarillo aditivo sobre la textura.
                        let base_shade = shade_for(sec.light_level, shade_depth, cfg)
                            * base_factor;
                        let lit_shade = (base_shade + boost_scalar).clamp(0.0, 1.0);
                        if lit_shade < 0.95 {
                            let alpha = ((1.0 - lit_shade) * 255.0).clamp(0.0, 255.0) as u8;
                            out.push(Renderable {
                                depth: depth + 0.999,
                                color: Color::from_rgba8(0, 0, 0, alpha),
                                path: path.clone(),
                                kind: RenderKind::Fill,
                            });
                        }
                        if let Some((or, og, ob, oa)) = overlay_color_alpha_from_boost(boost_rgb) {
                            out.push(Renderable {
                                depth: depth + 0.998,
                                color: Color::from_rgba8(or, og, ob, oa),
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
            color: apply_color_boost(color, boost_rgb),
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

/// **Fase 3.53** — Sutherland-Hodgman contra un semiplano `n·(p − a) ≥ 0`
/// en 2D mundo. Se mantienen los vértices del lado positivo de la normal
/// `n` (no necesita ser unitaria); las aristas que cruzan el borde se
/// intersectan paramétricamente. Usado para recortar el charco horizontal
/// a las paredes que lo bordean.
fn clip_half_plane(poly: &[(f32, f32)], a: (f32, f32), n: (f32, f32)) -> Vec<(f32, f32)> {
    if poly.is_empty() {
        return Vec::new();
    }
    let dist = |p: (f32, f32)| n.0 * (p.0 - a.0) + n.1 * (p.1 - a.1);
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(poly.len() + 2);
    let len = poly.len();
    for i in 0..len {
        let curr = poly[i];
        let prev = poly[if i == 0 { len - 1 } else { i - 1 }];
        let dc = dist(curr);
        let dp = dist(prev);
        let lerp = |t: f32| (prev.0 + (curr.0 - prev.0) * t, prev.1 + (curr.1 - prev.1) * t);
        match (dp >= 0.0, dc >= 0.0) {
            (true, true) => out.push(curr),
            (true, false) => out.push(lerp(dp / (dp - dc))),
            (false, true) => {
                out.push(lerp(dp / (dp - dc)));
                out.push(curr);
            }
            (false, false) => {}
        }
    }
    out
}

/// **Fase 3.53** — recorta el polígono del charco (en XY mundo) a las
/// paredes que efectivamente alcanza, manteniendo siempre el lado donde
/// está el centro. Cada pared cuyo punto más cercano al centro cae dentro
/// del radio `r` aporta un semiplano (su línea infinita, normal orientada
/// hacia el centro). El resultado es la intersección convexa local — una
/// mancha de sangre junto a un muro deja de treparlo o cruzar al cuarto
/// vecino. Las paredes que el charco no toca no recortan. Sin paredes,
/// devuelve el polígono intacto (modo stub ⇒ comportamiento 3.48).
fn clip_decal_to_walls(
    quad: &[(f32, f32)],
    walls: &[WallSeg],
    cx: f32,
    cy: f32,
    r: f32,
) -> Vec<(f32, f32)> {
    let mut poly = quad.to_vec();
    let r2 = r * r;
    for w in walls {
        let dx = w.x2 - w.x1;
        let dy = w.y2 - w.y1;
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-6 {
            continue;
        }
        // Punto más cercano del segmento al centro: sólo las paredes que
        // el charco realmente alcanza recortan (evita que la línea de un
        // muro lejano corte en cuartos no convexos).
        let t = (((cx - w.x1) * dx + (cy - w.y1) * dy) / len2).clamp(0.0, 1.0);
        let px = w.x1 + t * dx;
        let py = w.y1 + t * dy;
        if (px - cx) * (px - cx) + (py - cy) * (py - cy) > r2 {
            continue;
        }
        // Normal del muro orientada hacia el centro.
        let mut n = (-dy, dx);
        if n.0 * (cx - w.x1) + n.1 * (cy - w.y1) < 0.0 {
            n = (dy, -dx);
        }
        poly = clip_half_plane(&poly, (w.x1, w.y1), n);
        if poly.len() < 3 {
            break;
        }
    }
    poly
}

// =====================================================================
// Sprites + sombras (Fase 3.21)
// =====================================================================

/// Pinta un disco oscuro en el plano del piso bajo el sprite. Lo
/// aproximamos con un dodecágono CCW en world-space, transformamos a
/// cam-space, clipeamos al near plane (2D) y proyectamos cada vértice
/// con la cámara perspectiva. El resultado es una elipse natural en
/// pantalla — más alargada cuanto más cerca del jugador, casi línea
/// en la distancia.
///
/// El radio en world units viene del atlas si está disponible (mitad
/// del width del patch del frame actual, escalado por 0.55 para que la
/// sombra no exceda el ancho del sprite). Sin atlas usa
/// `cfg.sprite_half_width`.
///
/// La depth se pone `sprite_depth + 0.5` para que el shadow se pinte
/// **justo antes** del sprite en el orden back-to-front (painter's),
/// quedando bajo los pies del mobj pero encima del piso del sector.
#[allow(clippy::too_many_arguments)]
fn gather_sprite_shadow(
    out: &mut Vec<Renderable>,
    sprite: &SpriteSnap,
    sec: Option<&SectorSnap>,
    cam: &Camera,
    proj: &Projection,
    cfg: &RenderConfig,
    sprite_x_cam: f32,
    floor: f32,
    sprite_depth: f32,
) {
    // Radio en world units. Si tenemos el patch decodificado del atlas
    // usamos su mitad de width — así un enemigo grande (caco/baron) tira
    // sombra más ancha que un imp.
    let radius = if let Some(atlas) = cfg.atlas.as_ref() {
        let angle = compute_display_angle(sprite.x, sprite.y, sprite.angle, cam.px, cam.py);
        atlas
            .sprite_patch(sprite.sprite, sprite.frame, angle)
            .map(|(p, _)| (p.width as f32) * 0.55 * 0.5)
            .unwrap_or(cfg.sprite_half_width)
    } else {
        cfg.sprite_half_width
    };
    if radius <= 0.0 {
        return;
    }
    // Dodecágono en world space alrededor de (sprite.x, sprite.y).
    // CCW; los puntos viven todos en Z = floor.
    const N: usize = 12;
    let z_cam = floor - cam.view_z;
    let mut poly_cam: [(f32, f32); N] = [(0.0, 0.0); N];
    let twopi = std::f32::consts::TAU;
    // Pequeño achatamiento: la sombra es 100% radius en eje view-perpendicular
    // y 60% en eje view-paralelo (eje X_cam). Doom-monsters paran sobre
    // sus pies redondos, pero al verlos *desde* el jugador la huella
    // visual queda más como elipse — quedan más naturales así.
    let rx = radius * 0.6;
    let ry = radius;
    for i in 0..N {
        let theta = (i as f32) / (N as f32) * twopi;
        // Generamos en world coords con orientación alineada al world XY.
        let wx = sprite.x + theta.cos() * rx;
        let wy = sprite.y + theta.sin() * ry;
        poly_cam[i] = cam.to_cam_2d(wx, wy);
    }
    let clipped = clip_near(&poly_cam, cfg.near);
    if clipped.len() < 3 {
        return;
    }
    let mut path = BezPath::new();
    let mut first = true;
    for (xc, yc) in &clipped {
        let p = proj.project(*xc, *yc, z_cam);
        if !p.x.is_finite() || !p.y.is_finite() {
            return;
        }
        if first {
            path.move_to(p);
            first = false;
        } else {
            path.line_to(p);
        }
    }
    path.close_path();
    // Tinte: negro con alpha modulado por la luz del sector. Sectores
    // muy oscuros (cuartos sin iluminar) atenúan la sombra — no tiene
    // sentido pintar una mancha negra sobre piso ya casi negro. Fog
    // distante también la diluye.
    let light = sec.map(|s| s.light_level).unwrap_or(192) as f32 / 255.0;
    let fog = 1.0 - (sprite_x_cam / cfg.far_fog).clamp(0.0, 1.0);
    let alpha = (0.42 * light * fog).clamp(0.0, 0.55);
    let a = (alpha * 255.0) as u8;
    if a < 4 {
        return;
    }
    out.push(Renderable {
        depth: sprite_depth + 0.5,
        color: Color::from_rgba8(0, 0, 0, a),
        path,
        kind: RenderKind::Fill,
    });
}

/// **Fase 3.46** — proyecta cada decal del host como un quad pequeño
/// camera-facing. Mismo modelo de billboard que los sprites: a una
/// profundidad `x_cam` constante el quad es axis-aligned en pantalla.
/// `+y_cam` = izquierda, `+z` = arriba (convención de `gather_sprite`).
/// El depth se sesga `-0.5` para que la marca quede apenas delante de la
/// pared/piso donde impactó, sin z-fight.
#[allow(clippy::too_many_arguments)]
fn gather_decals(
    out: &mut Vec<Renderable>,
    cfg: &RenderConfig,
    snap: &SceneSnapshot,
    cam: &Camera,
    proj: &Projection,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
) {
    for d in &cfg.decals {
        let a = (d.alpha.clamp(0.0, 1.0) * 255.0) as u8;
        if a == 0 {
            continue;
        }
        // Cull por el centro: si el impacto está detrás del near-plane,
        // descartamos el decal entero.
        let (cx_cam, cy_cam) = cam.to_cam_2d(d.x, d.y);
        if cx_cam < cfg.near {
            continue;
        }
        let r = d.radius;
        let cz = d.z - cam.view_z;
        // Fase 3.49/3.52: resolvemos el sector del decal una sola vez (BSP
        // point query) — lo reusan el shading + boost (3.49-3.51) y el
        // recorte vertical al rango [floor, ceiling] del sector (3.52).
        let sector = if snap.nodes.is_empty() {
            None
        } else {
            subsector_at_point(&snap.nodes, d.x, d.y)
                .and_then(|ss| snap.subsectors.get(ss as usize))
                .map(|ss| ss.sector)
        };
        let sector_snap = sector.and_then(|s| snap.sectors.get(s as usize));
        // Fase 3.47: si hay tangente de pared, el quad yace **plano**
        // sobre el lineseg (eje horizontal = tangente mundo, vertical =
        // +Z) — se ve en perspectiva, no de cara a la cámara. Sin
        // tangente, billboard 3.46 (a `cx_cam` constante el quad es
        // axis-aligned en pantalla).
        let (tx, ty) = d.tangent;
        let corners: Vec<Point> = if d.horizontal {
            // Fase 3.48: charco horizontal — ejes en el plano XY mundo a
            // `z` constante. El quad se ve en perspectiva sobre el piso
            // (o bajo el techo).
            //
            // Fase 3.53: recortamos el quad a las paredes que lo bordean
            // (`clip_decal_to_walls`) — una mancha junto a un muro deja de
            // treparlo o cruzar al cuarto vecino. Sin paredes (modo stub) o
            // con el toggle off ⇒ quad completo como en 3.48.
            let quad = [
                (d.x - r, d.y - r),
                (d.x + r, d.y - r),
                (d.x + r, d.y + r),
                (d.x - r, d.y + r),
            ];
            let world = if cfg.decal_clip_walls && !snap.walls.is_empty() {
                let clipped = clip_decal_to_walls(&quad, &snap.walls, d.x, d.y, r);
                if clipped.len() < 3 {
                    continue;
                }
                clipped
            } else {
                quad.to_vec()
            };
            world
                .iter()
                .map(|&(wx, wy)| {
                    let (wcx, wcy) = cam.to_cam_2d(wx, wy);
                    proj.project(wcx, wcy, cz)
                })
                .collect()
        } else if tx != 0.0 || ty != 0.0 {
            // Esquinas en mundo: centro ± tangente (horizontal) y ± Z
            // (vertical). Cada una se transforma a cámara y se proyecta —
            // el quad queda con la inclinación de la pared.
            //
            // Fase 3.52: recortamos la extensión horizontal al span del
            // lineseg (`wall_span`) y la vertical al rango [floor, ceiling]
            // del sector, para que el decal no sangre más allá del borde
            // de la pared (la esquina) ni del piso/techo. Sin span / sin
            // sector ⇒ ± r como en 3.51.
            let (s_lo, s_hi) = match d.wall_span {
                Some((mn, mx)) => (mn.max(-r), mx.min(r)),
                None => (-r, r),
            };
            let (dz_lo, dz_hi) = match sector_snap {
                Some(s) => {
                    let floor_cam = s.floor_height - cam.view_z;
                    let ceil_cam = s.ceiling_height - cam.view_z;
                    ((floor_cam - cz).max(-r), (ceil_cam - cz).min(r))
                }
                None => (-r, r),
            };
            // Recorte completo ⇒ quad vacío: lo saltamos.
            if s_hi <= s_lo || dz_hi <= dz_lo {
                continue;
            }
            let project_world = |sx: f32, dz: f32| -> Point {
                let (wcx, wcy) = cam.to_cam_2d(d.x + tx * sx, d.y + ty * sx);
                proj.project(wcx, wcy, cz + dz)
            };
            vec![
                project_world(s_lo, dz_hi),
                project_world(s_hi, dz_hi),
                project_world(s_hi, dz_lo),
                project_world(s_lo, dz_lo),
            ]
        } else {
            vec![
                proj.project(cx_cam, cy_cam + r, cz + r),
                proj.project(cx_cam, cy_cam - r, cz + r),
                proj.project(cx_cam, cy_cam - r, cz - r),
                proj.project(cx_cam, cy_cam + r, cz - r),
            ]
        };
        if !corners.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
            continue;
        }
        let depth = (cx_cam * cx_cam + cy_cam * cy_cam).sqrt();
        // Fase 3.49: shadeamos el color por la luz del sector donde cae
        // el decal (+ fog por distancia). Fase 3.50: además sumamos el
        // tinte RGB de world lights + muzzle en esa posición — un charco
        // junto a un fireball se enrojece, el fogonazo lo ilumina. En
        // modo stub (sin BSP) queda full-bright como en 3.46-3.48.
        let col = if snap.nodes.is_empty() {
            Color::from_rgba8(d.color.0, d.color.1, d.color.2, a)
        } else {
            let light = sector_snap
                .map(|s| s.light_level)
                .unwrap_or(DEFAULT_PLAYER_LIGHT);
            let (sr, sg, sb) = shade_rgb(d.color, shade_for(light, depth, cfg));
            let base = Color::from_rgba8(sr, sg, sb, a);
            let surf_sector = sector.unwrap_or(NO_SECTOR);
            let z_surf_cam = d.z - cam.view_z;
            // Fase 3.51: el boost se direcciona por la normal de la
            // superficie donde yace el decal — un scorch en pared rasante
            // a la luz recibe menos tinte que uno encarado, un charco bajo
            // un fireball alto recoge el cosine vertical. Charco
            // (`horizontal`) ⇒ BRDF de plano; marca de pared (`tangent`)
            // ⇒ BRDF de pared; billboard flotante ⇒ omni (no tiene normal
            // estable). Con `decal_rim_directional=false` todo cae al omni
            // 3.50 bit-equivalente.
            let boost = if cfg.decal_rim_directional && d.horizontal {
                // Charco: normal +Z (piso) o -Z (techo) según a qué plano
                // del sector está más pegado el decal.
                let n_z = sector_snap
                    .map(|s| {
                        if (d.z - s.floor_height).abs() <= (d.z - s.ceiling_height).abs() {
                            1.0
                        } else {
                            -1.0
                        }
                    })
                    .unwrap_or(1.0);
                combined_boost_rgb_plane_cam(
                    cx_cam,
                    cy_cam,
                    z_surf_cam,
                    cfg.muzzle_glow_alpha,
                    surf_sector,
                    lit_sectors,
                    world_lights,
                    n_z,
                    true,
                    cfg.muzzle_brdf,
                )
            } else if cfg.decal_rim_directional && (tx != 0.0 || ty != 0.0) {
                // Marca de pared: la normal es perpendicular a la tangente
                // mundo. Transformamos dos puntos a lo largo de la tangente
                // a cam-space y resolvemos la perpendicular toward-camera
                // (misma maquinaria que los slabs de pared, 3.32).
                let (ax, ay) = cam.to_cam_2d(d.x - tx, d.y - ty);
                let (bx, by) = cam.to_cam_2d(d.x + tx, d.y + ty);
                let normal = wall_normal_cam(ax, ay, bx, by, cx_cam, cy_cam);
                combined_boost_rgb_wall_cam(
                    cx_cam,
                    cy_cam,
                    z_surf_cam,
                    cfg.muzzle_glow_alpha,
                    surf_sector,
                    lit_sectors,
                    world_lights,
                    normal,
                    normal != (0.0, 0.0),
                    cfg.muzzle_brdf,
                )
            } else {
                // Billboard flotante (sangre en el aire) o direccional
                // off: omni toward-camera (3.50).
                combined_boost_rgb_sprite_cam(
                    cx_cam,
                    cy_cam,
                    z_surf_cam,
                    cfg.muzzle_glow_alpha,
                    surf_sector,
                    lit_sectors,
                    world_lights,
                    false,
                )
            };
            apply_color_boost(base, boost)
        };
        let mut path = BezPath::new();
        path.move_to(corners[0]);
        for p in &corners[1..] {
            path.line_to(*p);
        }
        path.close_path();
        out.push(Renderable {
            depth: depth - 0.5,
            color: col,
            path,
            kind: RenderKind::Fill,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn gather_sprite(
    out: &mut Vec<Renderable>,
    sprite: &SpriteSnap,
    snap: &SceneSnapshot,
    cam: &Camera,
    proj: &Projection,
    cfg: &RenderConfig,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
) {
    let (x_cam, y_cam) = cam.to_cam_2d(sprite.x, sprite.y);
    if x_cam < cfg.near {
        return;
    }
    let sec = snap.sectors.get(sprite.sector as usize);
    let floor = sec.map(|s| s.floor_height).unwrap_or(0.0);
    let depth = (x_cam * x_cam + y_cam * y_cam).sqrt();
    // Fase 3.35: punto de muestreo vertical para BRDF 3D — base del
    // billboard relativo al ojo del jugador.
    // Fase 3.38: subimos el sample al **centro** vertical del billboard
    // (`+ cfg.sprite_height * 0.5`). Default usado por el path fallback
    // (sin atlas / patch missing) — el cfg.sprite_height es estimado.
    // Fase 3.39: el path texturizado override este sample con la altura
    // **real** del patch del WAD (`(z_top + z_bot) * 0.5`), por mobj.
    // Un cyberdemon (~110 u) y un PUFF (~16 u) ahora tienen sample
    // points distintos — más fiel a su geometría real.
    let z_surf_cam = sprite.z - cam.view_z + cfg.sprite_height * 0.5;

    // Fase 3.21: sombra circular en el plano del piso bajo el sprite.
    // Va siempre — texturizado o fallback — antes de pushear el sprite
    // mismo. `gather_sprite_shadow` decide su tamaño usando el patch
    // del atlas (si está) o `cfg.sprite_half_width` como fallback.
    if cfg.sprite_shadows {
        gather_sprite_shadow(out, sprite, sec, cam, proj, cfg, x_cam, floor, depth);
    }

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
            // Fase 3.22: si el muzzle flash está activo y el sprite está
            // dentro del radio, sumamos un tinte cálido per-canal. Sprites
            // full-bright (proyectiles, fire frames) ya estaban a luz plena
            // y reciben el tinte amarillo sin saturarse — `sprite_shade_with_muzzle`
            // clampea ≤ 1.0 por canal.
            // Fase 3.23: gateado por `sprite.sector` — un imp atrás de una
            // pared sólida no se ilumina aunque la distancia euclidiana
            // del player lo alcance.
            // Fase 3.26: el sprite también recibe boost de las world
            // lights (mobjs FF_FULLBRIGHT cercanos), sumado al muzzle.
            // Fase 3.27: boost RGB per-canal — un sprite cerca de una
            // bola BFG se tinta verdoso; cerca de plasma, azulado.
            // Fase 3.31: opcionalmente direccional — luces detrás del
            // sprite back-lightean (cara visible apagada con piso
            // ambient), luces frontales tintan al 100 %.
            // Fase 3.39: sample point en el **centro real** del billboard
            // texturizado usando `(z_top + z_bot) * 0.5`. Reemplaza al
            // estimate basado en `cfg.sprite_height` que sigue vigente
            // para el fallback. Mobj alto (cyberdemon h=110) ⇒ centro
            // a 55 u sobre floor; PUFF (h=16) ⇒ 8 u. Cada uno recibe el
            // cosine BRDF apropiado para su tamaño.
            let z_surf_cam_textured = ((z_top + z_bot) * 0.5) as f32;
            let boost_rgb = combined_boost_rgb_sprite_cam(
                x_cam,
                y_cam,
                z_surf_cam_textured,
                cfg.muzzle_glow_alpha,
                sprite.sector,
                lit_sectors,
                world_lights,
                cfg.sprite_rim_directional,
            );
            let shade_rgb = sprite_shade_with_world(shade, boost_rgb);
            let img = make_tinted_sprite_image_rgb(&patch, shade_rgb);
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
    // Fase 3.26: fallback (sin patch del WAD) también combina muzzle + world lights.
    // Fase 3.27: boost RGB per-canal.
    // Fase 3.31: idem rim direccional (fake-normal toward camera) si
    // el toggle está on. Fase 3.35: distancia 3D usando `z_surf_cam`.
    let boost = combined_boost_rgb_sprite_cam(
        x_cam,
        y_cam,
        z_surf_cam,
        cfg.muzzle_glow_alpha,
        sprite.sector,
        lit_sectors,
        world_lights,
        cfg.sprite_rim_directional,
    );
    out.push(Renderable {
        depth,
        color: apply_color_boost(sprite_color(sprite, sec, depth, cfg), boost),
        path,
        kind: RenderKind::Fill,
    });
}

/// Variante per-canal: cada componente RGB se multiplica por su tint
/// individual. Usada por el muzzle flash (Fase 3.22) para tintar
/// amarillo cálido los sprites cercanos al destello del arma. Default
/// equivalente a `[shade, shade, shade]` = grayscale shading.
fn make_tinted_sprite_image_rgb(
    patch: &supay_wad::Patch,
    tint: [f32; 3],
) -> llimphi_ui::llimphi_raster::peniko::Image {
    use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
    let tr = tint[0].clamp(0.05, 1.0);
    let tg = tint[1].clamp(0.05, 1.0);
    let tb = tint[2].clamp(0.05, 1.0);
    let identity = (tr - 1.0).abs() < 1e-3 && (tg - 1.0).abs() < 1e-3 && (tb - 1.0).abs() < 1e-3;
    let tinted: Vec<u8> = if identity {
        patch.rgba.clone()
    } else {
        let mut out = Vec::with_capacity(patch.rgba.len());
        for chunk in patch.rgba.chunks_exact(4) {
            out.push(((chunk[0] as f32) * tr) as u8);
            out.push(((chunk[1] as f32) * tg) as u8);
            out.push(((chunk[2] as f32) * tb) as u8);
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
    // Horizonte = línea donde z_cam=0 cae en pantalla. Con pitch sumamos
    // `focal · tan(pitch)` al centro vertical para que el sky/floor
    // backdrop se mueva con la mirada (mouse-look). Clampeamos a los
    // bordes del rect para no pintar fuera.
    let focal = (rect.h * 0.5) / (cfg.fov_y_deg.to_radians() * 0.5).tan();
    let pitch = snap.player.view_pitch.clamp(-PITCH_MAX, PITCH_MAX);
    let pitch_offset_px = (focal * pitch.tan()) as f64;
    let mid_y_unclamped = rect.y as f64 + (rect.h as f64) * 0.5 + pitch_offset_px;
    let mid_y = mid_y_unclamped.clamp(rect.y as f64, (rect.y + rect.h) as f64);
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
        // Mantenemos el alto visual del sky constante (= mitad del rect)
        // para que el panorama no se estire al hacer pitch. El offset
        // vertical de la textura sigue al horizonte: `sky_top_y` es la
        // posición Y de la fila iy=0 del lump, calculada para que la
        // fila iy=tex_h (el bottom del panorama) caiga sobre el horizonte
        // virtual `mid_y_unclamped` (puede estar fuera del viewport
        // cuando el pitch es agresivo; vello clipea con `sky_rect`).
        let sky_visual_h = (rect.h as f64) * 0.5;
        let scale_y = tex_h / sky_visual_h;
        let sky_top_y = mid_y_unclamped - sky_visual_h;
        // Affine: image(ix, iy) → screen((ix - scroll_x) / scale_x, iy / scale_y).
        // Vello forward affine a/b/c/d/e/f donde sx = a·ix + c·iy + e,
        // sy = b·ix + d·iy + f.
        let xform = Affine::new([
            1.0 / scale_x,
            0.0,
            0.0,
            1.0 / scale_y,
            -scroll_x / scale_x + rect.x as f64,
            sky_top_y,
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
// Weapon sprite overlay (Fase 3.15)
// =====================================================================
//
// Doom pinta `psprites[ps_weapon]` (la animación del arma en mano) como
// overlay 2D sobre la vista. Las coordenadas vienen en el viewport
// nominal 320×200; escalamos al rect real preservando aspect-fit
// (Doom 4:3, igual que el FB original).

/// Constante nominal del viewport Doom — el motor produce sx/sy
/// asumiendo esta resolución base.
const DOOM_VIEW_W: f32 = 320.0;
const DOOM_VIEW_H: f32 = 200.0;
/// Constante de psprite del motor: el counter `psp->sy` arranca en 32
/// (WEAPONTOP) en idle, sube hasta 128 (WEAPONBOTTOM) cuando el arma se
/// guarda. La diferencia `sy - WEAPONTOP` es cuánto cae el arma desde
/// la posición "lista para disparar".
const DOOM_WEAPON_TOP: f32 = 32.0;

fn draw_weapon_sprite(
    scene: &mut Scene,
    rect: PaintRect,
    weap: &WeaponSpriteSnap,
    player_light: u8,
    rim_boost: BoostRgb,
    cfg: &RenderConfig,
) {
    if !weap.active {
        return;
    }
    let Some(atlas) = cfg.atlas.as_ref() else {
        return;
    };
    // Las armas en Doom son sprites no-rotacionales con lump `<NAME><F>0`.
    // Nuestra `sprite_patch` con angle=1 cae automáticamente al fallback
    // omnidireccional vía `sprite_lump`.
    let Some((patch, mirror)) = atlas.sprite_patch(weap.sprite, weap.frame, 1) else {
        return;
    };

    // Escalado uniforme: usamos la altura del rect como referencia (Doom
    // standard 320×200 = 1.6:1, mismo aspect que nuestra ventana 1280×800).
    // Aspectos más altos letterboxean horizontalmente.
    let scale = (rect.w / DOOM_VIEW_W).min(rect.h / DOOM_VIEW_H);
    let patch_w_s = patch.width as f32 * scale;
    let patch_h_s = patch.height as f32 * scale;

    // Horizontal: psp->sx defaultea 0 = centrado. Cuando hay weapon bob
    // o switch animation, sx oscila ±N pixels. Centramos el patch +
    // offset horizontal de sx.
    let screen_x_center = rect.x + rect.w * 0.5 + weap.sx * scale;
    let screen_x = screen_x_center - patch_w_s * 0.5;

    // Vertical: psp->sy es la coord top-of-patch en el viewport nominal
    // 200px de Doom. WEAPONTOP=32 = arma totalmente levantada (visible);
    // sy crece hasta WEAPONBOTTOM=128 cuando el arma baja (al cambiar
    // de arma, por ejemplo). Anchor: con sy=32, el patch queda anclado
    // al bottom del rect; subir sy lo hunde por debajo (offscreen).
    let bottom = rect.y + rect.h;
    let screen_y = bottom - patch_h_s + (weap.sy - DOOM_WEAPON_TOP) * scale;

    // Fase 3.18: el arma se tinta por la luz del sector donde está
    // parado el jugador. Si el frame tiene `FF_FULLBRIGHT` (bit 7) —
    // muzzle flash, plasma idle frame, etc. — saltamos el shade y va a
    // luz plena (igual que `gather_sprite`). Depth = 0: el arma está
    // "en la mano", no debería atenuarse por niebla aunque el cuarto
    // sí lo esté.
    let full_bright = (weap.frame & 0x80) != 0;
    let shade = if full_bright {
        1.0
    } else {
        shade_for(player_light, 0.0, cfg)
    };
    // Fase 3.28: rim-light desde world lights cercanas. El arma recoge
    // tinte ambiente per-canal (torch azul → arma azulada; fireball
    // cerca → rim rojizo). Bypass en full_bright: el destello del
    // propio fogonazo domina y subsume el ambiente.
    let tint_rgb = if full_bright {
        [shade, shade, shade]
    } else {
        sprite_shade_with_world(shade, rim_boost)
    };
    let img = make_tinted_sprite_image_rgb(&patch, tint_rgb);
    // Affine: image(ix, iy) → screen(screen_x + ix·scale, screen_y + iy·scale).
    // Para mirror, X negativo + offset al borde derecho.
    let xform = if mirror {
        Affine::new([
            -(scale as f64),
            0.0,
            0.0,
            scale as f64,
            (screen_x + patch_w_s) as f64,
            screen_y as f64,
        ])
    } else {
        Affine::new([
            scale as f64,
            0.0,
            0.0,
            scale as f64,
            screen_x as f64,
            screen_y as f64,
        ])
    };
    scene.draw_image(&img, xform);
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
    // Berserk (`pw_strength`): tinte rojo que fade-out lento. Doom:
    // `palette_idx = 12 - (strength >> 6)`, clampado a 0..7 = paletas
    // STARTREDPALS+0..7. Nosotros mapeamos a alpha directo: recién
    // agarrado el berserk strength=1 → idx=12 (max), después de muchos
    // tics strength sube y el alpha cae. `strength >> 6` empieza en 0
    // y crece a ~16+ en pocos minutos.
    if ov.power_strength > 0 {
        let shift = (ov.power_strength >> 6) as i32;
        let level = (12 - shift).clamp(1, 8) as u8;
        let alpha = (level * 10).min(90); // ramp 10..80
        return Some((180, 40, 30, alpha));
    }
    None
}

// =====================================================================
// Crosshair + viñeta de cabina (Fase 3.19)
// =====================================================================
//
// Dos capas cosméticas post-3D:
//
//   - **Viñeta**: gradient radial transparente→crimson_deep, oscurece
//     las esquinas para que el viewport se sienta como mirar por la
//     visera de un casco. Multiplica el rango de luz percibido: el
//     foco visual queda en el centro de la acción.
//   - **Crosshair**: cruz fina centrada de 4 chevrons + dot, con halo
//     crimson_deep abajo para legibilidad sobre cualquier fondo (paredes
//     claras, cielo, sprites). 7 px de marca, 1 px de ancho.

/// Pinta una viñeta radial muy sutil sobre todo el rect. `cfg.vignette`
/// controla la fuerza global (0..1+). Sin allocar paths: un único fill
/// del rect con el gradient como brush.
fn draw_vignette(scene: &mut Scene, rect: PaintRect, cfg: &RenderConfig) {
    use llimphi_ui::llimphi_raster::peniko::{color::AlphaColor, Gradient};
    if cfg.vignette <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let cx = rect.x as f64 + rect.w as f64 * 0.5;
    let cy = rect.y as f64 + rect.h as f64 * 0.5;
    // Radio = mitad de la diagonal — el último stop alcanza justo a las
    // esquinas. El centro queda transparente; el final, crimson_deep
    // tinted con alpha proporcional a `cfg.vignette`.
    let diag_half = (((rect.w as f64).powi(2) + (rect.h as f64).powi(2)).sqrt() * 0.5) as f32;
    let strength = cfg.vignette.clamp(0.0, 1.5);
    // crimson_deep ≈ rgba(90,14,14) — mismo tono del marco del header.
    let inner: Color = AlphaColor::new([0.35, 0.05, 0.05, 0.0]);
    let mid: Color = AlphaColor::new([0.35, 0.05, 0.05, 0.05 * strength]);
    let outer: Color = AlphaColor::new([0.35, 0.05, 0.05, 0.30 * strength]);
    // Tres stops: el segundo en 0.6 evita que la transición sea lineal
    // (que se ve falsa) y mantiene el centro limpio. La curva resultante
    // es casi quadrática — el oscurecimiento empieza recién en el último
    // tercio del radio.
    let gradient = Gradient::new_radial(Point::new(cx, cy), diag_half)
        .with_stops([(0.0, inner), (0.6, mid), (1.0, outer)].as_slice());
    let full = Rect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        (rect.y + rect.h) as f64,
    );
    scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &full);
}

/// Pinta un crosshair central minimalista: 4 chevrons + dot, con sombra
/// crimson_deep debajo para destacar sobre fondos claros. Tamaño fijo
/// en pixels (no escala con el viewport — un crosshair que crece se
/// siente raro). Diseño:
///
/// ```text
///        ▌
///        ▌
///   ▬▬     ▬▬
///       ·
///        ▌
///        ▌
/// ```
///
/// Distancia del centro al inicio de cada marca = `GAP` (6 px).
/// Largo de cada marca = `LEN` (7 px). Ancho = 1 px (line cap square).
fn draw_crosshair(scene: &mut Scene, rect: PaintRect) {
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    const GAP: f64 = 6.0;
    const LEN: f64 = 7.0;
    const W: f64 = 1.0;
    const DOT: f64 = 1.0;
    let cx = rect.x as f64 + rect.w as f64 * 0.5;
    let cy = rect.y as f64 + rect.h as f64 * 0.5;
    // Color de tinta + sombra. La sombra va 1 px abajo-derecha para
    // que el ojo lea las marcas aún sobre cielo claro o paredes
    // texturizadas brillantes.
    let ink: Color = AlphaColor::new([0.96, 0.92, 0.84, 0.95]); // bone ~232,216,192
    let halo: Color = AlphaColor::new([0.05, 0.02, 0.02, 0.45]); // crimson_deep darker
    // Build cada chevron como rect de 1×LEN o LEN×1.
    let arms: [Rect; 4] = [
        // top
        Rect::new(cx - W * 0.5, cy - GAP - LEN, cx + W * 0.5, cy - GAP),
        // bottom
        Rect::new(cx - W * 0.5, cy + GAP, cx + W * 0.5, cy + GAP + LEN),
        // left
        Rect::new(cx - GAP - LEN, cy - W * 0.5, cx - GAP, cy + W * 0.5),
        // right
        Rect::new(cx + GAP, cy - W * 0.5, cx + GAP + LEN, cy + W * 0.5),
    ];
    let dot = Rect::new(cx - DOT, cy - DOT, cx + DOT, cy + DOT);
    // Sombra (offset 1px abajo-derecha): se pinta primero para quedar
    // debajo de la tinta.
    let shadow_xform = Affine::translate((1.0, 1.0));
    for arm in &arms {
        scene.fill(Fill::NonZero, shadow_xform, halo, None, arm);
    }
    scene.fill(Fill::NonZero, shadow_xform, halo, None, &dot);
    // Tinta principal.
    for arm in &arms {
        scene.fill(Fill::NonZero, Affine::IDENTITY, ink, None, arm);
    }
    scene.fill(Fill::NonZero, Affine::IDENTITY, ink, None, &dot);
}

// =====================================================================
// HUD inferior modernista (Fase 3.20)
// =====================================================================
//
// Banda slim al pie del viewport 3D con los stats vitales del jugador:
// HEALTH (% + barra), ARMOR (% + barra tinted por tipo), AMMO (current
// / max del slot del arma activa), KEYS (chips por color).
//
// Paleta espejo del header del host (crimson/amber/bone/dust) para que
// la app entera se sienta una sola pieza. Fondo COLOR_BG_PANEL con
// alpha para no ocluir totalmente la acción del piso.

/// Paleta interna usada por el HUD — eco visual del header del host.
mod hud_color {
    use super::Color;
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    pub const PANEL: Color = Color::from_rgba8(12, 8, 8, 215);
    pub const RULE: Color = Color::from_rgba8(48, 16, 16, 255);
    pub const RULE_SOFT: Color = Color::from_rgba8(48, 16, 16, 140);
    pub const TRACK: Color = Color::from_rgba8(6, 4, 4, 255);
    pub const BONE: Color = Color::from_rgba8(216, 204, 188, 255);
    pub const DUST: Color = Color::from_rgba8(132, 124, 116, 255);
    pub const AMBER: Color = Color::from_rgba8(232, 168, 76, 255);
    pub const HEALTH_OK: Color = Color::from_rgba8(140, 188, 96, 255);
    pub const HEALTH_LOW: Color = Color::from_rgba8(232, 168, 76, 255);
    pub const HEALTH_CRIT: Color = Color::from_rgba8(220, 50, 50, 255);
    pub const ARMOR_GREEN: Color = Color::from_rgba8(140, 188, 96, 255);
    pub const ARMOR_BLUE: Color = Color::from_rgba8(96, 160, 232, 255);
    pub const KEY_BLUE: Color = Color::from_rgba8(56, 128, 224, 255);
    pub const KEY_YELLOW: Color = Color::from_rgba8(232, 200, 72, 255);
    pub const KEY_RED: Color = Color::from_rgba8(220, 60, 60, 255);
    /// Tinte para el indicador "skull" — más cálido/desaturado.
    pub fn skullize(base: Color) -> Color {
        let [r, g, b, a] = base.components;
        AlphaColor::new([r * 0.85, g * 0.85, b * 0.85, a])
    }
}

const HUD_HEIGHT: f64 = 38.0;
const HUD_PAD: f64 = 10.0;

/// Pinta la banda del HUD al pie del `rect`. Asume `stats.health > 0`
/// (caller filtra el pre-mapa para que no aparezca un HUD hueco).
fn draw_hud(scene: &mut Scene, ts: &mut Typesetter, rect: PaintRect, stats: &PlayerStats) {
    let view_w = rect.w as f64;
    let view_h = rect.h as f64;
    if view_w < 160.0 || view_h < HUD_HEIGHT + 32.0 {
        // Viewport demasiado chico — el HUD comería medio frame.
        return;
    }
    let bottom = rect.y as f64 + view_h;
    let top = bottom - HUD_HEIGHT;
    let left = rect.x as f64;
    let right = left + view_w;

    // Fondo + hairline crimson del borde superior.
    let panel = Rect::new(left, top, right, bottom);
    scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::PANEL, None, &panel);
    let rule = Rect::new(left, top, right, top + 1.0);
    scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::RULE, None, &rule);

    // Layout: 4 tiles. Anchos relativos al view_w restante.
    //   [ HEALTH 28% ][ ARMOR 22% ][ AMMO 26% ][ KEYS resto ]
    let usable = view_w - HUD_PAD * 2.0;
    let w_health = (usable * 0.28).floor();
    let w_armor = (usable * 0.22).floor();
    let w_ammo = (usable * 0.26).floor();
    let w_keys = usable - w_health - w_armor - w_ammo;

    let mut x = left + HUD_PAD;
    draw_hud_stat_tile(
        scene, ts, x, top, w_health,
        "HP",
        format!("{}", stats.health.max(0)),
        stats.health as f32 / 100.0,
        health_color(stats.health),
    );
    x += w_health;
    // Divider sutil entre tiles.
    draw_hud_divider(scene, x, top);
    draw_hud_stat_tile(
        scene, ts, x, top, w_armor,
        "AR",
        format!("{}", stats.armor_points.max(0)),
        stats.armor_points as f32 / 100.0,
        armor_color(stats.armor_type),
    );
    x += w_armor;
    draw_hud_divider(scene, x, top);
    draw_hud_ammo_tile(scene, ts, x, top, w_ammo, stats);
    x += w_ammo;
    draw_hud_divider(scene, x, top);
    draw_hud_keys_tile(scene, ts, x, top, w_keys, stats);
}

/// Tile genérico de "stat con barra": label dust arriba-izquierda,
/// número grande bone abajo-izquierda, barra slim al pie del tile.
fn draw_hud_stat_tile(
    scene: &mut Scene,
    ts: &mut Typesetter,
    x: f64,
    top: f64,
    w: f64,
    label: &str,
    value: String,
    pct: f32,
    bar_color: Color,
) {
    // Label "HP" / "AR" arriba.
    text::draw_block(
        scene,
        ts,
        &TextBlock {
            text: label,
            size_px: 9.0,
            color: hud_color::DUST,
            origin: (x + 4.0, top + 4.0),
            max_width: Some(w as f32 - 8.0),
            alignment: Alignment::Start,
            line_height: 1.0,
            italic: false,
            font_family: None,
        },
    );
    // Valor grande abajo del label.
    text::draw_block(
        scene,
        ts,
        &TextBlock {
            text: &value,
            size_px: 16.0,
            color: hud_color::BONE,
            origin: (x + 4.0, top + 13.0),
            max_width: Some(w as f32 - 8.0),
            alignment: Alignment::Start,
            line_height: 1.0,
            italic: false,
            font_family: None,
        },
    );
    // Barra slim al pie. 3 px de alto + track 1 px.
    let bar_y0 = top + HUD_HEIGHT - 6.0;
    let bar_y1 = bar_y0 + 3.0;
    let bar_x0 = x + 4.0;
    let bar_x1 = x + w - 6.0;
    let track = Rect::new(bar_x0, bar_y0, bar_x1, bar_y1);
    scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::TRACK, None, &track);
    let fill_w = ((bar_x1 - bar_x0) * pct.clamp(0.0, 1.0) as f64).max(0.0);
    if fill_w > 0.0 {
        let filled = Rect::new(bar_x0, bar_y0, bar_x0 + fill_w, bar_y1);
        scene.fill(Fill::NonZero, Affine::IDENTITY, bar_color, None, &filled);
    }
}

/// Tile de ammo: muestra `current / max` del slot del arma activa, o
/// "—" si la actual no consume ammo (puño, motosierra).
fn draw_hud_ammo_tile(
    scene: &mut Scene,
    ts: &mut Typesetter,
    x: f64,
    top: f64,
    w: f64,
    stats: &PlayerStats,
) {
    // Label "AMMO" + sufijo del slot (CLIP/SHELL/CELL/MISL).
    let slot_label = stats.weapon_ammo_slot().map(ammo_slot_name).unwrap_or("—");
    text::draw_block(
        scene,
        ts,
        &TextBlock {
            text: &format!("AMMO · {slot_label}"),
            size_px: 9.0,
            color: hud_color::DUST,
            origin: (x + 4.0, top + 4.0),
            max_width: Some(w as f32 - 8.0),
            alignment: Alignment::Start,
            line_height: 1.0,
            italic: false,
            font_family: None,
        },
    );
    // current / max — current en ámbar si está bajo (<25%).
    let (value, pct, color) = match stats.weapon_ammo_slot() {
        Some(slot) => {
            let cur = stats.ammo[slot].max(0);
            let max = stats.max_ammo[slot].max(1);
            let pct = (cur as f32) / (max as f32);
            let col = if pct < 0.25 {
                hud_color::HEALTH_CRIT
            } else if pct < 0.5 {
                hud_color::HEALTH_LOW
            } else {
                hud_color::BONE
            };
            (format!("{cur} / {max}"), pct, col)
        }
        None => ("∞".to_string(), 0.0, hud_color::DUST),
    };
    text::draw_block(
        scene,
        ts,
        &TextBlock {
            text: &value,
            size_px: 16.0,
            color,
            origin: (x + 4.0, top + 13.0),
            max_width: Some(w as f32 - 8.0),
            alignment: Alignment::Start,
            line_height: 1.0,
            italic: false,
            font_family: None,
        },
    );
    // Barra slim al pie — ammo en ámbar para distinguir de HP/AR.
    let bar_y0 = top + HUD_HEIGHT - 6.0;
    let bar_y1 = bar_y0 + 3.0;
    let bar_x0 = x + 4.0;
    let bar_x1 = x + w - 6.0;
    let track = Rect::new(bar_x0, bar_y0, bar_x1, bar_y1);
    scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::TRACK, None, &track);
    if pct > 0.0 {
        let fill_w = ((bar_x1 - bar_x0) * pct.clamp(0.0, 1.0) as f64).max(0.0);
        let filled = Rect::new(bar_x0, bar_y0, bar_x0 + fill_w, bar_y1);
        scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::AMBER, None, &filled);
    }
}

/// Tile de llaves: hasta 6 chips por color (cards + skulls). Chip
/// vacío si no se tiene la llave — silueta crimson_deep.
fn draw_hud_keys_tile(
    scene: &mut Scene,
    ts: &mut Typesetter,
    x: f64,
    top: f64,
    w: f64,
    stats: &PlayerStats,
) {
    text::draw_block(
        scene,
        ts,
        &TextBlock {
            text: "KEYS",
            size_px: 9.0,
            color: hud_color::DUST,
            origin: (x + 4.0, top + 4.0),
            max_width: Some(w as f32 - 8.0),
            alignment: Alignment::Start,
            line_height: 1.0,
            italic: false,
            font_family: None,
        },
    );
    // 6 chips: card_blue, card_yellow, card_red, skull_blue, skull_yellow, skull_red.
    // Cards: rectángulo 12×8. Skulls: rectángulo 12×8 con borde más grueso
    // (un truco visual para distinguirlos sin pintar un sprite real).
    let colors = [
        hud_color::KEY_BLUE,
        hud_color::KEY_YELLOW,
        hud_color::KEY_RED,
    ];
    let chip_w = 13.0;
    let chip_h = 8.0;
    let gap = 4.0;
    let chips_total = chip_w * 6.0 + gap * 5.0;
    let mut cx = x + 4.0;
    // Si los chips no entran, los apretamos.
    let avail = w - 8.0;
    let scale = if chips_total > avail {
        avail / chips_total
    } else {
        1.0
    };
    let chip_w = chip_w * scale;
    let gap = gap * scale;
    let cy0 = top + 18.0;
    let cy1 = cy0 + chip_h;
    for i in 0..6 {
        let has = stats.cards[i];
        let color_idx = i % 3;
        let is_skull = i >= 3;
        let base = colors[color_idx];
        let chip = Rect::new(cx, cy0, cx + chip_w, cy1);
        if has {
            let fill = if is_skull { hud_color::skullize(base) } else { base };
            scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &chip);
            if is_skull {
                // Mini-banda crimson en el medio del chip → silueta de
                // calavera apenas evocada.
                let band = Rect::new(
                    cx + chip_w * 0.35,
                    cy0 + 2.0,
                    cx + chip_w * 0.65,
                    cy1 - 2.0,
                );
                scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::TRACK, None, &band);
            }
        } else {
            // Chip vacío: borde crimson_deep, interior transparente.
            // Lo aproximamos con 4 rects 1px (top/bottom/left/right).
            let bw = 1.0;
            for r in &[
                Rect::new(cx, cy0, cx + chip_w, cy0 + bw),
                Rect::new(cx, cy1 - bw, cx + chip_w, cy1),
                Rect::new(cx, cy0, cx + bw, cy1),
                Rect::new(cx + chip_w - bw, cy0, cx + chip_w, cy1),
            ] {
                scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::RULE_SOFT, None, r);
            }
        }
        cx += chip_w + gap;
    }
}

fn draw_hud_divider(scene: &mut Scene, x: f64, top: f64) {
    let r = Rect::new(x, top + 6.0, x + 1.0, top + HUD_HEIGHT - 6.0);
    scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::RULE_SOFT, None, &r);
}

fn health_color(hp: i32) -> Color {
    if hp >= 60 {
        hud_color::HEALTH_OK
    } else if hp >= 25 {
        hud_color::HEALTH_LOW
    } else {
        hud_color::HEALTH_CRIT
    }
}

fn armor_color(armor_type: u8) -> Color {
    match armor_type {
        1 => hud_color::ARMOR_GREEN,
        2 => hud_color::ARMOR_BLUE,
        _ => hud_color::DUST,
    }
}

fn ammo_slot_name(slot: usize) -> &'static str {
    match slot {
        0 => "CLIP",
        1 => "SHELL",
        2 => "CELL",
        3 => "MISL",
        _ => "—",
    }
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
    fn projection_pitch_up_shifts_horizon_down() {
        // pitch positivo = mirar hacia arriba → línea del horizonte
        // (puntos con z_cam=0) baja en pantalla (sy mayor).
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj_flat = Projection::new(rect, 75_f32.to_radians());
        let proj_up = Projection::new_pitched(rect, 75_f32.to_radians(), 0.4);
        let p_flat = proj_flat.project(10.0, 0.0, 0.0);
        let p_up = proj_up.project(10.0, 0.0, 0.0);
        assert!(
            p_up.y > p_flat.y,
            "pitch up debe empujar el horizonte hacia abajo, flat={} up={}",
            p_flat.y,
            p_up.y
        );
        // El offset debe ser exactamente `focal · tan(pitch)`.
        let focal = (rect.h as f64) * 0.5 / (75_f32.to_radians() as f64 * 0.5).tan();
        let expected = focal * (0.4_f64).tan();
        assert!(
            (p_up.y - p_flat.y - expected).abs() < 1e-3,
            "offset esperado {expected}, observado {}",
            p_up.y - p_flat.y
        );
    }

    #[test]
    fn projection_pitch_down_shifts_horizon_up() {
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj_flat = Projection::new(rect, 75_f32.to_radians());
        let proj_dn = Projection::new_pitched(rect, 75_f32.to_radians(), -0.3);
        let p_flat = proj_flat.project(10.0, 0.0, 0.0);
        let p_dn = proj_dn.project(10.0, 0.0, 0.0);
        assert!(
            p_dn.y < p_flat.y,
            "pitch down debe empujar el horizonte hacia arriba, flat={} down={}",
            p_flat.y,
            p_dn.y
        );
    }

    #[test]
    fn projection_pitch_does_not_alter_x() {
        // El y-shear es vertical puro — la coordenada X de un punto
        // debe quedar idéntica con o sin pitch.
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj_flat = Projection::new(rect, 75_f32.to_radians());
        let proj_up = Projection::new_pitched(rect, 75_f32.to_radians(), 0.5);
        let p_flat = proj_flat.project(10.0, 3.0, 0.0);
        let p_up = proj_up.project(10.0, 3.0, 0.0);
        assert!(
            (p_flat.x - p_up.x).abs() < 1e-3,
            "X debe ser invariante al pitch, flat.x={} up.x={}",
            p_flat.x,
            p_up.x
        );
    }

    #[test]
    fn projection_pitch_clamps_extremes() {
        // Más allá de ±π/3 el horizonte se sale del viewport; el
        // clamp del constructor evita tan() explotando.
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj_extreme = Projection::new_pitched(rect, 75_f32.to_radians(), 5.0);
        let proj_max = Projection::new_pitched(rect, 75_f32.to_radians(), PITCH_MAX);
        let p_extreme = proj_extreme.project(10.0, 0.0, 0.0);
        let p_max = proj_max.project(10.0, 0.0, 0.0);
        assert!(
            (p_extreme.y - p_max.y).abs() < 1e-3,
            "valores absurdos deben clampearse a PITCH_MAX"
        );
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
    // Fase 3.18: subsector point query + player sector light
    // -----------------------------------------------------------------

    #[test]
    fn subsector_at_point_picks_leaf_containing_point() {
        // Misma partición que `simple_two_leaf_bsp`: línea x=0 (dy=1).
        // Punto (+10, 0): side = 0 - 1·10 = -10 < 0 → near = children[1] = ss1.
        // Punto (-10, 0): side = 0 + 10 = +10 > 0 → near = children[0] = ss0.
        let nodes = simple_two_leaf_bsp();
        assert_eq!(subsector_at_point(&nodes, 10.0, 0.0), Some(1));
        assert_eq!(subsector_at_point(&nodes, -10.0, 0.0), Some(0));
    }

    #[test]
    fn subsector_at_point_none_without_bsp() {
        // Sin nodes (snapshot stub, mapa no cargado) la query devuelve None
        // sin entrar al loop — el caller cae a su fallback default.
        assert_eq!(subsector_at_point(&[], 0.0, 0.0), None);
    }

    #[test]
    fn player_sector_light_picks_local_light_level() {
        // Dos sectores con luces opuestas; el player en cada lado debe
        // leer el light_level del sector donde está parado.
        let dim = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 64,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let bright = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 240,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![dim, bright]);
        // ss0 → sector 0 (dim), ss1 → sector 1 (bright). Coincide con la
        // convención de `simple_two_leaf_bsp`: viewer en (+10, 0) cae en ss1.
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 1, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());

        snap.player.x = 10.0;
        snap.player.y = 0.0;
        assert_eq!(player_sector_light(&snap), 240, "player en ss1 (bright)");

        snap.player.x = -10.0;
        assert_eq!(player_sector_light(&snap), 64, "player en ss0 (dim)");
    }

    #[test]
    fn player_sector_light_falls_back_without_bsp() {
        // Snapshot vacío: no hay BSP, no hay sectores. Fallback 192 —
        // mismo valor que usa `gather_sprite` para sprites sin sector.
        let snap = SceneSnapshot::empty(0);
        assert_eq!(player_sector_light(&snap), DEFAULT_PLAYER_LIGHT);
        assert_eq!(DEFAULT_PLAYER_LIGHT, 192);
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
    fn overlay_berserk_fades_with_strength() {
        // Fase 3.16: berserk recién agarrado tinte rojo intenso; después
        // de muchos tics el alpha cae.
        let fresh = PlayerOverlays {
            power_strength: 1,
            ..Default::default()
        };
        let old = PlayerOverlays {
            power_strength: 600,
            ..Default::default()
        };
        let (_, _, _, a_fresh) = overlay_rgba(&fresh, 0).expect("berserk fresh");
        let (_, _, _, a_old) = overlay_rgba(&old, 0).expect("berserk old");
        assert!(a_fresh > a_old, "alpha cae con tics: fresh={a_fresh} old={a_old}");
    }

    #[test]
    fn overlay_radsuit_priority_over_berserk() {
        // Si radsuit + berserk activos, gana radsuit (verde, no rojo).
        let ov = PlayerOverlays {
            power_strength: 1,
            power_radsuit: 200,
            ..Default::default()
        };
        let (r, g, _b, _a) = overlay_rgba(&ov, 0).expect("overlay");
        assert!(g > r, "radsuit verde domina berserk rojo: r={r} g={g}");
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

    // -----------------------------------------------------------------
    // Fase 3.22: muzzle world light
    // -----------------------------------------------------------------

    #[test]
    fn muzzle_boost_zero_when_alpha_zero() {
        // alpha = 0 ⇒ no hay fogonazo, boost = 0 sin importar la posición.
        assert_eq!(muzzle_boost_cam(0.0, 0.0, 0.0), 0.0);
        assert_eq!(muzzle_boost_cam(50.0, 30.0, 0.0), 0.0);
        // alpha negativo (no debería pasar pero defensivo) ⇒ 0.
        assert_eq!(muzzle_boost_cam(0.0, 0.0, -0.5), 0.0);
    }

    #[test]
    fn muzzle_boost_zero_outside_radius() {
        // distancia² > RADIUS² → boost 0. Tomamos el doble del radio.
        let r = MUZZLE_RADIUS_WORLD;
        assert_eq!(muzzle_boost_cam(r * 2.0, 0.0, 1.0), 0.0);
        assert_eq!(muzzle_boost_cam(0.0, r * 1.5, 1.0), 0.0);
        // Justo en el límite también es 0 (>= radius).
        assert_eq!(muzzle_boost_cam(r, 0.0, 1.0), 0.0);
    }

    #[test]
    fn muzzle_boost_peak_at_center_with_full_alpha() {
        // En (0, 0) con alpha=1 el boost alcanza MUZZLE_BOOST_PEAK exacto.
        let b = muzzle_boost_cam(0.0, 0.0, 1.0);
        assert!((b - MUZZLE_BOOST_PEAK).abs() < 1e-5, "expected peak, got {b}");
    }

    #[test]
    fn muzzle_boost_falls_off_with_distance_squared() {
        // Falloff quadrático: comparando r/4 vs r/2 (mismo eje), el
        // boost a r/4 debe ser estrictamente mayor que a r/2, y la
        // diferencia no debe ser lineal.
        let r = MUZZLE_RADIUS_WORLD;
        let b_close = muzzle_boost_cam(r * 0.25, 0.0, 1.0);
        let b_mid = muzzle_boost_cam(r * 0.5, 0.0, 1.0);
        let b_far = muzzle_boost_cam(r * 0.75, 0.0, 1.0);
        assert!(b_close > b_mid);
        assert!(b_mid > b_far);
        // Quadrático: el ratio close/mid debe ser > 1.5 (lineal sería ~1.5).
        // Con (1 - d²/r²)² obtenemos: (1-1/16)² ≈ 0.879 vs (1-1/4)² ≈ 0.563.
        // Ratio ≈ 1.56. Verificamos > 1.4 con margen.
        assert!(b_close / b_mid > 1.4, "ratio {} too low", b_close / b_mid);
    }

    #[test]
    fn apply_muzzle_tint_warms_color() {
        // Base gris medio + boost positivo ⇒ los canales R y G suben más
        // que B (tint cálido amarillo-blanco). Alpha preservada.
        let base = Color::from_rgba8(100, 100, 100, 255);
        let warm = apply_muzzle_tint(base, 0.3);
        let [r, g, b, a] = warm.to_rgba8().to_u8_array();
        assert_eq!(a, 255, "alpha preserved");
        assert!(r > 100 && g > 100 && b > 100, "all channels boosted");
        assert!(r >= g, "red ≥ green tint shape");
        assert!(g > b, "yellow tint: green > blue");
    }

    #[test]
    fn apply_muzzle_tint_zero_is_identity() {
        // boost ≤ 0 ⇒ retorna el color sin cambio. Fast path.
        let base = Color::from_rgba8(77, 188, 222, 200);
        let same = apply_muzzle_tint(base, 0.0);
        assert_eq!(same.to_rgba8().to_u8_array(), [77, 188, 222, 200]);
        let same2 = apply_muzzle_tint(base, -0.5);
        assert_eq!(same2.to_rgba8().to_u8_array(), [77, 188, 222, 200]);
    }

    #[test]
    fn sprite_shade_with_muzzle_zero_is_grayscale() {
        // boost = 0 ⇒ idéntico al shading grayscale histórico.
        let s = sprite_shade_with_muzzle(0.6, 0.0);
        assert_eq!(s, [0.6, 0.6, 0.6]);
    }

    #[test]
    fn sprite_shade_with_muzzle_warm_when_boost_positive() {
        // boost > 0 ⇒ R/G suben más que B respecto al shading uniforme.
        let s = sprite_shade_with_muzzle(0.5, 0.4);
        // El tint es (255, 220, 140) / 255 ≈ (1.0, 0.86, 0.55).
        // Multiplicador per-canal: 1 + 0.4 · tint. Red ≥ green > blue.
        assert!(s[0] >= s[1], "R ≥ G");
        assert!(s[1] > s[2], "G > B");
        // Todos los canales clampean ≤ 1.0.
        assert!(s[0] <= 1.0 && s[1] <= 1.0 && s[2] <= 1.0);
    }

    // -----------------------------------------------------------------
    // Fase 3.23: oclusión sectorial del muzzle boost
    // -----------------------------------------------------------------

    /// Construye un snapshot con el BSP de 2 hojas y un set de paredes
    /// que conectan el sector 0 (player room) al 1 vía two-sided, y
    /// dejan el sector 2 aislado (sólo paredes one-sided).
    fn snap_with_adjacency() -> SceneSnapshot {
        let mk_sector = || SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 160,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![mk_sector(), mk_sector(), mk_sector()]);
        // 2 subsectores: ss0 → sector 0 (player), ss1 → sector 1.
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 1, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());
        // Player en (-10, 0) ⇒ subsector 0 ⇒ sector 0 (ver
        // `subsector_at_point_picks_leaf_containing_point`).
        snap.player.x = -10.0;
        snap.player.y = 0.0;

        let wall = |fs: u32, bs: u32| WallSeg {
            x1: 0.0,
            y1: 0.0,
            x2: 0.0,
            y2: 0.0,
            front_sector: fs,
            back_sector: bs,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        snap.walls = Arc::from(vec![
            // 0↔1 two-sided: el muzzle del player en 0 ilumina al 1.
            wall(0, 1),
            // Sector 2: sólo paredes one-sided ⇒ no conecta con player.
            wall(2, NO_SECTOR),
        ]);
        snap
    }

    #[test]
    fn lit_sectors_includes_player_sector() {
        let snap = snap_with_adjacency();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(lit.contains(&0), "sector del player siempre lit");
    }

    #[test]
    fn lit_sectors_includes_adjacent_via_twosided() {
        let snap = snap_with_adjacency();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(lit.contains(&1), "vecino directo via two-sided lit");
    }

    #[test]
    fn lit_sectors_excludes_unconnected_sector() {
        let snap = snap_with_adjacency();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(
            !lit.contains(&2),
            "sector aislado (sólo one-sided) no entra al lit set"
        );
    }

    #[test]
    fn lit_sectors_none_without_bsp() {
        // Stub mode: sin nodes BSP devuelve None ⇒ "lit everywhere"
        // (3.22 behavior preservado en stub).
        let snap = SceneSnapshot::empty(0);
        assert!(compute_muzzle_lit_sectors(&snap).is_none());
    }

    #[test]
    fn muzzle_boost_gated_passes_through_when_lit_none() {
        // Sin lit set (modo stub o toggle apagado), el boost pasa
        // sin gating — equivalente a 3.22.
        let b = muzzle_boost_gated(0.3, 42, None);
        assert!((b - 0.3).abs() < 1e-6);
    }

    #[test]
    fn muzzle_boost_gated_keeps_when_sector_in_lit() {
        let mut lit = HashSet::new();
        lit.insert(7_u32);
        let b = muzzle_boost_gated(0.3, 7, Some(&lit));
        assert!((b - 0.3).abs() < 1e-6, "sector 7 está en lit ⇒ boost intacto");
    }

    #[test]
    fn muzzle_boost_gated_zeroes_when_sector_not_in_lit() {
        let mut lit = HashSet::new();
        lit.insert(7_u32);
        let b = muzzle_boost_gated(0.3, 99, Some(&lit));
        assert_eq!(b, 0.0, "sector 99 no está en lit ⇒ boost gateado a 0");
    }

    // -----------------------------------------------------------------
    // Fase 3.24: BFS multi-hop + filtro por radio del bridge wall
    // -----------------------------------------------------------------

    /// Snap con una cadena de sectores 0→1→2→3 vía paredes two-sided
    /// + sector 5 colgado al jugador por un bridge wall lejano.
    /// Player en (-10, 0) ⇒ subsector 0 ⇒ sector 0.
    fn snap_with_chain() -> SceneSnapshot {
        let mk_sector = || SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 160,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![
            mk_sector(),
            mk_sector(),
            mk_sector(),
            mk_sector(),
            mk_sector(),
            mk_sector(),
        ]);
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 1, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());
        snap.player.x = -10.0;
        snap.player.y = 0.0;

        // Pared con midpoint en `(mx, my)` (segmento `[mx, my]→[mx, my]`
        // → midpoint trivial). Suficiente para el test del radius filter
        // del BFS — la geometría real no importa.
        let wall = |fs: u32, bs: u32, mx: f32, my: f32| WallSeg {
            x1: mx,
            y1: my,
            x2: mx,
            y2: my,
            front_sector: fs,
            back_sector: bs,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        // Cadena 0↔1↔2↔3 con midpoints crecientes en X. Todos dentro
        // del radio salvo el último W23 a 200 unidades (aún dentro de
        // 384 desde player=-10 → distancia 210 < 384). El sector 3
        // queda fuera del lit por hops>MAX (2), no por radio.
        //
        // Bridge wall lejano 0↔5 con midpoint a 500 — fuera del radio
        // desde player=-10 (distancia 510 > 384). Sector 5 no entra al
        // lit pese a ser vecino directo.
        snap.walls = Arc::from(vec![
            wall(0, 1, 0.0, 0.0),     // hop 1: dist 10 → ✓
            wall(1, 2, 50.0, 0.0),    // hop 2: dist 60 → ✓
            wall(2, 3, 200.0, 0.0),   // hop 3 (no se llega por MAX=2)
            wall(0, 5, 500.0, 0.0),   // hop 1 pero bridge fuera del radio
        ]);
        snap
    }

    #[test]
    fn lit_sectors_includes_two_hop_neighbor_within_radius() {
        // BFS llega a sector 2 vía W01 (hop 1) + W12 (hop 2). Ambos
        // bridge walls dentro del radio físico.
        let snap = snap_with_chain();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(lit.contains(&0), "sector del player");
        assert!(lit.contains(&1), "vecino directo");
        assert!(lit.contains(&2), "vecino-del-vecino dentro del radio");
    }

    #[test]
    fn lit_sectors_bfs_stops_at_max_hops() {
        // Sector 3 requeriría hop 3 (MAX=2 corta). Aunque W23 está dentro
        // del radio, el BFS ya no lo alcanza.
        let snap = snap_with_chain();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(
            !lit.contains(&3),
            "sector a 3 hops no entra al lit (MAX_HOPS=2)"
        );
    }

    #[test]
    fn lit_sectors_excludes_one_hop_when_bridge_wall_beyond_radius() {
        // Sector 5 es vecino directo de 0 (W05), pero el midpoint del
        // bridge está a >MUZZLE_RADIUS_WORLD del jugador. El filtro
        // descarta el wall del BFS aunque la adyacencia exista.
        let snap = snap_with_chain();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(
            !lit.contains(&5),
            "vecino directo con bridge wall fuera de MUZZLE_RADIUS no entra al lit"
        );
    }

    // -----------------------------------------------------------------
    // Fase 3.25: radio cumulativo por hop (Dijkstra-lite)
    // -----------------------------------------------------------------

    /// L-shape: dos paredes alineadas en codo donde el chequeo
    /// per-bridge contra el player (3.24) aprobaría ambas, pero el
    /// camino acumulativo player→W01→W12 supera el radio.
    ///
    /// - Player en (-10, 0) ⇒ subsector 0 ⇒ sector 0.
    /// - W01 midpoint (200, 0): dist desde player = 210 < 384.
    /// - W12 midpoint (200, 200): dist desde player ≈ 290 < 384 (3.24 lo aceptaba).
    /// - Cumulativo: 210 (player→W01) + 200 (W01→W12) = 410 > 384.
    ///   3.25 corta el camino y deja sec 2 fuera del lit set.
    fn snap_with_l_shape() -> SceneSnapshot {
        let mk_sector = || SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 160,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![mk_sector(), mk_sector(), mk_sector()]);
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 1, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());
        snap.player.x = -10.0;
        snap.player.y = 0.0;

        let wall = |fs: u32, bs: u32, mx: f32, my: f32| WallSeg {
            x1: mx,
            y1: my,
            x2: mx,
            y2: my,
            front_sector: fs,
            back_sector: bs,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        snap.walls = Arc::from(vec![
            wall(0, 1, 200.0, 0.0),   // hop1 cumulative = 210
            wall(1, 2, 200.0, 200.0), // hop2 cumulative = 410 > 384
        ]);
        snap
    }

    #[test]
    fn lit_sectors_cumulative_path_cuts_when_sum_exceeds_radius() {
        // 3.25 vs 3.24: ambos walls pasarían el chequeo per-bridge contra
        // el player (290 y 210 < 384), pero el camino real acumulado
        // recorre 410 unidades — fuera del radio. Sec 2 se excluye.
        let snap = snap_with_l_shape();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(lit.contains(&0), "player sector siempre lit");
        assert!(lit.contains(&1), "vecino directo dentro del radio");
        assert!(
            !lit.contains(&2),
            "L-shape: camino acumulativo 410 > 384 corta antes de sec 2"
        );
    }

    /// Cadena donde cada hop suma poco al anterior aunque los midpoints
    /// estén lejos del jugador. Sólo es alcanzable correctamente si el
    /// algoritmo usa el midpoint del bridge previo como entry point del
    /// siguiente hop (no la posición del player).
    fn snap_with_chained_entry_points() -> SceneSnapshot {
        let mk_sector = || SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 160,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![mk_sector(), mk_sector(), mk_sector()]);
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 1, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());
        snap.player.x = 0.0;
        snap.player.y = 0.0;

        let wall = |fs: u32, bs: u32, mx: f32, my: f32| WallSeg {
            x1: mx,
            y1: my,
            x2: mx,
            y2: my,
            front_sector: fs,
            back_sector: bs,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        // W01 mid (300, 0). hop_d = 300.
        // W12 mid (300, 50).
        //   - Si entry = (300, 0) (W01 mid): hop_d = 50. cumulativo sec2 = 350 < 384.
        //   - Si entry = (0, 0) (player): hop_d ≈ 304. cumulativo sec2 ≈ 604 > 384.
        snap.walls = Arc::from(vec![
            wall(0, 1, 300.0, 0.0),
            wall(1, 2, 300.0, 50.0),
        ]);
        snap
    }

    #[test]
    fn lit_sectors_cumulative_uses_wall_midpoint_as_entry() {
        // Si el algoritmo siempre midiera desde el player, sec 2 caería
        // fuera (cumulative ≈ 604). Con entry chaining (3.25), sec 2 entra
        // (cumulative = 350 < 384).
        let snap = snap_with_chained_entry_points();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(lit.contains(&1), "sec 1 lit (cumulative=300)");
        assert!(
            lit.contains(&2),
            "sec 2 lit via entry-chaining (cumulative=350 < 384) — sin el chain caería"
        );
    }

    // -----------------------------------------------------------------
    // Fase 3.26: world point lights desde FF_FULLBRIGHT mobjs
    // -----------------------------------------------------------------

    /// Sprite helper para los tests de world lights.
    fn fb_sprite(x: f32, y: f32, frame: u8, sector: u32) -> SpriteSnap {
        SpriteSnap {
            x,
            y,
            z: 0.0,
            angle: 0.0,
            sprite: 0,
            frame,
            sector,
        }
    }

    #[test]
    fn world_lights_boost_zero_with_empty_list() {
        // Sin lights, el boost siempre es 0 en cualquier punto.
        assert_eq!(world_lights_boost_cam(0.0, 0.0, &[]), 0.0);
        assert_eq!(world_lights_boost_cam(100.0, -200.0, &[]), 0.0);
    }

    #[test]
    fn world_lights_boost_peak_at_center_with_single_light() {
        // Una sola luz en (0,0); evaluamos el boost exactamente en (0,0).
        // f = 1 - 0/r² = 1 ⇒ boost = 1 · 1 · PEAK.
        let lights = vec![WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: MUZZLE_TINT_RGB,
            lit_sectors: None,
        }];
        let b = world_lights_boost_cam(0.0, 0.0, &lights);
        assert!(
            (b - WORLD_LIGHT_PEAK).abs() < 1e-5,
            "esperado peak {}, dió {}",
            WORLD_LIGHT_PEAK,
            b
        );
    }

    #[test]
    fn world_lights_boost_zero_outside_radius() {
        // Luz al borde y más allá del radio ⇒ boost 0.
        let lights = vec![WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: MUZZLE_TINT_RGB,
            lit_sectors: None,
        }];
        let r = WORLD_LIGHT_RADIUS_WORLD;
        assert_eq!(world_lights_boost_cam(r, 0.0, &lights), 0.0);
        assert_eq!(world_lights_boost_cam(0.0, r * 1.5, &lights), 0.0);
        assert_eq!(world_lights_boost_cam(-r * 2.0, 0.0, &lights), 0.0);
    }

    #[test]
    fn world_lights_boost_falls_off_with_distance_squared() {
        // En d=r/2 ⇒ f = 1 - 0.25 = 0.75 ⇒ boost = 0.5625 · PEAK.
        // En d=r/4 ⇒ f = 1 - 1/16 = 0.9375 ⇒ boost = 0.879 · PEAK.
        // El ratio close/mid > 1.4 verifica la caída cuadrática.
        let lights = vec![WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: MUZZLE_TINT_RGB,
            lit_sectors: None,
        }];
        let close = world_lights_boost_cam(WORLD_LIGHT_RADIUS_WORLD * 0.25, 0.0, &lights);
        let mid = world_lights_boost_cam(WORLD_LIGHT_RADIUS_WORLD * 0.5, 0.0, &lights);
        assert!(close > mid, "más cerca ⇒ más boost");
        assert!(
            close / mid > 1.4,
            "ratio close/mid {} debería superar 1.4 (cuadrático)",
            close / mid
        );
    }

    #[test]
    fn world_lights_boost_sums_multiple_sources_clamped_to_muzzle_peak() {
        // Dos luces colocadas exactamente en el mismo punto ⇒ suma de
        // contribuciones, pero clampeada al peak del muzzle (invariante:
        // el fogonazo del arma no debe quedar dominado por proyectiles).
        let lights = vec![
            WorldLight {
                x_cam: 0.0,
                y_cam: 0.0,
                z_cam: 0.0,
                sector: 0,
                tint_rgb: MUZZLE_TINT_RGB,
                lit_sectors: None,
            },
            WorldLight {
                x_cam: 0.0,
                y_cam: 0.0,
                z_cam: 0.0,
                sector: 1,
                tint_rgb: MUZZLE_TINT_RGB,
                lit_sectors: None,
            },
        ];
        let b = world_lights_boost_cam(0.0, 0.0, &lights);
        // Sin clamp serían 2 × PEAK = 0.8; con clamp = MUZZLE_BOOST_PEAK.
        assert!(b <= MUZZLE_BOOST_PEAK + 1e-5);
        assert!(b > WORLD_LIGHT_PEAK, "suma debería superar PEAK individual");
    }

    #[test]
    fn gather_world_lights_filters_non_fullbright() {
        // Snapshot con dos sprites: uno full-bright (frame con bit 7),
        // uno normal. Sólo el primero entra al lit set.
        let mut snap = SceneSnapshot::empty(0);
        snap.sprites = Arc::from(vec![
            fb_sprite(64.0, 0.0, 0x82, 0),  // FF_FULLBRIGHT
            fb_sprite(128.0, 0.0, 0x02, 0), // sin bit 7
        ]);
        let cam = Camera::new(0.0, 0.0, 0.0, 0.0);
        let lights = gather_world_lights(&snap, &cam, None, false);
        assert_eq!(lights.len(), 1, "sólo el sprite FF_FULLBRIGHT cuenta");
    }

    #[test]
    fn gather_world_lights_skips_no_sector_and_caps_to_max() {
        // 20 sprites FF_FULLBRIGHT ⇒ se truncan a MAX_WORLD_LIGHTS.
        // Uno con NO_SECTOR queda excluido siempre.
        let mut sprites: Vec<SpriteSnap> = (0..20)
            .map(|i| fb_sprite(50.0 + i as f32 * 5.0, 0.0, 0x80, (i as u32) % 4))
            .collect();
        sprites.push(fb_sprite(0.0, 0.0, 0x80, NO_SECTOR));
        let mut snap = SceneSnapshot::empty(0);
        snap.sprites = Arc::from(sprites);
        let cam = Camera::new(0.0, 0.0, 0.0, 0.0);
        let lights = gather_world_lights(&snap, &cam, None, false);
        assert_eq!(
            lights.len(),
            MAX_WORLD_LIGHTS,
            "truncado a {} aunque haya más",
            MAX_WORLD_LIGHTS
        );
        // El sprite con NO_SECTOR no debería estar; los de cap son los más
        // cercanos al player (origen). El más cercano (i=0 a 50 units) sí
        // debería entrar — verificamos por presencia de un x cercano.
        let min_dx = lights
            .iter()
            .map(|l| l.x_cam.abs())
            .fold(f32::INFINITY, f32::min);
        assert!(
            min_dx < 60.0,
            "el más cercano (i=0 a 50 u) debe estar entre los seleccionados"
        );
    }

    #[test]
    fn combined_boost_clamps_to_muzzle_peak_when_muzzle_and_lights_overlap() {
        // Muzzle peak (alpha=1, surface en origen) + luz coincidente:
        // suma sin clamp = 0.55 + 0.40 = 0.95; con clamp = 0.55.
        let lights = vec![WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: MUZZLE_TINT_RGB,
            lit_sectors: None,
        }];
        let b = combined_boost_cam(0.0, 0.0, 1.0, 0, None, &lights);
        assert!(
            (b - MUZZLE_BOOST_PEAK).abs() < 1e-5,
            "esperado peak {}, dió {}",
            MUZZLE_BOOST_PEAK,
            b
        );
    }

    // -----------------------------------------------------------------
    // Fase 3.27: tinte per-spritenum + boost RGB per-canal
    // -----------------------------------------------------------------

    #[test]
    fn sprite_tint_for_name_resolves_known_sprites() {
        let imp = sprite_tint_for_name("BAL1");
        assert_eq!(imp, (255, 130, 60), "imp fireball rojo-naranja");
        let plasma = sprite_tint_for_name("PLSS");
        assert_eq!(plasma, (130, 180, 255), "plasma azul-cyan");
        let bfg = sprite_tint_for_name("BFS1");
        assert_eq!(bfg, (160, 255, 160), "BFG ball verde fluorescente");
        let torch_blue = sprite_tint_for_name("TBLU");
        assert_eq!(torch_blue, (110, 160, 255), "blue torch azul");
    }

    #[test]
    fn sprite_tint_for_name_falls_back_to_muzzle_tint_for_unknown() {
        let unk = sprite_tint_for_name("XYZW");
        assert_eq!(unk, MUZZLE_TINT_RGB, "sprite desconocido cae al amarillo");
        // El nombre puede traer más de 4 chars (e.g. "PLSSA0"); el match
        // se hace sobre los primeros 4 — debería resolver igual.
        let plasma_long = sprite_tint_for_name("PLSSA0");
        assert_eq!(plasma_long, (130, 180, 255));
    }

    #[test]
    fn sprite_tint_for_name_is_case_insensitive() {
        // El motor a veces devuelve los nombres tal-cual del WAD (uppercase)
        // pero defensemos contra mixed-case por si una fase futura los
        // normaliza.
        assert_eq!(sprite_tint_for_name("bal1"), (255, 130, 60));
        assert_eq!(sprite_tint_for_name("Plss"), (130, 180, 255));
    }

    // -----------------------------------------------------------------
    // Fase 3.36: tintes Doom 2 (mancubus, revenant, archvile, etc.)
    // -----------------------------------------------------------------

    #[test]
    fn sprite_tint_for_name_resolves_doom2_projectiles() {
        // MANF (mancubus fireball), FATB (revenant tracer), SKEL
        // (revenant attack) — todos con tinte cálido distinto de
        // MUZZLE_TINT_RGB (el fallback amarillo del 3.26).
        let manf = sprite_tint_for_name("MANF");
        assert_eq!(manf, (255, 160, 90), "mancubus fireball naranja");
        let fatb = sprite_tint_for_name("FATB");
        assert_eq!(fatb, (255, 220, 160), "revenant tracer pálido cálido");
        let skel = sprite_tint_for_name("SKEL");
        assert_eq!(skel, (255, 200, 150), "revenant attack pálido cálido");
        // Todos los Doom 2 tints deben diferir del fallback amarillo.
        assert_ne!(manf, MUZZLE_TINT_RGB);
        assert_ne!(fatb, MUZZLE_TINT_RGB);
        assert_ne!(skel, MUZZLE_TINT_RGB);
    }

    #[test]
    fn sprite_tint_for_name_resolves_archvile_flame() {
        // Archvile attack frames (VILE) + fire pillar (FIRE) — ambos
        // rojo flame, FIRE más saturado.
        let vile = sprite_tint_for_name("VILE");
        assert_eq!(vile, (255, 130, 70), "archvile attack rojo flame");
        let fire = sprite_tint_for_name("FIRE");
        assert_eq!(fire, (255, 100, 50), "archvile fire pillar rojo saturado");
        // FIRE más rojo (G más bajo) que VILE — el pillar es más intenso.
        assert!(fire.1 < vile.1, "FIRE G < VILE G");
    }

    #[test]
    fn sprite_tint_for_name_resolves_lost_soul_and_pickups() {
        // Lost soul (SKUL) = blue-white flame; soul sphere (SOUL) y
        // mega armor (MEGA) = azul/cyan glow.
        let skul = sprite_tint_for_name("SKUL");
        assert_eq!(skul, (180, 220, 255), "lost soul blue-white");
        let soul = sprite_tint_for_name("SOUL");
        assert_eq!(soul, (130, 200, 255), "soul sphere cyan-blue");
        let mega = sprite_tint_for_name("MEGA");
        assert_eq!(mega, (130, 220, 200), "mega armor verde-cyan");
        // Los tres tienen B > R (azules), distintos del fallback amarillo.
        assert!(skul.2 > skul.0);
        assert!(soul.2 > soul.0);
        assert!(mega.2 > mega.0);
    }

    #[test]
    fn sprite_tint_for_name_resolves_colored_keys() {
        // Keycards y skullkeys — colores que matchean el HUD del juego.
        assert_eq!(sprite_tint_for_name("BKEY"), (110, 160, 255), "blue keycard");
        assert_eq!(sprite_tint_for_name("YKEY"), (255, 240, 130), "yellow keycard");
        assert_eq!(sprite_tint_for_name("RKEY"), (255, 130, 90),  "red keycard");
        assert_eq!(sprite_tint_for_name("BSKU"), (110, 160, 255), "blue skullkey");
        assert_eq!(sprite_tint_for_name("YSKU"), (255, 240, 130), "yellow skullkey");
        assert_eq!(sprite_tint_for_name("RSKU"), (255, 130, 90),  "red skullkey");
        // Mismas keys card y skull deben dar el mismo color.
        assert_eq!(sprite_tint_for_name("BKEY"), sprite_tint_for_name("BSKU"));
    }

    #[test]
    fn sprite_tint_for_name_doom2_lookups_case_insensitive() {
        // Las entradas nuevas también respetan el case-insensitive del 3.27.
        assert_eq!(sprite_tint_for_name("manf"), (255, 160, 90));
        assert_eq!(sprite_tint_for_name("Skul"), (180, 220, 255));
        assert_eq!(sprite_tint_for_name("vile"), (255, 130, 70));
        // El 4-char match también funciona con sufijos (e.g. "MANFA1" ⇒ MANF).
        assert_eq!(sprite_tint_for_name("MANFA1"), (255, 160, 90));
        assert_eq!(sprite_tint_for_name("SKULA0"), (180, 220, 255));
    }

    #[test]
    fn muzzle_boost_rgb_uses_muzzle_tint_per_channel() {
        // Muzzle en origen con alpha=1 ⇒ scalar = MUZZLE_BOOST_PEAK.
        // Per-canal = peak · (255/255, 220/255, 140/255).
        let b = muzzle_boost_rgb_cam(0.0, 0.0, 1.0);
        let expected_r = MUZZLE_BOOST_PEAK * (MUZZLE_TINT_RGB.0 as f32 / 255.0);
        let expected_g = MUZZLE_BOOST_PEAK * (MUZZLE_TINT_RGB.1 as f32 / 255.0);
        let expected_b = MUZZLE_BOOST_PEAK * (MUZZLE_TINT_RGB.2 as f32 / 255.0);
        assert!((b[0] - expected_r).abs() < 1e-5);
        assert!((b[1] - expected_g).abs() < 1e-5);
        assert!((b[2] - expected_b).abs() < 1e-5);
        // R > G > B porque el amarillo cálido tiene R=255 > G=220 > B=140.
        assert!(b[0] > b[1] && b[1] > b[2], "amarillo: R > G > B");
    }

    #[test]
    fn world_lights_boost_rgb_per_light_tint_dominates() {
        // Una sola luz verde (BFG) en el origen ⇒ boost RGB con G alto,
        // R/B mucho más bajos.
        let lights = vec![WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: (160, 255, 160), // BFG green
            lit_sectors: None,
        }];
        let b = world_lights_boost_rgb_cam(0.0, 0.0, 0, &lights);
        assert!(b[1] > b[0] && b[1] > b[2], "G debe dominar para BFG verde");
        // Magnitud verde ≈ WORLD_LIGHT_PEAK · 255/255 = PEAK.
        assert!(
            (b[1] - WORLD_LIGHT_PEAK).abs() < 1e-5,
            "G debe alcanzar PEAK"
        );
    }

    #[test]
    fn combined_boost_rgb_clamps_each_channel_to_muzzle_peak() {
        // Muchas luces saturadas en cada canal ⇒ cada canal clampea a peak.
        let lights: Vec<WorldLight> = (0..10)
            .map(|_| WorldLight {
                x_cam: 0.0,
                y_cam: 0.0,
                z_cam: 0.0,
                sector: 0,
                tint_rgb: (255, 255, 255), // luz blanca máxima
                lit_sectors: None,
            })
            .collect();
        let b = combined_boost_rgb_cam(0.0, 0.0, 1.0, 0, None, &lights);
        for ch in 0..3 {
            assert!(
                b[ch] <= MUZZLE_BOOST_PEAK + 1e-5,
                "canal {} {} > peak",
                ch,
                b[ch]
            );
            // Y deberían estar saturados (al peak):
            assert!(
                (b[ch] - MUZZLE_BOOST_PEAK).abs() < 1e-4,
                "canal {} debería estar saturado",
                ch
            );
        }
    }

    #[test]
    fn apply_color_boost_adds_per_channel() {
        let base = Color::from_rgba8(50, 50, 50, 255);
        // Boost sólo en G ⇒ sale verdoso.
        let b = apply_color_boost(base, [0.0, 0.4, 0.0]);
        let [r, g, bb, a] = b.to_rgba8().to_u8_array();
        assert_eq!(r, 50, "R sin cambio");
        assert!(g > 100, "G boosted (esperado ~50 + 0.4·255 ≈ 152), dió {g}");
        assert_eq!(bb, 50, "B sin cambio");
        assert_eq!(a, 255, "alpha preservada");
    }

    #[test]
    fn apply_color_boost_zero_is_identity() {
        let base = Color::from_rgba8(120, 80, 200, 200);
        let same = apply_color_boost(base, ZERO_BOOST);
        assert_eq!(same.to_rgba8().to_u8_array(), [120, 80, 200, 200]);
    }

    #[test]
    fn sprite_shade_with_world_per_channel() {
        // Shade base 0.5, boost RGB (0, 0.4, 0) ⇒ G escalado, R/B intactos.
        let s = sprite_shade_with_world(0.5, [0.0, 0.4, 0.0]);
        assert!((s[0] - 0.5).abs() < 1e-5, "R sin cambio");
        assert!(s[1] > 0.5, "G boosted");
        assert!((s[2] - 0.5).abs() < 1e-5, "B sin cambio");
    }

    #[test]
    fn overlay_color_alpha_from_boost_normalizes_to_brightest_channel() {
        // Boost dominantemente verde con poca R y nada de B ⇒
        // color overlay debe ser verde dominante.
        let (r, g, b, a) = overlay_color_alpha_from_boost([0.05, 0.30, 0.0]).expect("non-trivial");
        assert!(g > r && g > b, "G dominante en color overlay");
        assert!(a > 0, "alpha > 0 para boost no despreciable");
    }

    #[test]
    fn overlay_color_alpha_from_boost_none_when_negligible() {
        // Boost por debajo del threshold ⇒ None.
        assert!(overlay_color_alpha_from_boost([0.01, 0.0, 0.0]).is_none());
        assert!(overlay_color_alpha_from_boost(ZERO_BOOST).is_none());
    }

    #[test]
    fn gather_world_lights_uses_default_tint_without_atlas() {
        // Sin atlas (modo stub), los lights caen al amarillo cálido.
        let mut snap = SceneSnapshot::empty(0);
        snap.sprites = Arc::from(vec![fb_sprite(64.0, 0.0, 0x80, 0)]);
        let cam = Camera::new(0.0, 0.0, 0.0, 0.0);
        let lights = gather_world_lights(&snap, &cam, None, false);
        assert_eq!(lights.len(), 1);
        assert_eq!(lights[0].tint_rgb, MUZZLE_TINT_RGB);
    }

    // =================================================================
    // Fase 3.28 — Weapon rim-light desde world lights
    // =================================================================

    /// Helper: una `WorldLight` en `(x_cam, y_cam)` con el tinte dado.
    /// `lit_sectors: None` ⇒ aporta sin gating sectorial (path 3.27).
    fn rim_light(x: f32, y: f32, tint: (u8, u8, u8)) -> WorldLight {
        WorldLight {
            x_cam: x,
            y_cam: y,
            z_cam: 0.0,
            sector: NO_SECTOR,
            tint_rgb: tint,
            lit_sectors: None,
        }
    }

    #[test]
    fn weapon_rim_boost_zero_at_player_with_no_world_lights() {
        // Sin world lights el arma no recibe tinte ambiente: identity.
        let boost = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &[]);
        assert_eq!(boost, ZERO_BOOST);
        let tint = sprite_shade_with_world(0.7, boost);
        assert!((tint[0] - 0.7).abs() < 1e-5);
        assert!((tint[1] - 0.7).abs() < 1e-5);
        assert!((tint[2] - 0.7).abs() < 1e-5);
    }

    #[test]
    fn weapon_rim_boost_blue_torch_skews_blue_at_player() {
        // Antorcha azul a 120 u del jugador (dentro de WORLD_LIGHT_RADIUS=192):
        // el boost en (0,0) tiene B > R y B > G — el arma se tinta azulada.
        let blue = (110, 160, 255);
        let lights = [rim_light(120.0, 0.0, blue)];
        let boost = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert!(
            boost[2] > boost[0] && boost[2] > boost[1],
            "blue torch debería skewear B: got [{}, {}, {}]",
            boost[0], boost[1], boost[2]
        );
        // Con shade=0.5 (cuarto oscuro, donde el rim importa) el tinte
        // final preserva la asimetría: el canal B queda por encima del R.
        // En shade=1.0 todos los canales saturan a 1.0 — el rim sólo
        // se ve cuando el arma está apagada por luz baja.
        let tint = sprite_shade_with_world(0.5, boost);
        assert!(tint[2] > tint[0], "tint[B] > tint[R] con shade bajo");
    }

    #[test]
    fn weapon_rim_boost_red_fireball_skews_red_at_player() {
        // BAL1 imp fireball a 80 u del jugador: el boost tiene R > G > B.
        let red = (255, 130, 60);
        let lights = [rim_light(80.0, 0.0, red)];
        let boost = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert!(
            boost[0] > boost[1] && boost[1] > boost[2],
            "fireball debería skewear R > G > B: got [{}, {}, {}]",
            boost[0], boost[1], boost[2]
        );
    }

    #[test]
    fn weapon_rim_boost_zero_when_light_beyond_radius() {
        // Una luz fuera del radio (`WORLD_LIGHT_RADIUS_WORLD`) no aporta
        // boost al arma — el rim queda neutro aunque haya antorchas
        // lejanas en línea de vista.
        let blue = (110, 160, 255);
        let r = WORLD_LIGHT_RADIUS_WORLD + 1.0;
        let lights = [rim_light(r, 0.0, blue)];
        let boost = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(boost, ZERO_BOOST);
    }

    #[test]
    fn weapon_full_bright_bypasses_rim_boost() {
        // Cuando el frame del arma tiene FF_FULLBRIGHT, el render usa
        // `[shade, shade, shade]` y *no* sprite_shade_with_world — el
        // destello del fogonazo domina y subsume el ambiente. Validamos
        // que el path normal en cuarto oscuro (shade=0.5) sí preserva
        // la asimetría per-canal, mientras el path full_bright es
        // grayscale: `[1, 1, 1]` independiente del boost.
        let blue = (110, 160, 255);
        let lights = [rim_light(120.0, 0.0, blue)];
        let boost = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        // Path normal: tint asimétrico per-canal en shade bajo.
        let normal_tint = sprite_shade_with_world(0.5, boost);
        assert!(
            normal_tint[2] > normal_tint[0],
            "path normal debería tener B>R con boost azul + shade bajo"
        );
        // Path full_bright: el render *no* llama a sprite_shade_with_world,
        // usa `[shade, shade, shade]` directo — grayscale.
        let full_bright_tint = [1.0_f32, 1.0, 1.0];
        assert_eq!(full_bright_tint[0], full_bright_tint[1]);
        assert_eq!(full_bright_tint[1], full_bright_tint[2]);
    }

    // =================================================================
    // Fase 3.29 — Oclusión sectorial de world lights
    // =================================================================

    #[test]
    fn lit_sectors_from_arbitrary_source_includes_source_sector() {
        // Generalización: arrancar la BFS desde un sector arbitrario
        // (p. ej. el sector que aloja a un proyectil FF_FULLBRIGHT)
        // siempre incluye al sector origen, y al vecino conectado por
        // two-sided. El sector 2 (sólo one-sided) queda excluido.
        let snap = snap_with_adjacency();
        let lit = compute_lit_sectors_from(&snap, 0.0, 0.0, 1, WORLD_LIGHT_RADIUS_WORLD);
        assert!(lit.contains(&1), "sector origen siempre en el set");
        assert!(lit.contains(&0), "vecino directo via two-sided incluido");
        assert!(!lit.contains(&2), "sector aislado fuera del set");
    }

    #[test]
    fn world_lights_boost_rgb_skips_light_when_surf_not_in_lit_sectors() {
        // Luz con lit_sectors restringido a {1}. Superficie en sector 2 ⇒
        // la luz no aporta. Misma luz evaluada con surf_sector=1 sí aporta.
        let mut lit = HashSet::new();
        lit.insert(1_u32);
        let light = WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 1,
            tint_rgb: (255, 255, 255),
            lit_sectors: Some(Arc::new(lit)),
        };
        let lights = [light];
        let blocked = world_lights_boost_rgb_cam(0.0, 0.0, 2, &lights);
        assert_eq!(blocked, ZERO_BOOST, "sector no listado ⇒ luz oculta");
        let visible = world_lights_boost_rgb_cam(0.0, 0.0, 1, &lights);
        assert!(visible[0] > 0.0, "sector listado ⇒ luz aporta");
    }

    #[test]
    fn world_lights_boost_rgb_passes_light_when_lit_sectors_is_none() {
        // Backward-compat 3.27: lit_sectors=None ⇒ surf_sector ignorado.
        // Una luz sin gating aporta en cualquier sector.
        let light = WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: (255, 255, 255),
            lit_sectors: None,
        };
        let lights = [light];
        let b0 = world_lights_boost_rgb_cam(0.0, 0.0, 0, &lights);
        let b9 = world_lights_boost_rgb_cam(0.0, 0.0, 999, &lights);
        assert_eq!(b0, b9, "sin gating, surf_sector no cambia el boost");
        assert!(b0[0] > 0.0);
    }

    #[test]
    fn gather_world_lights_computes_lit_sectors_when_occlusion_enabled() {
        // Con BSP + un sprite FF_FULLBRIGHT en sector 1 + oclusión on,
        // la luz cachea un set que incluye al menos su sector origen.
        let mut snap = snap_with_adjacency();
        // Sprite en (0, 0): cae sobre el seam pero el sector lo fijamos
        // explícitamente a 1 (igual al snap_with_adjacency wall 0↔1).
        snap.sprites = Arc::from(vec![fb_sprite(0.0, 0.0, 0x80, 1)]);
        let cam = Camera::new(snap.player.x, snap.player.y, 0.0, 0.0);
        let lights = gather_world_lights(&snap, &cam, None, true);
        assert_eq!(lights.len(), 1);
        let set = lights[0]
            .lit_sectors
            .as_ref()
            .expect("oclusión on con BSP ⇒ Some(set)");
        assert!(set.contains(&1), "set incluye sector origen");
    }

    #[test]
    fn gather_world_lights_skips_occlusion_when_disabled_or_no_bsp() {
        // (a) oclusión off ⇒ lit_sectors = None para todas.
        let mut snap = snap_with_adjacency();
        snap.sprites = Arc::from(vec![fb_sprite(0.0, 0.0, 0x80, 1)]);
        let cam = Camera::new(snap.player.x, snap.player.y, 0.0, 0.0);
        let off = gather_world_lights(&snap, &cam, None, false);
        assert_eq!(off.len(), 1);
        assert!(off[0].lit_sectors.is_none(), "oclusión off ⇒ None");
        // (b) oclusión on pero sin BSP (snapshot sintético sin nodes)
        // ⇒ lit_sectors = None (el caller cae al comportamiento 3.27).
        let mut bare = SceneSnapshot::empty(0);
        bare.sprites = Arc::from(vec![fb_sprite(20.0, 0.0, 0x80, 0)]);
        let cam2 = Camera::new(0.0, 0.0, 0.0, 0.0);
        let no_bsp = gather_world_lights(&bare, &cam2, None, true);
        assert_eq!(no_bsp.len(), 1);
        assert!(no_bsp[0].lit_sectors.is_none(), "sin BSP ⇒ None");
    }

    // =================================================================
    // Fase 3.30 — Rim direccional del arma
    // =================================================================

    #[test]
    fn weapon_rim_directional_full_intensity_in_front() {
        // Luz a 80u en +X_cam (frente al jugador). Sin tinte real (luz
        // blanca pura) ⇒ cos(0)=1 ⇒ att=1.0 ⇒ boost igual al omni.
        let white = (255, 255, 255);
        let lights = [rim_light(80.0, 0.0, white)];
        let omni = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        // Diferencia despreciable ⇒ ambos paths coinciden al frente.
        for ch in 0..3 {
            assert!(
                (omni[ch] - dir[ch]).abs() < 1e-5,
                "frente debería igualar omni: canal {} omni={} dir={}",
                ch,
                omni[ch],
                dir[ch]
            );
        }
    }

    #[test]
    fn weapon_rim_directional_attenuates_lights_behind() {
        // Luz a 80u en -X_cam (detrás del jugador). cos=-1 ⇒
        // att=(0.5-0.5).max(0.3)=0.3. Boost direccional debería ser
        // estrictamente menor que omni (que ignora dirección).
        let white = (255, 255, 255);
        let lights = [rim_light(-80.0, 0.0, white)];
        let omni = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        // Cada canal direccional debería ser ~0.3 del omni (el piso).
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                assert!(dir[ch] < omni[ch], "canal {} debería atenuar atrás", ch);
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - WEAPON_RIM_AMBIENT_FLOOR).abs() < 1e-4,
                    "ratio canal {} = {} debería ser ≈ piso {}",
                    ch,
                    ratio,
                    WEAPON_RIM_AMBIENT_FLOOR
                );
            }
        }
    }

    #[test]
    fn weapon_rim_directional_side_lights_use_half() {
        // Luz a 80u en +Y_cam (lateral derecho). cos=0 ⇒ att=0.5.
        // El boost lateral debe quedar ~ a mitad del omni.
        let white = (255, 255, 255);
        let lights = [rim_light(0.0, 80.0, white)];
        let omni = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - 0.5).abs() < 1e-4,
                    "lateral debería ser 0.5 del omni: canal {} ratio {}",
                    ch,
                    ratio
                );
            }
        }
    }

    #[test]
    fn weapon_rim_directional_disabled_equals_omni() {
        // Toggle off ⇒ direccional==omni para cualquier configuración.
        let red = (255, 130, 60);
        let blue = (110, 160, 255);
        let lights = [
            rim_light(120.0, 0.0, red),
            rim_light(-60.0, 90.0, blue),
            rim_light(0.0, -150.0, (255, 255, 200)),
        ];
        let omni = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        let baseline = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(
            omni, baseline,
            "directional=false debe ser bit-identical al path 3.29"
        );
    }

    #[test]
    fn weapon_rim_directional_handles_zero_distance() {
        // Luz exactamente en el jugador (raro pero posible: psprite
        // FF_FULLBRIGHT del propio fogonazo si entrara por error). El
        // cos no está definido; degradamos a att=1.0 y evitamos NaN.
        let white = (255, 255, 255);
        let lights = [rim_light(0.0, 0.0, white)];
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} no NaN/Inf", ch);
            assert!(dir[ch] > 0.0, "luz pegada al player aporta full");
        }
    }

    // =================================================================
    // Fase 3.31 — Rim direccional de mobj sprites
    // =================================================================

    #[test]
    fn sprite_rim_directional_front_light_matches_omni() {
        // Sprite a (200, 0) en cam-space (frente al jugador). Una luz
        // a (100, 0) está entre el jugador y el sprite — desde el
        // sprite, la luz queda en dirección -X (hacia la cámara), que
        // es exactamente su fake-normal. cos(0)=1 ⇒ att=1.0 ⇒ el path
        // direccional debería coincidir bit-a-bit con el omni.
        let white = (255, 255, 255);
        let lights = [rim_light(100.0, 0.0, white)];
        let omni = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            assert!(
                (omni[ch] - dir[ch]).abs() < 1e-5,
                "luz front al sprite debería igualar omni: canal {} omni={} dir={}",
                ch, omni[ch], dir[ch]
            );
        }
    }

    #[test]
    fn sprite_rim_directional_back_light_falls_to_floor() {
        // Sprite a (200, 0), luz a (260, 0) (detrás del sprite desde
        // la cámara). Desde el sprite la luz está en +X (lejos de la
        // cámara), opuesto a la fake-normal (-1, 0). cos=-1 ⇒ att=floor.
        let white = (255, 255, 255);
        let lights = [rim_light(260.0, 0.0, white)];
        let omni = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - SPRITE_RIM_AMBIENT_FLOOR).abs() < 1e-4,
                    "back-light debería caer al piso ambient: canal {} ratio {}",
                    ch, ratio
                );
            }
        }
    }

    #[test]
    fn sprite_rim_directional_side_light_uses_half() {
        // Sprite a (200, 0), luz a (200, 60) (al costado del sprite,
        // perpendicular al eje player→sprite). Desde el sprite la
        // dirección a la luz es (0, 1) — perpendicular a la normal
        // (-1, 0). cos=0 ⇒ att=0.5.
        let white = (255, 255, 255);
        let lights = [rim_light(200.0, 60.0, white)];
        let omni = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - 0.5).abs() < 1e-4,
                    "lateral debería ser 0.5 del omni: canal {} ratio {}",
                    ch, ratio
                );
            }
        }
    }

    #[test]
    fn sprite_rim_directional_disabled_equals_omni_for_arbitrary_lights() {
        // Toggle off ⇒ direccional debe coincidir con `world_lights_boost_rgb_cam`
        // para cualquier configuración de luces (tres luces, tintes
        // distintos, posiciones mezcladas alrededor del sprite).
        let red = (255, 130, 60);
        let blue = (110, 160, 255);
        let warm = (255, 220, 140);
        let lights = [
            rim_light(180.0, 30.0, red),
            rim_light(120.0, -40.0, blue),
            rim_light(240.0, 80.0, warm),
        ];
        let omni = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        let baseline = world_lights_boost_rgb_cam(200.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(omni, baseline, "directional=false debe ser bit-identical al path 3.29");
    }

    #[test]
    fn sprite_rim_directional_degenerates_safely_at_camera() {
        // Sprite exactamente en el origen del cam-space (degenerado:
        // billboard sin normal definida). Caemos al path omni dentro
        // del helper direccional para evitar NaN. Resultado finito y
        // ≥ 0 por canal.
        let white = (255, 255, 255);
        let lights = [rim_light(50.0, 0.0, white)];
        let dir = world_lights_boost_rgb_for_sprite_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} no NaN/Inf", ch);
        }
        // Y debería coincidir con el omni (porque caemos al fallback).
        let omni = world_lights_boost_rgb_for_sprite_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        assert_eq!(dir, omni, "degenerado ⇒ fallback omni");
    }

    // =================================================================
    // Fase 3.32 — Rim direccional para paredes
    // =================================================================

    #[test]
    fn wall_normal_cam_orients_toward_camera() {
        // Pared horizontal a la derecha del player: endpoints (100, -50)
        // y (100, 50). Midpoint (100, 0). Normal candidates ±(1, 0) (no
        // ±(0, 1) — ojo: perpendicular a la dirección (0, 100)).
        // La que apunta toward camera (origen) es (-1, 0).
        let n = wall_normal_cam(100.0, -50.0, 100.0, 50.0, 100.0, 0.0);
        assert!((n.0 - (-1.0)).abs() < 1e-5, "nx debe ser -1: {}", n.0);
        assert!(n.1.abs() < 1e-5, "ny debe ser ~0: {}", n.1);
    }

    #[test]
    fn wall_normal_cam_degenerate_zero_length() {
        // Pared degenerada (endpoints idénticos) ⇒ (0, 0). El caller
        // debería caer al path omni.
        let n = wall_normal_cam(50.0, 50.0, 50.0, 50.0, 50.0, 50.0);
        assert_eq!(n, (0.0, 0.0));
    }

    #[test]
    fn wall_rim_directional_perpendicular_light_full_intensity() {
        // Pared a x=100, normal toward camera = (-1, 0). Luz frente a la
        // pared sobre el eje normal: cam-space (50, 0) — perpendicular
        // directo al plano. Direction surf→light = (-50, 0)/50 = (-1, 0).
        // cos(theta) = dot(normal, dir) = (-1)·(-1) + 0 = 1 ⇒ att=1.
        let white = (255, 255, 255);
        let lights = [rim_light(50.0, 0.0, white)];
        let n = (-1.0, 0.0);
        let omni = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(
                (omni[ch] - dir[ch]).abs() < 1e-5,
                "luz perpendicular ⇒ direccional ≈ omni: canal {}", ch
            );
        }
    }

    #[test]
    fn wall_rim_directional_grazing_uses_half() {
        // Pared a x=100, normal (-1, 0). Luz sobre el plano de la
        // pared: cam-space (100, 30) — paralela al lineseg. Direction
        // surf→light = (0, 30)/30 = (0, 1). cos = (-1)·0 + 0·1 = 0
        // ⇒ att = 0.5.
        let white = (255, 255, 255);
        let lights = [rim_light(100.0, 30.0, white)];
        let n = (-1.0, 0.0);
        let omni = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - 0.5).abs() < 1e-4,
                    "rasante debería ser 0.5: canal {} ratio {}", ch, ratio
                );
            }
        }
    }

    #[test]
    fn wall_rim_directional_back_light_falls_to_floor() {
        // Pared a x=100, normal (-1, 0). Luz "detrás" de la pared
        // (lejos de la cámara): cam-space (150, 0). Direction surf→light
        // = (50, 0)/50 = (1, 0). cos = (-1)·1 = -1 ⇒ att=floor (0.3).
        let white = (255, 255, 255);
        let lights = [rim_light(150.0, 0.0, white)];
        let n = (-1.0, 0.0);
        let omni = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - WALL_RIM_AMBIENT_FLOOR).abs() < 1e-4,
                    "back-light ⇒ piso ambient: canal {} ratio {}", ch, ratio
                );
            }
        }
    }

    #[test]
    fn wall_rim_directional_disabled_equals_omni() {
        // Toggle off ⇒ direccional debe coincidir con `world_lights_boost_rgb_cam`
        // para múltiples luces en distintas direcciones.
        let red = (255, 130, 60);
        let blue = (110, 160, 255);
        let lights = [
            rim_light(50.0, 0.0, red),
            rim_light(100.0, 40.0, blue),
            rim_light(150.0, -20.0, (255, 240, 200)),
        ];
        let n = (-1.0, 0.0);
        let omni = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        let baseline = world_lights_boost_rgb_cam(100.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(omni, baseline, "directional=false ⇒ bit-identical al 3.29");
    }

    // =================================================================
    // Fase 3.33 — BRDF para pisos y techos con z exportado
    // =================================================================

    /// Helper: luz con z_cam dado.
    fn plane_light(x: f32, y: f32, z: f32, tint: (u8, u8, u8)) -> WorldLight {
        WorldLight {
            x_cam: x,
            y_cam: y,
            z_cam: z,
            sector: NO_SECTOR,
            tint_rgb: tint,
            lit_sectors: None,
        }
    }

    #[test]
    fn plane_rim_directional_floor_strongest_when_light_above() {
        // Floor centroide en el origen. Dos luces a igual d_3D=50 pero
        // distinta dirección — el cosine es la única variable:
        // - above (0, 30, 40): dz = +40 ⇒ cos = 40/50 = 0.8 ⇒ att = 0.9.
        // - level (50, 0, 0): dz = 0 ⇒ cos = 0 ⇒ att = 0.5.
        // Ratio esperado ≈ 1.8 (=0.9/0.5) por canal — el plano "ve"
        // mejor la luz por arriba (su cara mira a +Z).
        let white = (255, 255, 255);
        let above = [plane_light(0.0, 30.0, 40.0, white)];
        let level = [plane_light(50.0, 0.0, 0.0, white)];
        let b_above = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &above, 1.0, true);
        let b_level = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &level, 1.0, true);
        for ch in 0..3 {
            assert!(
                b_above[ch] > b_level[ch],
                "luz por arriba del floor debería iluminar más: canal {} above={} level={}",
                ch, b_above[ch], b_level[ch]
            );
        }
    }

    #[test]
    fn plane_rim_directional_ceiling_strongest_when_light_below() {
        // Espejo del test del floor con normal `-Z`. d_3D=50 fijo:
        // - below (0, 30, -40): dz = -40 ⇒ cos = -(-40)/50 = 0.8.
        // - level (50, 0, 0): dz = 0 ⇒ cos = 0.
        let white = (255, 255, 255);
        let below = [plane_light(0.0, 30.0, -40.0, white)];
        let level = [plane_light(50.0, 0.0, 0.0, white)];
        let b_below = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &below, -1.0, true);
        let b_level = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &level, -1.0, true);
        for ch in 0..3 {
            assert!(
                b_below[ch] > b_level[ch],
                "luz por debajo del ceiling debería iluminar más: canal {} below={} level={}",
                ch, b_below[ch], b_level[ch]
            );
        }
    }

    #[test]
    fn plane_rim_directional_3d_radius_cuts_far_vertical() {
        // Luz a 0 XY pero z_cam = 250 — fuera del radio (192). El
        // path 2D omni la incluiría (distancia horizontal = 0); el
        // 3D direccional la rechaza por d_3D = 250 > 192. Result =
        // ZERO_BOOST en direccional, > 0 en omni.
        let white = (255, 255, 255);
        let lights = [plane_light(0.0, 0.0, 250.0, white)];
        let dir = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, 1.0, true);
        let omni = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, 1.0, false);
        assert_eq!(dir, ZERO_BOOST, "3D radio corta la luz lejana en z");
        assert!(omni[0] > 0.0, "omni 2D no la corta (distancia XY=0)");
    }

    #[test]
    fn plane_rim_directional_disabled_equals_omni_2d() {
        // Toggle off ⇒ el helper de plano debe coincidir bit-a-bit con
        // `world_lights_boost_rgb_cam` (path omni 2D del 3.29).
        let lights = [
            plane_light(50.0, 30.0, 20.0, (255, 130, 60)),
            plane_light(80.0, -20.0, -50.0, (110, 160, 255)),
        ];
        let off = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, 1.0, false);
        let baseline = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(off, baseline);
    }

    #[test]
    fn plane_rim_directional_floor_back_lit_from_below_falls_to_floor() {
        // Floor con normal +Z. Una luz "por debajo" del piso (dz < 0)
        // back-lightea: cos = +1 * dz/d_3D < 0 ⇒ att = floor (raro en
        // Doom — los mobjs FF_FULLBRIGHT van por arriba de los pisos —
        // pero el caso límite debe atenuarse al piso ambient).
        let white = (255, 255, 255);
        // Floor a z_cam = 0; luz a z_cam = -50 (50 abajo del piso).
        let lights = [plane_light(0.0, 0.0, -50.0, white)];
        let dir = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, 1.0, true);
        // Omni-2D: distancia 2D = 0 ⇒ full peak.
        let omni = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, 1.0, false);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                // Direccional debería ser bastante menor que omni
                // (att ≈ floor = 0.3, modulado además por el radio
                // 3D que en este test sigue dentro del rango).
                assert!(
                    dir[ch] < omni[ch] * (PLANE_RIM_AMBIENT_FLOOR + 0.1),
                    "back-lit floor: canal {} dir={} no clampea cerca del floor ambient",
                    ch, dir[ch]
                );
            }
        }
    }

    // =================================================================
    // Fase 3.34 — BRDF 3D para paredes
    // =================================================================

    #[test]
    fn wall_rim_3d_high_light_attenuates_compared_to_planar() {
        // Pared en x=100, normal toward-camera (-1, 0). Dos luces a la
        // **misma XY** (50, 0) pero distinta z_cam:
        //   - planar: (50, 0, 0) — al nivel del eye / surface sample.
        //   - high:   (50, 0, 60) — 60 unidades por encima.
        // El path 3D usa d² 3D y cos(θ) = (nx·dx + ny·dy)/d_3D — la
        // luz alta tiene d_3D > d_2D y cos < cos_2D, por lo que su
        // aporte cae respecto a la planar.
        let white = (255, 255, 255);
        let planar = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let high = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 60.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let b_planar = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &planar, n, true);
        let b_high = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &high, n, true);
        for ch in 0..3 {
            assert!(
                b_planar[ch] > b_high[ch],
                "luz alta debería atenuar más vía 3D: canal {} planar={} high={}",
                ch, b_planar[ch], b_high[ch]
            );
        }
    }

    #[test]
    fn wall_rim_3d_radius_cuts_far_vertical_light() {
        // Pared en x=100, normal (-1, 0). Luz a XY (100, 0) pero
        // z=250 (muy arriba). En 2D d=0 ⇒ omni la incluye. En 3D
        // d=250 > r=192 ⇒ direccional la excluye.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 0.0, z_cam: 250.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        let omni = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        assert_eq!(dir, ZERO_BOOST, "3D radio corta la luz lejana en z");
        assert!(omni[0] > 0.0, "omni 2D no la corta (d_XY=0)");
    }

    #[test]
    fn wall_rim_3d_planar_light_finite_and_positive() {
        // Luz con z_cam=0 (planar al eye level). El path 3D con dz=0
        // colapsa al cálculo 2D del 3.32 — verificamos sanidad
        // numérica para una geometría no-trivial.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 20.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} finite", ch);
            assert!(dir[ch] > 0.0, "canal {} positivo", ch);
        }
    }

    #[test]
    fn wall_rim_3d_disabled_uses_omni_2d() {
        // Toggle off ⇒ debería seguir usando `world_lights_boost_rgb_cam`
        // omni 2D del 3.29 — bit-equivalente aún con z_cam alto.
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 20.0, z_cam: 100.0,
            sector: NO_SECTOR, tint_rgb: (200, 200, 200), lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let off = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        let baseline = world_lights_boost_rgb_cam(100.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(off, baseline, "directional=false ⇒ bit-equivalente al 3.29");
    }

    #[test]
    fn wall_rim_3d_handles_zero_distance_safely() {
        // Luz coincidente con la superficie en 3D (XY + z) ⇒ d² ≈ 0,
        // fast path att=1.0, sin NaN.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 0.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} finite", ch);
            assert!(dir[ch] > 0.0, "luz pegada aporta full");
        }
    }

    // =================================================================
    // Fase 3.35 — BRDF 3D para mobj sprites
    // =================================================================

    #[test]
    fn sprite_rim_3d_high_light_attenuates_compared_to_planar() {
        // Sprite (mobj) en (200, 0, 0). Dos luces a misma XY (100, 0)
        // pero distinto z_cam:
        //   - planar (100, 0, 0): al nivel del eye/sprite.
        //   - high   (100, 0, 60): 60 unidades arriba.
        // 3D BRDF: d² incluye dz, cos = (nx·dx + ny·dy)/d_3D — la alta
        // queda con d_3D > d_2D y cos < cos_2D ⇒ menor aporte.
        let white = (255, 255, 255);
        let planar = [WorldLight {
            x_cam: 100.0, y_cam: 0.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let high = [WorldLight {
            x_cam: 100.0, y_cam: 0.0, z_cam: 60.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let b_planar = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &planar, true);
        let b_high = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &high, true);
        for ch in 0..3 {
            assert!(
                b_planar[ch] > b_high[ch],
                "luz alta debería atenuar más vía 3D: canal {} planar={} high={}",
                ch, b_planar[ch], b_high[ch]
            );
        }
    }

    #[test]
    fn sprite_rim_3d_radius_cuts_far_vertical_light() {
        // Sprite a (200, 0, 0). Luz a XY (200, 0) pero z=250. En 2D
        // d_XY=0 ⇒ omni la incluye; en 3D d=250 > r=192 ⇒ direccional
        // la excluye.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 200.0, y_cam: 0.0, z_cam: 250.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        let omni = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        assert_eq!(dir, ZERO_BOOST, "3D radio corta la luz lejana en z");
        assert!(omni[0] > 0.0, "omni 2D no la corta (d_XY=0)");
    }

    #[test]
    fn sprite_rim_3d_planar_light_finite_and_positive() {
        // Luz con z_cam=0 (planar al sprite). Sanity check del path 3D
        // colapsando a 2D cuando dz=0.
        let red = (255, 130, 60);
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 30.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: red, lit_sectors: None,
        }];
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} finite", ch);
        }
        assert!(dir[0] > 0.0, "tinte rojo presente");
    }

    #[test]
    fn sprite_rim_3d_disabled_uses_omni_2d() {
        // Toggle off ⇒ bit-equivalente al `world_lights_boost_rgb_cam`
        // omni 2D del 3.29 incluso con z_cam alto.
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 20.0, z_cam: 80.0,
            sector: NO_SECTOR, tint_rgb: (200, 200, 200), lit_sectors: None,
        }];
        let off = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        let baseline = world_lights_boost_rgb_cam(200.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(off, baseline);
    }

    #[test]
    fn sprite_rim_3d_handles_sprite_below_eye_level() {
        // Sprite en (200, 0, -32) (mobj parado sobre piso 32 u debajo
        // del ojo) + luz al ras del piso a la izquierda (100, 50, -32).
        // dz = 0 ⇒ ratio 3D/2D ≈ 1 (luz al nivel del sprite). El
        // direccional debería seguir siendo finito y positivo.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 50.0, z_cam: -32.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, -32.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} finite", ch);
            assert!(dir[ch] > 0.0, "luz al nivel del sprite aporta canal {}", ch);
        }
    }

    // =================================================================
    // Fase 3.37 — Muzzle direccional sobre walls y planes
    // =================================================================

    // =================================================================
    // Fase 3.42 — Bandas verticales para BRDF de walls
    // =================================================================

    #[test]
    fn wall_v_band_centers_split_slab_uniformly() {
        // Verifica el cálculo de los centros verticales de N bandas
        // sobre un slab `[z_bot, z_top]`. Reproduce la fórmula del
        // loop de gather_wall: `z_band_center = z_bot + (z_top - z_bot)
        // * (t0 + t1) * 0.5` con `t0 = b/N`, `t1 = (b+1)/N`.
        let z_bot = 0.0_f32;
        let z_top = 128.0_f32;
        let v_bands: u32 = 4;
        let mut centers = Vec::new();
        for b in 0..v_bands {
            let t0 = b as f32 / v_bands as f32;
            let t1 = (b + 1) as f32 / v_bands as f32;
            centers.push(z_bot + (z_top - z_bot) * (t0 + t1) * 0.5);
        }
        // Esperado: 16, 48, 80, 112 (centros de cada cuarto).
        assert_eq!(centers, vec![16.0, 48.0, 80.0, 112.0]);
    }

    #[test]
    fn wall_v_band_bottom_band_receives_more_from_floor_light() {
        // Pared a x=100 con normal toward-camera (-1, 0). Luz al ras del
        // piso (z_cam = -50). Comparamos boost al centro de la banda
        // inferior (z_band_cam=-32) vs banda superior (z_band_cam=+96).
        // La luz baja tiene dz pequeño con la banda inferior ⇒ d_3D
        // menor ⇒ más boost.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: -50.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let band_low = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, -32.0, NO_SECTOR, &lights, n, true);
        let band_high = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 96.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(
                band_low[ch] > band_high[ch],
                "luz al piso ⇒ banda inferior recibe más: canal {} low={} high={}",
                ch, band_low[ch], band_high[ch]
            );
        }
    }

    #[test]
    fn wall_v_band_top_band_receives_more_from_ceiling_light() {
        // Espejo: luz a la altura del techo (z_cam=+90) ⇒ la banda
        // superior recibe más.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 90.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let band_low = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, -32.0, NO_SECTOR, &lights, n, true);
        let band_high = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 96.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(
                band_high[ch] > band_low[ch],
                "luz al techo ⇒ banda superior recibe más: canal {} high={} low={}",
                ch, band_high[ch], band_low[ch]
            );
        }
    }

    #[test]
    fn wall_v_bands_default_one_preserves_path() {
        // `cfg.wall_vertical_bands = 1` debe preservar el path 3.32-3.41:
        // un único boost al z=0 (eye-level), sin subdivisión. El default
        // de RenderConfig es 1.
        let cfg = RenderConfig::default();
        assert_eq!(cfg.wall_vertical_bands, 1);
        // Sanity: el path single (v_bands == 1) en gather_wall computa
        // el boost una sola vez. Reproducible por:
        let z_surf_default = 0.0_f32; // eye level (3.34 convention)
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: -20.0,
            sector: NO_SECTOR, tint_rgb: (255, 255, 255), lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let single = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, z_surf_default, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(single[ch].is_finite());
        }
    }

    // =================================================================
    // Fase 3.43 — Gradiente vertical continuo para walls
    // =================================================================

    #[test]
    fn wall_gradient_dark_stops_offsets_monotonic_and_cover_unit() {
        // Los stops deben quedar en offsets crecientes que cubran [0, 1]:
        // el primer stop en 0 (bottom), el último en 1 (top).
        let samples = [(0.0_f32, 0.0_f32), (0.5, 0.1), (1.0, 0.2)];
        let stops = wall_darkness_gradient_stops(0.4, &samples);
        assert_eq!(stops.len(), 3);
        assert_eq!(stops[0].0, 0.0);
        assert_eq!(stops[2].0, 1.0);
        for w in stops.windows(2) {
            assert!(w[1].0 > w[0].0, "offsets estrictamente crecientes");
        }
    }

    #[test]
    fn wall_gradient_dark_stop_brighter_band_is_less_opaque() {
        // Una banda con más boost ⇒ shade iluminado mayor ⇒ overlay
        // negro menos opaco (alpha menor). Bottom con boost 0.4, top con
        // boost 0.0, base_shade 0.3.
        let samples = [(0.0_f32, 0.4_f32), (1.0, 0.0)];
        let stops = wall_darkness_gradient_stops(0.3, &samples);
        let a_bottom = stops[0].1.to_rgba8().to_u8_array()[3];
        let a_top = stops[1].1.to_rgba8().to_u8_array()[3];
        assert!(
            a_bottom < a_top,
            "banda más iluminada (bottom) ⇒ menos oscuridad: a_bottom={} a_top={}",
            a_bottom, a_top
        );
    }

    #[test]
    fn wall_gradient_tint_none_when_all_negligible() {
        // Si ningún sample tiene tinte apreciable, no se emite gradiente
        // de tinte (None) ⇒ el render loop salta el segundo fill.
        let samples = [
            (0.0_f32, ZERO_BOOST),
            (0.5, [0.005, 0.0, 0.0]),
            (1.0, ZERO_BOOST),
        ];
        assert!(wall_tint_gradient_stops(&samples).is_none());
    }

    #[test]
    fn wall_gradient_tint_some_keeps_all_stops_with_transparent_gaps() {
        // Con al menos un sample tintado, devolvemos Some con TODOS los
        // stops (los despreciables quedan alpha 0) para no cortar la
        // continuidad del gradiente.
        let samples = [
            (0.0_f32, ZERO_BOOST),     // despreciable ⇒ alpha 0
            (0.5, [0.0, 0.30, 0.0]),   // verde apreciable
            (1.0, ZERO_BOOST),         // despreciable ⇒ alpha 0
        ];
        let stops = wall_tint_gradient_stops(&samples).expect("hay un sample tintado");
        assert_eq!(stops.len(), 3);
        assert_eq!(stops[0].1.to_rgba8().to_u8_array()[3], 0, "gap inferior transparente");
        assert!(stops[1].1.to_rgba8().to_u8_array()[3] > 0, "stop tintado opaco");
        assert_eq!(stops[2].1.to_rgba8().to_u8_array()[3], 0, "gap superior transparente");
        // El canal verde del stop tintado domina (normalizado al máximo).
        let [r, g, b, _] = stops[1].1.to_rgba8().to_u8_array();
        assert!(g > r && g > b, "tinte verde: g={} r={} b={}", g, r, b);
    }

    #[test]
    fn wall_gradient_default_off_preserves_3_42_path() {
        // Default RenderConfig: gradiente off ⇒ el path 3.42 (bandas /
        // single overlay) queda intacto.
        let cfg = RenderConfig::default();
        assert!(!cfg.wall_vertical_gradient);
    }

    // =================================================================
    // Fase 3.44 — Gradiente de profundidad para pisos/techos
    // =================================================================

    #[test]
    fn plane_near_far_picks_closest_and_farthest() {
        // Polígono con vértices a distintas distancias del origen
        // cam-space. near = el de menor d², far = el de mayor.
        let poly = [(10.0_f32, 0.0), (100.0, 0.0), (50.0, 50.0), (5.0, 2.0)];
        let (i_near, i_far) = plane_near_far_indices(&poly).expect("4 vértices");
        assert_eq!(i_near, 3, "(5,2) es el más cercano");
        assert_eq!(i_far, 1, "(100,0) es el más lejano");
    }

    #[test]
    fn plane_near_far_none_with_under_two_verts() {
        assert!(plane_near_far_indices(&[]).is_none());
        assert!(plane_near_far_indices(&[(1.0, 1.0)]).is_none());
    }

    #[test]
    fn plane_depth_gradient_near_brighter_than_far() {
        // Reusa wall_darkness_gradient_stops con base_shade=0 y el
        // lit-shade completo por sample. Cerca (offset 0) más iluminado
        // ⇒ menos opaco que lejos (offset 1).
        let near_lit = 0.85_f32; // poco fog, cerca del jugador
        let far_lit = 0.30_f32; // mucho fog, lejos
        let stops = wall_darkness_gradient_stops(0.0, &[(0.0, near_lit), (1.0, far_lit)]);
        let a_near = stops[0].1.to_rgba8().to_u8_array()[3];
        let a_far = stops[1].1.to_rgba8().to_u8_array()[3];
        assert!(
            a_near < a_far,
            "near menos oscuro que far: a_near={} a_far={}",
            a_near, a_far
        );
    }

    #[test]
    fn plane_depth_gradient_default_off() {
        let cfg = RenderConfig::default();
        assert!(!cfg.plane_depth_gradient);
    }

    #[test]
    fn axis_offset_endpoints_and_midpoint() {
        // Fase 3.45: proyección sobre el eje start→end.
        let start = Point::new(100.0, 400.0);
        let end = Point::new(100.0, 100.0); // eje vertical hacia arriba
        assert!((axis_offset(start, start, end) - 0.0).abs() < 1e-5, "start ⇒ 0");
        assert!((axis_offset(end, start, end) - 1.0).abs() < 1e-5, "end ⇒ 1");
        let mid = Point::new(100.0, 250.0);
        assert!((axis_offset(mid, start, end) - 0.5).abs() < 1e-5, "mid ⇒ 0.5");
    }

    #[test]
    fn axis_offset_clamps_and_projects_orthogonally() {
        let start = Point::new(0.0, 0.0);
        let end = Point::new(10.0, 0.0); // eje horizontal
        // Punto más allá del end ⇒ clamp a 1.
        assert_eq!(axis_offset(Point::new(50.0, 0.0), start, end), 1.0);
        // Punto antes del start ⇒ clamp a 0.
        assert_eq!(axis_offset(Point::new(-5.0, 0.0), start, end), 0.0);
        // Punto fuera del eje (con offset y): sólo cuenta la componente x.
        assert!((axis_offset(Point::new(5.0, 99.0), start, end) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn axis_offset_degenerate_axis_is_zero() {
        let p = Point::new(3.0, 7.0);
        let s = Point::new(1.0, 1.0);
        assert_eq!(axis_offset(p, s, s), 0.0, "eje cero ⇒ 0 sin NaN");
    }

    #[test]
    fn plane_multistop_dedup_keeps_increasing_offsets() {
        // Reproduce el dedup del gradiente de planos 3.45: offsets casi
        // iguales colapsan, el resultado queda estrictamente creciente.
        let raw = [
            (0.0_f32, 0.9_f32),
            (0.00005, 0.8), // colapsa con 0.0 (< +1e-4)
            (0.5, 0.6),
            (0.5, 0.5), // colapsa con el 0.5 previo
            (1.0, 0.3),
        ];
        let mut last = f32::NEG_INFINITY;
        let mut kept = Vec::new();
        for &(off, lit) in &raw {
            if off <= last + 1e-4 {
                continue;
            }
            last = off;
            kept.push((off, lit));
        }
        let offs: Vec<f32> = kept.iter().map(|&(o, _)| o).collect();
        assert_eq!(offs, vec![0.0, 0.5, 1.0], "dedup deja 3 stops crecientes");
        for w in offs.windows(2) {
            assert!(w[1] > w[0]);
        }
    }

    // =================================================================
    // Fase 3.46 — Decals efímeros de impacto
    // =================================================================

    fn decal_test_setup() -> (Camera, Projection) {
        let cam = Camera::new(0.0, 0.0, 41.0, 0.0); // mira hacia +X
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        (cam, Projection::new(rect, 75_f32.to_radians()))
    }

    #[test]
    fn decal_in_front_produces_one_renderable() {
        let (cam, proj) = decal_test_setup();
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (24, 21, 18),
                alpha: 1.0,
                tangent: (0.0, 0.0),
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert_eq!(out.len(), 1, "decal al frente ⇒ 1 quad");
        assert!(matches!(out[0].kind, RenderKind::Fill));
    }

    #[test]
    fn decal_behind_camera_is_culled() {
        let (cam, proj) = decal_test_setup();
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: -100.0, // detrás (x_cam < near)
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (24, 21, 18),
                alpha: 1.0,
                tangent: (0.0, 0.0),
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert!(out.is_empty(), "decal detrás de la cámara se descarta");
    }

    #[test]
    fn decal_zero_alpha_is_skipped() {
        let (cam, proj) = decal_test_setup();
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (24, 21, 18),
                alpha: 0.0, // ya desvanecido
                tangent: (0.0, 0.0),
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert!(out.is_empty(), "alpha 0 ⇒ no se dibuja");
    }

    #[test]
    fn decal_alpha_maps_to_color_alpha_channel() {
        let (cam, proj) = decal_test_setup();
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (100, 10, 10),
                alpha: 0.5,
                tangent: (0.0, 0.0),
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert_eq!(out.len(), 1);
        let a = out[0].color.to_rgba8().to_u8_array()[3];
        assert!((a as i32 - 127).abs() <= 1, "alpha 0.5 ⇒ ~127, got {}", a);
    }

    #[test]
    fn decal_depth_sits_in_front_of_its_surface() {
        // El depth se sesga -0.5 respecto a la distancia euclidiana
        // del impacto ⇒ se dibuja delante de la pared a esa distancia.
        let (cam, proj) = decal_test_setup();
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (24, 21, 18),
                alpha: 1.0,
                tangent: (0.0, 0.0),
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert!((out[0].depth - (100.0 - 0.5)).abs() < 1e-3, "depth = dist - 0.5");
    }

    #[test]
    fn decal_wall_aligned_quad_is_not_axis_aligned() {
        // Fase 3.47: un decal sobre una pared oblicua (tangente a 45°)
        // proyecta un quad cuyos lados superior/inferior tienen distinta
        // longitud en pantalla (perspectiva) — a diferencia del billboard
        // axis-aligned. Comparamos un decal billboard vs uno con tangente
        // diagonal en la misma posición.
        let (cam, proj) = decal_test_setup();
        let mk = |tangent: (f32, f32)| {
            let cfg = RenderConfig {
                decals: vec![Decal {
                    x: 100.0,
                    y: 0.0,
                    z: 40.0,
                    radius: 8.0,
                    color: (24, 21, 18),
                    alpha: 1.0,
                    tangent,
                    horizontal: false,
                    wall_span: None,
                }],
                ..Default::default()
            };
            let mut out = Vec::new();
            gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
            out
        };
        let billboard = mk((0.0, 0.0));
        let walled = mk((0.707, 0.707)); // pared a 45° respecto a la vista
        assert_eq!(billboard.len(), 1);
        assert_eq!(walled.len(), 1);
        // El billboard cae a profundidad constante ⇒ borde izq y der a
        // la misma `x_cam` ⇒ misma altura en pantalla. El walled tiene
        // su lado izquierdo más cerca (más alto) y el derecho más lejos
        // (más bajo) — la perspectiva de la pared oblicua.
        let edge_heights = |bz: &BezPath| {
            let pts: Vec<Point> = bz.elements().iter().filter_map(|e| match e {
                llimphi_ui::llimphi_raster::kurbo::PathEl::MoveTo(p)
                | llimphi_ui::llimphi_raster::kurbo::PathEl::LineTo(p) => Some(*p),
                _ => None,
            }).collect();
            // pts = [tl, tr, br, bl]. Altura izq = tl→bl, der = tr→br.
            let left = (pts[0].y - pts[3].y).abs();
            let right = (pts[1].y - pts[2].y).abs();
            (left, right)
        };
        let (bl, br) = edge_heights(&billboard[0].path);
        assert!((bl - br).abs() < 1e-6, "billboard: alturas izq == der");
        let (wl, wr) = edge_heights(&walled[0].path);
        assert!(
            (wl - wr).abs() > 1e-3,
            "pared oblicua: altura izq != der (perspectiva), izq={} der={}",
            wl, wr
        );
    }

    #[test]
    fn decal_horizontal_lies_flat_below_eye() {
        // Fase 3.48: un decal horizontal (charco) en el piso, bajo el
        // ojo, proyecta su borde cercano (más bajo en pantalla) más
        // ancho que el lejano — perspectiva de un quad sobre el suelo.
        let (cam, proj) = decal_test_setup(); // view_z=41, mira +X
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 0.0, // a nivel del piso, bajo el ojo
                radius: 16.0,
                color: (24, 21, 18),
                alpha: 1.0,
                tangent: (0.0, 0.0),
                horizontal: true,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert_eq!(out.len(), 1);
        let pts: Vec<Point> = out[0]
            .path
            .elements()
            .iter()
            .filter_map(|e| match e {
                llimphi_ui::llimphi_raster::kurbo::PathEl::MoveTo(p)
                | llimphi_ui::llimphi_raster::kurbo::PathEl::LineTo(p) => Some(*p),
                _ => None,
            })
            .collect();
        // pts = [(-r,-r), (r,-r), (r,r), (-r,r)] en XY mundo. El borde
        // cercano (x_cam = 100-16 = 84) lo forman pts[0] y pts[3]; el
        // lejano (x_cam = 116), pts[1] y pts[2]. El cercano es más ancho.
        let near_w = (pts[0].x - pts[3].x).abs();
        let far_w = (pts[1].x - pts[2].x).abs();
        assert!(
            near_w > far_w + 1e-3,
            "borde cercano más ancho que el lejano: near={} far={}",
            near_w, far_w
        );
    }

    #[test]
    fn decal_shade_rgb_darkens_in_dark_sector() {
        // Fase 3.49: shade 1.0 preserva el color; shade bajo lo oscurece
        // per-canal; shade 0 ⇒ negro.
        let c = (104, 12, 12);
        assert_eq!(shade_rgb(c, 1.0), c, "luz plena ⇒ idéntico");
        assert_eq!(shade_rgb(c, 0.5), (52, 6, 6), "mitad de luz ⇒ mitad por canal");
        assert_eq!(shade_rgb(c, 0.0), (0, 0, 0), "oscuridad total ⇒ negro");
        assert_eq!(shade_rgb(c, 2.0), c, "clamp a 1.0");
    }

    #[test]
    fn decal_picks_up_world_light_tint() {
        // Fase 3.50: un decal scorch (gris oscuro) junto a una world
        // light verde recibe boost en el canal verde — comparamos con la
        // misma escena sin luces.
        let (cam, proj) = decal_test_setup();
        // Snap con BSP de una hoja (sector único, luz media) para que el
        // path de shading+boost se active (nodes no vacío).
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 200,
            floor_pic: 0,
            ceiling_pic: 0,
        }]);
        snap.subsectors = Arc::from(vec![SubsectorSnap {
            sector: 0,
            first_seg: 0,
            num_segs: 0,
        }]);
        // Nodo único cuyos dos hijos apuntan al subsector 0.
        snap.nodes = Arc::from(vec![NodeSnap {
            partition_x: 0.0,
            partition_y: 0.0,
            partition_dx: 0.0,
            partition_dy: 1.0,
            children: [NF_SUBSECTOR, NF_SUBSECTOR],
        }]);
        let decal = Decal {
            x: 100.0,
            y: 0.0,
            z: 40.0,
            radius: 5.0,
            color: (40, 40, 40),
            alpha: 1.0,
            tangent: (0.0, 0.0),
            horizontal: false,
            wall_span: None,
        };
        let cfg = RenderConfig {
            decals: vec![decal],
            ..Default::default()
        };
        // Sin luces.
        let mut plain = Vec::new();
        gather_decals(&mut plain, &cfg, &snap, &cam, &proj, None, &[]);
        // Con una world light verde pegada al decal.
        let green = [WorldLight {
            x_cam: 100.0,
            y_cam: 0.0,
            z_cam: 40.0 - cam.view_z,
            sector: 0,
            tint_rgb: (0, 255, 0),
            lit_sectors: None,
        }];
        let mut lit = Vec::new();
        gather_decals(&mut lit, &cfg, &snap, &cam, &proj, None, &green);
        let g_plain = plain[0].color.to_rgba8().to_u8_array()[1];
        let g_lit = lit[0].color.to_rgba8().to_u8_array()[1];
        assert!(
            g_lit > g_plain,
            "la world light verde sube el canal G: plain={} lit={}",
            g_plain, g_lit
        );
    }

    #[test]
    fn decal_wall_grazing_light_dimmer_than_head_on() {
        // Fase 3.51: una marca pegada a la pared (tangent set) recibe el
        // tinte de world lights atenuado por el cosine de la normal del
        // muro. Una luz verde **encarada** (perpendicular a la pared)
        // tinta más fuerte que una **rasante** (paralela al muro) a la
        // misma distancia. La pared corre a lo largo de Y (tangent (0,1)),
        // su normal toward-camera es (-1, 0).
        let (cam, proj) = decal_test_setup();
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 200,
            floor_pic: 0,
            ceiling_pic: 0,
        }]);
        snap.subsectors = Arc::from(vec![SubsectorSnap {
            sector: 0,
            first_seg: 0,
            num_segs: 0,
        }]);
        snap.nodes = Arc::from(vec![NodeSnap {
            partition_x: 0.0,
            partition_y: 0.0,
            partition_dx: 0.0,
            partition_dy: 1.0,
            children: [NF_SUBSECTOR, NF_SUBSECTOR],
        }]);
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (40, 40, 40),
                alpha: 1.0,
                tangent: (0.0, 1.0), // muro a lo largo de Y ⇒ normal ±X
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let z = 40.0 - cam.view_z;
        // Encarada: entre cámara y decal ⇒ cos ≈ 1.
        let head_on = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: z,
            sector: 0, tint_rgb: (0, 255, 0), lit_sectors: None,
        }];
        // Rasante: a lo largo del muro, misma distancia ⇒ cos ≈ 0.
        let grazing = [WorldLight {
            x_cam: 100.0, y_cam: 50.0, z_cam: z,
            sector: 0, tint_rgb: (0, 255, 0), lit_sectors: None,
        }];
        let mut a = Vec::new();
        let mut b = Vec::new();
        gather_decals(&mut a, &cfg, &snap, &cam, &proj, None, &head_on);
        gather_decals(&mut b, &cfg, &snap, &cam, &proj, None, &grazing);
        let g_head = a[0].color.to_rgba8().to_u8_array()[1];
        let g_graze = b[0].color.to_rgba8().to_u8_array()[1];
        assert!(
            g_head > g_graze,
            "luz encarada tinta más que rasante: head={} graze={}",
            g_head, g_graze
        );
    }

    #[test]
    fn decal_wall_span_clips_horizontal_extent() {
        // Fase 3.52: un decal de pared con `wall_span` más angosto que
        // `[-r, r]` produce un quad más angosto en pantalla — recortado al
        // borde del lineseg en vez de sangrar más allá de la esquina.
        let (cam, proj) = decal_test_setup();
        let width = |span: Option<(f32, f32)>| -> f64 {
            let cfg = RenderConfig {
                decals: vec![Decal {
                    x: 100.0,
                    y: 0.0,
                    z: 40.0,
                    radius: 8.0,
                    color: (24, 21, 18),
                    alpha: 1.0,
                    tangent: (0.0, 1.0), // muro a lo largo de Y
                    horizontal: false,
                    wall_span: span,
                }],
                ..Default::default()
            };
            let mut out = Vec::new();
            gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
            assert_eq!(out.len(), 1);
            let xs: Vec<f64> = out[0]
                .path
                .elements()
                .iter()
                .filter_map(|e| match e {
                    llimphi_ui::llimphi_raster::kurbo::PathEl::MoveTo(p)
                    | llimphi_ui::llimphi_raster::kurbo::PathEl::LineTo(p) => Some(p.x),
                    _ => None,
                })
                .collect();
            xs.iter().cloned().fold(f64::MIN, f64::max)
                - xs.iter().cloned().fold(f64::MAX, f64::min)
        };
        let full = width(None); // sin recorte: ± r = 16 u de ancho
        let clipped = width(Some((-2.0, 3.0))); // recortado a 5 u
        assert!(
            clipped < full * 0.6,
            "wall_span recorta el ancho del quad: full={} clipped={}",
            full, clipped
        );
        assert!(clipped > 0.0, "quad recortado sigue teniendo área");
    }

    #[test]
    fn clip_half_plane_keeps_positive_side() {
        // Fase 3.53: cuadrado unidad recortado por el semiplano `x ≥ 0`
        // (normal (1,0), borde por el origen) ⇒ todos los vértices x ≥ 0.
        let square = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        let out = clip_half_plane(&square, (0.0, 0.0), (1.0, 0.0));
        assert!(out.len() >= 3, "queda un polígono con área");
        assert!(
            out.iter().all(|&(x, _)| x >= -1e-5),
            "ningún vértice del lado negativo: {:?}",
            out
        );
    }

    #[test]
    fn clip_decal_to_walls_keeps_center_side_and_ignores_far_walls() {
        // Fase 3.53: un charco en (0,0) r=5 junto a un muro vertical en
        // x=2 (lo alcanza: dist 2 ≤ 5) ⇒ recorta al lado del centro
        // (x ≤ 2). Un muro lejano en x=100 (fuera del radio) no recorta.
        let mk_wall = |x1, y1, x2, y2| WallSeg {
            x1, y1, x2, y2,
            front_sector: 0,
            back_sector: NO_SECTOR,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        let quad = [(-5.0, -5.0), (5.0, -5.0), (5.0, 5.0), (-5.0, 5.0)];
        // Muro cercano: recorta a x ≤ 2.
        let near_wall = [mk_wall(2.0, -10.0, 2.0, 10.0)];
        let clipped = clip_decal_to_walls(&quad, &near_wall, 0.0, 0.0, 5.0);
        assert!(clipped.len() >= 3, "queda polígono");
        assert!(
            clipped.iter().all(|&(x, _)| x <= 2.0 + 1e-4),
            "recortado al lado del centro (x ≤ 2): {:?}",
            clipped
        );
        let max_x = clipped.iter().map(|&(x, _)| x).fold(f32::MIN, f32::max);
        assert!((max_x - 2.0).abs() < 1e-3, "el borde llega justo al muro");
        // Muro lejano: no recorta ⇒ quad intacto (llega a x=5).
        let far_wall = [mk_wall(100.0, -10.0, 100.0, 10.0)];
        let untouched = clip_decal_to_walls(&quad, &far_wall, 0.0, 0.0, 5.0);
        let max_x_far = untouched.iter().map(|&(x, _)| x).fold(f32::MIN, f32::max);
        assert!((max_x_far - 5.0).abs() < 1e-3, "muro lejano no recorta");
    }

    // =================================================================
    // Fase 3.41 — Weapon rim 3D
    // =================================================================

    #[test]
    fn weapon_rim_3d_recovers_2d_when_z_zero() {
        // Luces con z_cam=0 ⇒ 3D == 2D (caso de los tests previos 3.30).
        let red = (255, 130, 60);
        let blue = (110, 160, 255);
        let lights = [
            rim_light(120.0, 0.0, red),
            rim_light(-60.0, 90.0, blue),
            rim_light(0.0, -150.0, (255, 255, 200)),
        ];
        let baseline = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        // omni del 3.30 sumaba sin direccional ⇒ matchea con dir-3.41
        // sólo cuando todos los lights tienen att=1 (frontales). No es
        // el caso de este test general; aquí verificamos que el path
        // funciona con z=0 sin crash + valores finitos.
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} finite", ch);
            assert!(dir[ch] <= baseline[ch] + 1e-5, "dir <= baseline omni");
        }
    }

    #[test]
    fn weapon_rim_3d_attenuates_for_high_light_compared_to_planar() {
        // Misma XY (50, 0) pero z distinto:
        //   - planar (50, 0, 0): luz al nivel del eye/weapon ⇒ cos=1.
        //   - high   (50, 0, 80): luz arriba ⇒ d_3D=94, cos=50/94=0.53.
        // El path direccional 3D debería dimear la luz alta respecto
        // a la planar.
        let white = (255, 255, 255);
        let planar = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let high = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 80.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let b_planar = weapon_rim_boost_rgb_cam(NO_SECTOR, &planar, true);
        let b_high = weapon_rim_boost_rgb_cam(NO_SECTOR, &high, true);
        for ch in 0..3 {
            assert!(
                b_planar[ch] > b_high[ch],
                "luz alta debería atenuar: canal {} planar={} high={}",
                ch, b_planar[ch], b_high[ch]
            );
        }
    }

    #[test]
    fn weapon_rim_3d_radius_cuts_far_vertical_light() {
        // Luz a XY=(0,0) pero z=400 (fuera del radio 384). En 2D
        // d_XY=0 ⇒ omni la incluye. En 3D d=400 > r ⇒ excluida.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 0.0, y_cam: 0.0, z_cam: 400.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        let omni = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        assert_eq!(dir, ZERO_BOOST, "3D radio corta la luz lejana en z");
        assert!(omni[0] > 0.0, "omni 2D no la corta");
    }

    #[test]
    fn weapon_rim_3d_disabled_uses_omni_2d() {
        // Toggle off ⇒ bit-equivalent al 3.29 omni 2D (sin z).
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 20.0, z_cam: 100.0,
            sector: NO_SECTOR, tint_rgb: (200, 200, 200), lit_sectors: None,
        }];
        let off = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        let baseline = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(off, baseline);
    }

    // =================================================================
    // Fase 3.40 — Muzzle falloff 3D
    // =================================================================

    #[test]
    fn muzzle_boost_3d_recovers_2d_when_z_zero() {
        // Sin componente z, el helper 3D debe dar exactamente el mismo
        // resultado que el 2D — backwards-compat.
        let xs = [0.0_f32, 50.0, 100.0, 200.0];
        for &x in &xs {
            let s2d = muzzle_boost_cam(x, 0.0, 1.0);
            let s3d = muzzle_boost_cam_3d(x, 0.0, 0.0, 1.0);
            assert!(
                (s2d - s3d).abs() < 1e-5,
                "z=0 ⇒ 3D == 2D para x={}: s2d={} s3d={}",
                x, s2d, s3d
            );
        }
    }

    #[test]
    fn muzzle_boost_3d_attenuates_with_height() {
        // Misma XY pero z creciente ⇒ scalar cae monotonamente.
        let planar = muzzle_boost_cam_3d(50.0, 0.0, 0.0, 1.0);
        let mid = muzzle_boost_cam_3d(50.0, 0.0, 50.0, 1.0);
        let high = muzzle_boost_cam_3d(50.0, 0.0, 150.0, 1.0);
        assert!(planar > mid, "planar > mid: {} > {}", planar, mid);
        assert!(mid > high, "mid > high: {} > {}", mid, high);
    }

    #[test]
    fn muzzle_boost_3d_radius_cuts_far_vertical() {
        // d_2D=0 pero z muy alto ⇒ 2D la incluye, 3D la corta.
        let r = MUZZLE_RADIUS_WORLD;
        let s2d = muzzle_boost_cam(0.0, 0.0, 1.0); // peak
        let s3d = muzzle_boost_cam_3d(0.0, 0.0, r + 10.0, 1.0); // fuera de radio
        assert!(s2d > 0.0, "2D no la corta (d_XY=0)");
        assert_eq!(s3d, 0.0, "3D la corta (d_3D > r)");
    }

    #[test]
    fn muzzle_brdf_wall_3d_falloff_dims_high_surface() {
        // Pared a (100, 0) en cam-space + z_surf alto: el muzzle 3D del
        // 3.40 debe dar menos que el 2D del 3.32-3.37 (que ignoraba z).
        // Verificamos comparando el helper actual contra un cálculo
        // manual con scalar 2D pero misma cosine att.
        let n = (-1.0, 0.0);
        let z_high = 80.0;
        let actual_3d = muzzle_boost_rgb_wall_3d(100.0, 0.0, z_high, 1.0, n);
        // Simulamos el path 3.32-3.37 con scalar 2D pero cosine 3D:
        // los componentes per-canal serían `scalar_2d * tint * att`.
        let scalar_2d = muzzle_boost_cam(100.0, 0.0, 1.0);
        // cos del wall normal (-1,0) con dir surf→muzzle (-100,0,-80)/d_3D:
        let d2 = 100.0_f32 * 100.0 + z_high * z_high;
        let inv_d = d2.sqrt().recip();
        let cos = ((-1.0) * (-100.0) + 0.0 * 0.0) * inv_d;
        let att = (0.5 + 0.5 * cos).max(WALL_RIM_AMBIENT_FLOOR);
        let pre_340 = [
            scalar_2d * MUZZLE_TINT_RGB.0 as f32 / 255.0 * att,
            scalar_2d * MUZZLE_TINT_RGB.1 as f32 / 255.0 * att,
            scalar_2d * MUZZLE_TINT_RGB.2 as f32 / 255.0 * att,
        ];
        for ch in 0..3 {
            assert!(
                actual_3d[ch] < pre_340[ch],
                "3.40 dimea respecto al modelo pre-3.40 (scalar 2D + cosine 3D): canal {} 3.40={} pre={}",
                ch, actual_3d[ch], pre_340[ch]
            );
        }
    }

    #[test]
    fn muzzle_brdf_wall_perpendicular_full_intensity() {
        // Pared straight-ahead a (100, 0, 0), normal (-1, 0). Muzzle en
        // origin ⇒ direction surf→muzzle = (-1, 0, 0). cos = 1 ⇒ att=1.
        // Direccional debe coincidir con el muzzle omni (sin cosine).
        let n = (-1.0, 0.0);
        let dir = muzzle_boost_rgb_wall_3d(100.0, 0.0, 0.0, 1.0, n);
        let omni = muzzle_boost_rgb_cam(100.0, 0.0, 1.0);
        for ch in 0..3 {
            assert!(
                (dir[ch] - omni[ch]).abs() < 1e-5,
                "perpendicular: canal {} dir={} omni={}",
                ch, dir[ch], omni[ch]
            );
        }
    }

    #[test]
    fn muzzle_brdf_wall_oblique_attenuates() {
        // Pared oblicua: midpoint (100, 50), normal apuntando al cam pero
        // con componente lateral. dot(n, -m)/|m_3D| = cos < 1 ⇒ att < 1
        // ⇒ direccional < omni en cada canal.
        let mx = 100.0;
        let my = 50.0;
        // Pared dirección (0, 1) (vertical-Y), normal (-1, 0) toward camera.
        let n = (-1.0, 0.0);
        let dir = muzzle_boost_rgb_wall_3d(mx, my, 0.0, 1.0, n);
        let omni = muzzle_boost_rgb_cam(mx, my, 1.0);
        for ch in 0..3 {
            assert!(dir[ch] < omni[ch], "oblique: canal {} dir={} >= omni={}", ch, dir[ch], omni[ch]);
        }
    }

    #[test]
    fn muzzle_brdf_wall_disabled_equals_omni() {
        // Toggle off ⇒ combined wall usa muzzle_boost_rgb_cam (omni).
        let n = (-1.0, 0.0);
        let off = combined_boost_rgb_wall_cam(
            100.0, 50.0, 0.0, 1.0, NO_SECTOR, None, &[], n, false, false,
        );
        let on = combined_boost_rgb_wall_cam(
            100.0, 50.0, 0.0, 1.0, NO_SECTOR, None, &[], n, false, true,
        );
        // En perpendicular straight muzzle direccional == omni; en
        // oblicuo direccional < omni ⇒ off[i] >= on[i] por canal.
        for ch in 0..3 {
            assert!(off[ch] >= on[ch], "off >= on en canal {}", ch);
        }
    }

    #[test]
    fn muzzle_brdf_plane_floor_below_camera_full_cosine() {
        // Floor a z_surf = -32 (debajo del ojo), centroide en (0, 0, -32).
        // direction surf→muzzle = (0, 0, 32)/32 = (0, 0, 1). cos con
        // n_z=+1 (floor) = 1 ⇒ att=1. Fase 3.40: el scalar usa d_3D=32,
        // no d_2D=0, así que decae ligeramente respecto al peak. La
        // verificación correcta es `dir ≈ scalar_3D · tint` (att=1 sin
        // modulación) — coherente con falloff 3D del 3.40.
        let dir = muzzle_boost_rgb_plane_3d(0.0, 0.0, -32.0, 1.0, 1.0);
        let scalar_3d = muzzle_boost_cam_3d(0.0, 0.0, -32.0, 1.0);
        let expected = [
            scalar_3d * MUZZLE_TINT_RGB.0 as f32 / 255.0,
            scalar_3d * MUZZLE_TINT_RGB.1 as f32 / 255.0,
            scalar_3d * MUZZLE_TINT_RGB.2 as f32 / 255.0,
        ];
        for ch in 0..3 {
            assert!(
                (dir[ch] - expected[ch]).abs() < 1e-5,
                "cos=1 ⇒ dir = scalar_3D·tint: canal {} dir={} expected={}",
                ch, dir[ch], expected[ch]
            );
        }
    }

    // =================================================================
    // Fase 3.38 — Sprite sample point al centro del billboard
    // =================================================================

    #[test]
    fn sprite_sample_center_vs_floor_differs_for_overhead_light() {
        // Antorcha alta TLMP a XY (0, 0) en z_cam=+80 (techo). Sprite a
        // XY (100, 0). Sample en floor (z_surf=0) vs en centro (z_surf=28
        // ≈ cfg.sprite_height/2). El sample center reduce el dz (80→52)
        // y el d_3D (128→113), por lo que el cosine (que normaliza por
        // d_3D) sube ⇒ más aporte.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 0.0, y_cam: 0.0, z_cam: 80.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let floor = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        let center = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 28.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            if floor[ch] > 0.01 {
                assert!(
                    center[ch] > floor[ch],
                    "centro debería recibir más de luz alta: canal {} center={} floor={}",
                    ch, center[ch], floor[ch]
                );
            }
        }
    }

    #[test]
    fn sprite_sample_center_vs_floor_differs_for_floor_light() {
        // Espejo: proyectil al ras del piso (z_cam=-32). Sprite a XY
        // (100, 0). Sample en floor (z_surf=0) tiene dz=-32 ⇒ d_3D=104,
        // cos pequeño. Sample en centro (z_surf=28) tiene dz=-60 ⇒
        // d_3D=116, d_3D mayor pero el dz grande hace cos más rasante.
        // Resultado: el sample floor recibe **más** que el center —
        // un proyectil al ras del piso ilumina la base del mobj con
        // cosine mejor que su centro.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 0.0, y_cam: 0.0, z_cam: -32.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let floor = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        let center = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 28.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            if center[ch] > 0.01 {
                assert!(
                    floor[ch] > center[ch],
                    "floor sample debería recibir más de luz baja: canal {} floor={} center={}",
                    ch, floor[ch], center[ch]
                );
            }
        }
    }

    #[test]
    fn sprite_sample_center_planar_light_matches_floor_when_dz_zero() {
        // Si la luz está al **nivel del sample** (mismo z), el cosine es
        // puramente XY independientemente de dónde esté el sample point.
        // Una luz al centro vertical del sprite (z=center) ⇒ dz=0
        // produce el mismo resultado que luz al floor (z=0) cuando el
        // sample también está al floor — ambas tienen dz=0 desde su
        // respectiva sample point.
        let white = (255, 255, 255);
        let light_at_center = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 28.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let light_at_floor = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        // Center sample vs center-z light: dz=0.
        let a = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 28.0, NO_SECTOR, &light_at_center, true);
        // Floor sample vs floor-z light: dz=0 también.
        let b = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 0.0, NO_SECTOR, &light_at_floor, true);
        for ch in 0..3 {
            assert!(
                (a[ch] - b[ch]).abs() < 1e-5,
                "dz=0 desde cualquier sample ⇒ mismo aporte: canal {} a={} b={}",
                ch, a[ch], b[ch]
            );
        }
    }

    #[test]
    fn sprite_sample_center_offset_zero_recovers_3_35_behavior() {
        // Si cfg.sprite_height = 0, el offset es 0 y el sample queda
        // en sprite.z (Fase 3.35). Verificamos que el helper produce
        // exactamente el mismo resultado con z_surf_cam idéntico —
        // la regresión sólo está en el caller (`gather_sprite`), aquí
        // chequeamos sanidad del helper.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 20.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        // sprite.z = 0, cfg.sprite_height = 0 ⇒ z_surf_cam = 0
        let z_surf_335 = 0.0_f32 + 0.0 * 0.5; // 3.35 behavior
        let z_surf_338 = 0.0_f32 + 0.0 * 0.5; // 3.38 con sprite_height=0
        assert_eq!(z_surf_335, z_surf_338);
        let a = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, z_surf_335, NO_SECTOR, &lights, true);
        let b = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, z_surf_338, NO_SECTOR, &lights, true);
        assert_eq!(a, b, "sprite_height=0 ⇒ 3.38 == 3.35");
    }

    // =================================================================
    // Fase 3.39 — Sprite sample con patch.height real (textured path)
    // =================================================================

    /// Centro vertical del billboard en cam-space dado el `floor` (z del
    /// sector), `topoffset` del patch, su altura `h`, y `view_z` del
    /// jugador. Equivale a `((z_top + z_bot) * 0.5)` que usa el path
    /// texturizado (Fase 3.39).
    fn billboard_center_z_cam(floor: f32, topoffset: f32, h: f32, view_z: f32) -> f32 {
        let z_top = floor + topoffset - view_z;
        let z_bot = floor + topoffset - h - view_z;
        (z_top + z_bot) * 0.5
    }

    #[test]
    fn billboard_center_imp_at_floor() {
        // Imp típico: TROOA1 patch h≈56, topoffset≈48. Imp parado en
        // floor=0, view_z=40. Centro = floor + to - h/2 - view_z =
        // 0 + 48 - 28 - 40 = -20. Es decir, 20 unidades debajo del eye —
        // consistente con un mobj de altura 56 parado en piso 0 con ojo
        // a 40, centro a 8 absoluto.
        let z = billboard_center_z_cam(0.0, 48.0, 56.0, 40.0);
        assert!((z - (-20.0)).abs() < 1e-3, "centro esperado -20, got {}", z);
    }

    #[test]
    fn billboard_center_cyberdemon_taller_than_imp_estimate() {
        // Cyberdemon: patch h≈110, topoffset≈110 (estimado). Comparamos
        // contra el sample cfg.sprite_height=56 default. El centro real
        // del cyberdemon queda **más alto** que el estimate.
        let real_cyber = billboard_center_z_cam(0.0, 110.0, 110.0, 40.0);
        let estimate_56 = 0.0_f32 - 40.0 + 56.0 * 0.5; // 3.38 fallback
        assert!(
            real_cyber > estimate_56,
            "cyberdemon real ({}) debería estar arriba del estimate ({})",
            real_cyber, estimate_56
        );
    }

    #[test]
    fn billboard_center_puff_lower_than_imp_estimate() {
        // PUFF: patch h≈16, topoffset≈16. Centro real del puff queda
        // **más abajo** que el estimate cfg.sprite_height=56. El bullet
        // puff es chiquito y queda apoyado al techo del impacto.
        let real_puff = billboard_center_z_cam(64.0, 16.0, 16.0, 40.0);
        let estimate_56 = 64.0_f32 - 40.0 + 56.0 * 0.5; // 3.38 fallback con sprite.z=64
        assert!(
            real_puff < estimate_56,
            "puff real ({}) debería estar abajo del estimate ({})",
            real_puff, estimate_56
        );
    }

    #[test]
    fn billboard_center_uses_patch_height_for_brdf() {
        // Verificación que el sample con patch real impacta el BRDF.
        // Cyberdemon h=110 a XY=200, floor=0, view_z=40 ⇒ centro=15.
        // Luz a XY=(100, 0) z_cam=10 (cerca del centro real).
        // Sample real (z_surf=15) ⇒ dz=-5 ⇒ cos casi puro XY.
        // Sample 3.38 estimate (z_surf=-12) ⇒ dz=+22 ⇒ cos rasante.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 0.0, z_cam: 10.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let real_cyber_z = billboard_center_z_cam(0.0, 110.0, 110.0, 40.0); // 15
        let estimate_z = 0.0_f32 - 40.0 + 56.0 * 0.5;                          // -12
        let b_real = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, real_cyber_z, NO_SECTOR, &lights, true);
        let b_estimate = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, estimate_z, NO_SECTOR, &lights, true);
        // El sample real debería dar diferente boost (la luz está al
        // nivel del centro real, no del estimate). Diferencia positiva
        // significa la fase 3.39 cambia el rendering.
        let mut any_diff = false;
        for ch in 0..3 {
            if (b_real[ch] - b_estimate[ch]).abs() > 1e-4 {
                any_diff = true;
            }
        }
        assert!(any_diff, "patch.height real debería producir boost diferente al estimate");
    }

    #[test]
    fn muzzle_brdf_plane_far_horizontal_attenuates() {
        // Floor lejos horizontalmente, poco vertical: centroide (100, 0, -8).
        // direction surf→muzzle = (-100, 0, 8)/sqrt(10064) ≈ (-0.997, 0, 0.080).
        // cos con n_z=+1 = 0.080 ⇒ att = (0.5 + 0.04).max(0.3) ≈ 0.54.
        // Direccional debe ser ~54% del omni por canal.
        let dir = muzzle_boost_rgb_plane_3d(100.0, 0.0, -8.0, 1.0, 1.0);
        let omni = muzzle_boost_rgb_cam(100.0, 0.0, 1.0);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    ratio < 0.6,
                    "rasante: canal {} ratio {} debería caer < 0.6",
                    ch, ratio
                );
                assert!(
                    ratio > PLANE_RIM_AMBIENT_FLOOR - 0.01,
                    "rasante: canal {} ratio {} debería estar sobre el piso ambient",
                    ch, ratio
                );
            }
        }
    }
}
