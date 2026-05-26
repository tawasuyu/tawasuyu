//! `supay-scene` — snapshots inmutables del estado visible de Doom por
//! tick.
//!
//! Cada [`SceneSnapshot`] describe el mundo en el tick `N`: dónde está
//! el jugador, qué paredes hay, alturas de sectores, sprites visibles.
//! El renderer (Fase 3) consume dos snapshots consecutivos y los
//! interpola con [`interpolate`] para alcanzar 144+ Hz suaves sobre la
//! simulación bit-exact de 35 Hz que vive en `supay-core`.
//!
//! ## Hardline
//!
//! - **Inmutable.** Un snapshot tomado en el tick N no cambia jamás —
//!   `Arc<[T]>` para listas, clonar es O(1).
//! - **Lectura pura.** Construir un snapshot no toca el motor; mutarlo
//!   tampoco. Es la base del contrato del proyecto: las demos `.lmp`
//!   reproducen bit-exact independientemente del renderer.
//! - **Unidades Doom.** Conservamos las unidades originales (1 unit ≈
//!   1 pulgada, ángulo en radianes desde +X antihorario). El renderer
//!   aplica la escala que quiera.
//!
//! ## Modelo de tiempo
//!
//! El motor produce un snapshot por tick (35 Hz). El renderer mantiene
//! una [`SnapshotPair`] (prev + next) y calcula `alpha ∈ [0, 1]` con
//! `alpha = (now - tick_next_start) / tick_period`. Cuando un tick
//! nuevo arriba, `next` se mueve a `prev` y el snapshot recién llegado
//! ocupa `next`.
//!
//! ## Lo que NO está acá
//!
//! - Texturas / lumps WAD → resolver por id desde el renderer.
//! - BSP / segs / subsectors → necesarios para front-to-back ordering;
//!   los expondrá una Fase 2.1 cuando el renderer 3D los demande.
//! - Audio → fuera de scope; vive en `supay-audio` cuando llegue Fase 4.

#![forbid(unsafe_code)]

use std::sync::Arc;

/// Estado del jugador local (multiplayer queda fuera de scope).
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerSnap {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Ángulo en radianes (0 = mirando +X, sentido antihorario).
    pub angle: f32,
    /// Altura de la cámara por encima del piso del sector (incluye el
    /// view-bob de caminata). En Doom default ≈ 41.
    pub view_height: f32,
}

impl Default for PlayerSnap {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            angle: 0.0,
            view_height: 41.0,
        }
    }
}

/// Una linedef del mapa con sus dos vértices + referencias a sectores.
///
/// Las paredes en Doom no se mueven entre ticks — lo que cambia es la
/// altura de los sectores que las flanquean (puertas, ascensores). Por
/// eso la lista de walls puede tomarse del snapshot más reciente sin
/// interpolar; los cambios visibles vienen vía sectores.
#[derive(Clone, Debug, PartialEq)]
pub struct WallSeg {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    /// Índice en `SceneSnapshot::sectors` del sector al frente
    /// (lado v1→v2 a la derecha del seg).
    pub front_sector: u32,
    /// Sector detrás. [`NO_SECTOR`] = pared sólida sin lado trasero.
    pub back_sector: u32,
    /// Flags Doom (`ML_BLOCKING`, `ML_TWOSIDED`, `ML_DONTPEGTOP`, etc.).
    pub flags: u32,
    /// Texturas asignadas a la pared, por sidedef + kind (sin alocación).
    /// Layout: `[front_mid, front_up, front_lo, back_mid, back_up, back_lo]`.
    /// Cada slot son 8 chars (null-padded) del nombre del lump
    /// TEXTURE1. Todo cero = sin textura asignada (slot vacío Doom
    /// "no texture", convención id 0).
    pub textures: [[u8; 8]; 6],
    /// `sidedef.textureoffset` — desplazamiento horizontal de la
    /// textura, en unidades Doom. Indexado `[front, back]`. Doom usa
    /// este offset para alinear texturas entre paredes adyacentes:
    /// el U de un pixel a distancia `d` de `v1` es `tex_x_offsets[side] + d`
    /// (mod tex_width). Sin esto, las costuras saltan cuando dos
    /// paredes consecutivas usan la misma textura.
    pub tex_x_offsets: [f32; 2],
    /// `sidedef.rowoffset` — desplazamiento vertical de la textura,
    /// en unidades Doom. Indexado `[front, back]`. Se combina con la
    /// convención de pegging (controlada por `flags` `ML_DONTPEGTOP` /
    /// `ML_DONTPEGBOTTOM`) para decidir dónde "ancla" la textura
    /// verticalmente cada kind (mid/upper/lower).
    pub tex_y_offsets: [f32; 2],
}

/// Flag `ML_DONTPEGTOP` de Doom: cuando set, la textura **upper** se
/// "pegga" al techo del front sector en vez de al techo del back
/// (el bottom del opening). Usado para que las puertas no muevan su
/// textura cuando suben.
pub const ML_DONTPEGTOP: u32 = 0x0008;

/// Flag `ML_DONTPEGBOTTOM` de Doom: cuando set, la textura **middle**
/// (one-sided) o **lower** se "pegga" al piso / techo del sector en
/// vez del default. Usado para que los pasos de ascensor no muevan
/// su textura cuando suben.
pub const ML_DONTPEGBOTTOM: u32 = 0x0010;

/// Helper: extrae el nombre de una entrada `[u8; 8]` como string ascii
/// (recortando en el primer 0). Devuelve `None` si está vacío.
pub fn texture_name(slot: &[u8; 8]) -> Option<&str> {
    let end = slot.iter().position(|&c| c == 0).unwrap_or(8);
    if end == 0 {
        return None;
    }
    std::str::from_utf8(&slot[..end]).ok()
}

/// Marca de "sin sector trasero" en [`WallSeg::back_sector`].
pub const NO_SECTOR: u32 = u32::MAX;

/// Sentinel para [`SceneSnapshot::sky_pic`] cuando el motor aún no
/// resolvió `skyflatnum` (mapa todavía sin cargar) o estamos en stub.
pub const NO_SKY_PIC: u16 = 0xFFFF;

/// Estado de los overlays de pantalla del jugador. Doom intercambia
/// PLAYPAL[1..13] cuando algo de esto está activo (red flash al daño,
/// yellow al pickup, green con radsuit, white con invuln); como
/// sampleamos siempre con PLAYPAL[0], esto se convierte en overlay
/// alpha sobre el frame final en el renderer.
///
/// Los valores son los counters internos del motor — la conversión
/// a alpha vive en `supay-render-llimphi`. Mantenerlos crudos nos da
/// flexibilidad de ajustar la presentación sin tocar la captura.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerOverlays {
    /// 0..100, +N por hp de daño, decae 1/tick. Drives el red flash.
    pub damage_count: u16,
    /// 0..32, +6/+12 por pickup, decae 1/tick. Drives el yellow flash.
    pub bonus_count: u16,
    /// Tics restantes de invulnerabilidad. >0 = activo. En los últimos
    /// 32 tics blinkea (el motor expone el valor, no el blink); el
    /// renderer aplica el blink basado en `tick`.
    pub power_invuln: u32,
    /// Tics restantes del traje anti-radiación.
    pub power_radsuit: u32,
}

/// Una hoja convexa del BSP — referencia a un sector y un rango
/// contiguo en [`SceneSnapshot::segs`] (`first_seg`, `num_segs`).
///
/// Los segs en una hoja forman una cadena ordenada (CCW por la
/// convención de Doom) que bordea parcialmente el polígono convexo del
/// subsector. Algunos lados pueden estar bordeados por particiones BSP
/// sin seg correspondiente — en esos casos la cadena no cierra el
/// polígono completo y el renderer asume que el subsector vecino del
/// mismo sector cubre el hueco.
#[derive(Clone, Debug, PartialEq)]
pub struct SubsectorSnap {
    pub sector: u32,
    pub first_seg: u32,
    pub num_segs: u32,
}

/// Un lineseg del mapa — v1 y v2 en coordenadas Doom (float, 1 unit ≈
/// 1 pulgada). Compartido por todos los subsectors que lo referencian.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegSnap {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

/// Flag de Doom: si un `NodeSnap::children[i]` tiene este bit set, el
/// hijo es un subsector (`index = child & !NF_SUBSECTOR`). Si no está set,
/// es otro nodo interno del árbol (`index = child`).
pub const NF_SUBSECTOR: u16 = 0x8000;

/// Un nodo interno del árbol BSP — partición + dos hijos.
///
/// La línea de partición es `(x, y) + t·(dx, dy)`. La convención Doom
/// para decidir de qué lado cae un punto `(px, py)`:
///
/// ```text
/// side = dx·(py - y) - dy·(px - x)
/// ```
///
/// `side > 0` → front (children[0]), `side < 0` → back (children[1]),
/// `side == 0` → arbitrario (Doom decide por dx ≷ 0 / dy ≷ 0).
///
/// La raíz del árbol es `nodes[len - 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeSnap {
    /// Origen de la partición.
    pub partition_x: f32,
    pub partition_y: f32,
    /// Dirección de la partición.
    pub partition_dx: f32,
    pub partition_dy: f32,
    /// Hijos: front (children[0]) y back (children[1]).
    /// Bit 15 ([`NF_SUBSECTOR`]) set → subsector; sino → otro nodo.
    pub children: [u16; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct SectorSnap {
    pub floor_height: f32,
    pub ceiling_height: f32,
    /// Brightness 0..255 (clamp aplicado por el productor).
    pub light_level: u8,
    /// Índice del lump de textura del piso (`R_FlatNumForName` resuelve).
    pub floor_pic: u16,
    pub ceiling_pic: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpriteSnap {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub angle: f32,
    /// `spritenum_t` de Doom (e.g. `SPR_TROO`, `SPR_PLAY`). Índice en
    /// `sprites[]` del motor.
    pub sprite: u16,
    /// Frame actual (bit 0..6 = índice de frame, bit 7 = full bright).
    pub frame: u8,
    /// Sector donde está parado el mobj (para iluminación dinámica
    /// basada en `sectors[sector].light_level`).
    pub sector: u32,
}

/// Snapshot inmutable del estado visual del tick `tick`.
///
/// Las listas son `Arc<[T]>` para que el renderer pueda mantener dos
/// snapshots vivos sin pagar copia — clonar es bumping refcount.
#[derive(Clone, Debug)]
pub struct SceneSnapshot {
    pub tick: u64,
    pub player: PlayerSnap,
    pub walls: Arc<[WallSeg]>,
    pub sectors: Arc<[SectorSnap]>,
    pub sprites: Arc<[SpriteSnap]>,
    /// Fase 3.2: subsectors del BSP, cada uno apuntando a un rango
    /// contiguo de `segs`. Si está vacío, el renderer cae al modo
    /// "fake floor" de 3.1 (trapezoides per-pared).
    pub subsectors: Arc<[SubsectorSnap]>,
    pub segs: Arc<[SegSnap]>,
    /// Fase 3.13: árbol BSP del mapa. La raíz es el último elemento.
    /// Vacío en modo stub o antes de que cargue el mapa — el renderer
    /// cae al sort euclidiano clásico si esto está vacío.
    pub nodes: Arc<[NodeSnap]>,
    /// Índice del flat que el motor trata como "cielo" (ceiling_pic
    /// con este valor → renderer pinta sky en vez de techo sólido).
    /// [`NO_SKY_PIC`] = stub o mapa no cargado.
    pub sky_pic: u16,
    /// Fase 3.14: counters del jugador para overlays de pantalla
    /// (red flash, yellow flash, etc.). Default = sin overlays.
    pub player_overlays: PlayerOverlays,
}

impl Default for SceneSnapshot {
    fn default() -> Self {
        Self::empty(0)
    }
}

impl SceneSnapshot {
    /// Snapshot vacío con un `tick` dado — útil para arrancar antes de
    /// que el motor haya cargado un mapa.
    pub fn empty(tick: u64) -> Self {
        Self {
            tick,
            player: PlayerSnap::default(),
            walls: Arc::from(Vec::<WallSeg>::new()),
            sectors: Arc::from(Vec::<SectorSnap>::new()),
            sprites: Arc::from(Vec::<SpriteSnap>::new()),
            subsectors: Arc::from(Vec::<SubsectorSnap>::new()),
            segs: Arc::from(Vec::<SegSnap>::new()),
            nodes: Arc::from(Vec::<NodeSnap>::new()),
            sky_pic: NO_SKY_PIC,
            player_overlays: PlayerOverlays::default(),
        }
    }
}

/// Buffer rotatorio de los dos últimos snapshots. El renderer consulta
/// [`Self::prev`] y [`Self::next`] e interpola con [`interpolate`].
///
/// Patrón de uso desde el host:
///
/// ```
/// # use supay_scene::{SnapshotPair, SceneSnapshot, interpolate};
/// let mut pair = SnapshotPair::new();
/// // por tick (35 Hz):
/// pair.push(SceneSnapshot::empty(1));
/// pair.push(SceneSnapshot::empty(2));
/// // por frame (144 Hz):
/// if let (Some(p), Some(n)) = (pair.prev(), pair.next()) {
///     let _frame = interpolate(p, n, 0.5);
/// }
/// ```
#[derive(Default, Clone)]
pub struct SnapshotPair {
    prev: Option<SceneSnapshot>,
    next: Option<SceneSnapshot>,
}

impl SnapshotPair {
    pub fn new() -> Self {
        Self::default()
    }

    /// Empuja un snapshot nuevo. El anterior `next` pasa a `prev`; el
    /// `prev` viejo se descarta.
    pub fn push(&mut self, snap: SceneSnapshot) {
        self.prev = self.next.take();
        self.next = Some(snap);
    }

    pub fn prev(&self) -> Option<&SceneSnapshot> {
        self.prev.as_ref()
    }

    pub fn next(&self) -> Option<&SceneSnapshot> {
        self.next.as_ref()
    }

    /// `true` si ya hay dos snapshots — el renderer puede interpolar.
    pub fn is_ready(&self) -> bool {
        self.prev.is_some() && self.next.is_some()
    }
}

/// Snapshot interpolado entre `prev` y `next` con factor `alpha ∈ [0, 1]`.
///
/// Reglas:
/// - **Player** (x, y, z, view_height): lineal. Ángulo por arc-shortest
///   (maneja wraparound 2π — sin esto, girar de 350° a 10° haría que el
///   jugador "haga un giro largo" durante la interpolación).
/// - **Sectors**: lineal en alturas + light_level. Texturas (`floor_pic`,
///   `ceiling_pic`) tomadas de `next` — no se interpolan, son enteros.
/// - **Walls**: tomadas de `next` directamente. En Doom las linedefs
///   nunca se mueven (los movimientos visuales son cambios en
///   `sector.height`, que sí interpolamos).
/// - **Sprites**: si las longitudes coinciden, interpolan posición y
///   ángulo por índice — asumimos que el productor emite los mobjs en
///   el orden estable de `thinkercap`. Si difiere (spawn / destroy entre
///   ticks), tomamos `next` puro y no hay glitch visual visible: el
///   sprite nuevo aparece en su posición real, el viejo desaparece.
///
/// `alpha` se clampea a `[0, 1]` — pasar valores fuera no rompe pero
/// devuelve el extremo.
pub fn interpolate(prev: &SceneSnapshot, next: &SceneSnapshot, alpha: f32) -> SceneSnapshot {
    let a = alpha.clamp(0.0, 1.0);
    let player = PlayerSnap {
        x: lerp(prev.player.x, next.player.x, a),
        y: lerp(prev.player.y, next.player.y, a),
        z: lerp(prev.player.z, next.player.z, a),
        angle: lerp_angle(prev.player.angle, next.player.angle, a),
        view_height: lerp(prev.player.view_height, next.player.view_height, a),
    };
    let sectors: Arc<[SectorSnap]> = if prev.sectors.len() == next.sectors.len() {
        let v: Vec<SectorSnap> = prev
            .sectors
            .iter()
            .zip(next.sectors.iter())
            .map(|(p, n)| SectorSnap {
                floor_height: lerp(p.floor_height, n.floor_height, a),
                ceiling_height: lerp(p.ceiling_height, n.ceiling_height, a),
                light_level: lerp(p.light_level as f32, n.light_level as f32, a)
                    .round()
                    .clamp(0.0, 255.0) as u8,
                floor_pic: n.floor_pic,
                ceiling_pic: n.ceiling_pic,
            })
            .collect();
        v.into()
    } else {
        next.sectors.clone()
    };
    let sprites: Arc<[SpriteSnap]> = if prev.sprites.len() == next.sprites.len() {
        let v: Vec<SpriteSnap> = prev
            .sprites
            .iter()
            .zip(next.sprites.iter())
            .map(|(p, n)| SpriteSnap {
                x: lerp(p.x, n.x, a),
                y: lerp(p.y, n.y, a),
                z: lerp(p.z, n.z, a),
                angle: lerp_angle(p.angle, n.angle, a),
                sprite: n.sprite,
                frame: n.frame,
                sector: n.sector,
            })
            .collect();
        v.into()
    } else {
        next.sprites.clone()
    };
    SceneSnapshot {
        tick: next.tick,
        player,
        walls: next.walls.clone(),
        sectors,
        sprites,
        // Topología BSP: nunca se interpola — los subsectores, segs y
        // nodos son estables por mapa cargado. Tomamos `next` directamente.
        subsectors: next.subsectors.clone(),
        segs: next.segs.clone(),
        nodes: next.nodes.clone(),
        sky_pic: next.sky_pic,
        // Overlays: counters integers — tomamos `next` puro. Interpolar
        // un counter no tiene sentido visual (el flash sube/baja en
        // pasos discretos por tick); el cambio entre snapshots se nota
        // como cambio de alpha del overlay.
        player_overlays: next.player_overlays,
    }
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Interpola dos ángulos por el arco más corto. Maneja wraparound 2π
/// para que pasar de 350° a 10° atraviese 0°, no 180°.
#[inline]
fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let two_pi = std::f32::consts::TAU;
    let pi = std::f32::consts::PI;
    let mut delta = (b - a) % two_pi;
    if delta > pi {
        delta -= two_pi;
    } else if delta < -pi {
        delta += two_pi;
    }
    a + delta * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_pair_rotates() {
        let mut pair = SnapshotPair::new();
        assert!(!pair.is_ready());
        pair.push(SceneSnapshot::empty(1));
        assert!(!pair.is_ready()); // sólo uno todavía
        pair.push(SceneSnapshot::empty(2));
        assert!(pair.is_ready());
        assert_eq!(pair.prev().unwrap().tick, 1);
        assert_eq!(pair.next().unwrap().tick, 2);
        pair.push(SceneSnapshot::empty(3));
        assert_eq!(pair.prev().unwrap().tick, 2);
        assert_eq!(pair.next().unwrap().tick, 3);
    }

    #[test]
    fn lerp_midpoint() {
        let p = PlayerSnap {
            x: 0.0,
            y: 10.0,
            z: 0.0,
            angle: 0.0,
            view_height: 41.0,
        };
        let n = PlayerSnap {
            x: 10.0,
            y: 20.0,
            z: 2.0,
            angle: 1.0,
            view_height: 43.0,
        };
        let prev = SceneSnapshot {
            tick: 0,
            player: p,
            ..Default::default()
        };
        let next = SceneSnapshot {
            tick: 1,
            player: n,
            ..Default::default()
        };
        let mid = interpolate(&prev, &next, 0.5);
        assert!((mid.player.x - 5.0).abs() < 1e-5);
        assert!((mid.player.y - 15.0).abs() < 1e-5);
        assert!((mid.player.z - 1.0).abs() < 1e-5);
        assert!((mid.player.angle - 0.5).abs() < 1e-5);
        assert!((mid.player.view_height - 42.0).abs() < 1e-5);
        // `next.tick` se preserva — el snapshot interpolado vive
        // conceptualmente en next.
        assert_eq!(mid.tick, 1);
    }

    #[test]
    fn lerp_angle_shortest_arc() {
        // 350° (5.846) → 10° (0.175). Diferencia naive = -5.671 rad;
        // shortest arc = +0.611 rad. Midpoint debería estar cerca de 0°
        // (o 360°, equivalente).
        let a = 350.0_f32.to_radians();
        let b = 10.0_f32.to_radians();
        let mid = lerp_angle(a, b, 0.5);
        // Cae cerca de 0° o 2π — normalizamos para verificar.
        let n = ((mid % std::f32::consts::TAU) + std::f32::consts::TAU) % std::f32::consts::TAU;
        let target = 0.0_f32; // 0° tras normalizar
        // Aceptamos cualquiera de los dos polos de equivalencia.
        let dist = n.min(std::f32::consts::TAU - n);
        assert!(dist < 0.01, "mid={mid} normalised={n} target={target}");
    }

    #[test]
    fn alpha_clamps_outside_range() {
        let prev = SceneSnapshot {
            tick: 0,
            player: PlayerSnap {
                x: 0.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let next = SceneSnapshot {
            tick: 1,
            player: PlayerSnap {
                x: 10.0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!((interpolate(&prev, &next, -1.0).player.x - 0.0).abs() < 1e-5);
        assert!((interpolate(&prev, &next, 2.0).player.x - 10.0).abs() < 1e-5);
    }

    #[test]
    fn sector_count_mismatch_falls_back_to_next() {
        // Si entre ticks la cantidad de sectores cambia (no debería
        // pasar en Doom — los sectores son del mapa cargado y son
        // estables — pero defendemos el invariante).
        let prev = SceneSnapshot {
            tick: 0,
            sectors: Arc::from(vec![SectorSnap {
                floor_height: 0.0,
                ceiling_height: 128.0,
                light_level: 100,
                floor_pic: 0,
                ceiling_pic: 0,
            }]),
            ..Default::default()
        };
        let next = SceneSnapshot {
            tick: 1,
            sectors: Arc::from(vec![
                SectorSnap {
                    floor_height: 64.0,
                    ceiling_height: 192.0,
                    light_level: 200,
                    floor_pic: 1,
                    ceiling_pic: 2,
                },
                SectorSnap {
                    floor_height: 0.0,
                    ceiling_height: 64.0,
                    light_level: 50,
                    floor_pic: 0,
                    ceiling_pic: 0,
                },
            ]),
            ..Default::default()
        };
        let mid = interpolate(&prev, &next, 0.5);
        // Cae a `next` puro.
        assert_eq!(mid.sectors.len(), 2);
        assert_eq!(mid.sectors[0].light_level, 200);
    }
}
