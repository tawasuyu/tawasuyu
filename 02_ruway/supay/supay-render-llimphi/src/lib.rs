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
//! - **Occlusion culling** (solidsegs clip-list estilo Doom para *no
//!   pintar* columnas tapadas). El ordering BSP **sí** está (fase 3.13b:
//!   `walk_bsp` back-to-front → `bsp_rank` como clave primaria del sort
//!   unificado en `frame.rs`, con tests `bsp_walk_*`/`bsp_ranks_*`), así
//!   que la imagen es correcta para geometría opaca convexa; lo que falta
//!   es saltarse el overdraw de lo oculto — es **perf**, no correctitud.
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
    interpolate, NodeSnap, PlayerOverlays, PlayerStats, SceneSnapshot, SectorSnap, SegSnap,
    SnapshotPair, SpriteSnap, SubsectorSnap, WallSeg, WeaponSpriteSnap, ML_DONTPEGBOTTOM,
    ML_DONTPEGTOP,
    NF_SUBSECTOR, NO_SECTOR, NO_SKY_PIC,
};

// =====================================================================
// Config
// =====================================================================

// =====================================================================
// Módulos (split Fase 3.53 — lib.rs era 8556 LOC, regla dura #1)
// =====================================================================
mod atlas;
mod config;
mod lighting;
mod frame;
mod camera;
mod walls;
mod planes;
mod sprites;
mod palette;
mod hud;
mod godrays;

pub use atlas::*;
pub use config::*;
pub(crate) use lighting::*;
pub use frame::*;
pub(crate) use camera::*;
pub(crate) use walls::*;
pub(crate) use planes::*;
pub(crate) use sprites::*;
pub(crate) use palette::*;
pub(crate) use hud::*;
pub(crate) use godrays::*;

#[cfg(test)]
mod tests;
