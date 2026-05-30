//! Bridge real: `cosmos_model::Chart` → cosmos_astrology → [`RenderModel`].
//!
//! La sesión de efemérides VSOP2013 es **compartida globalmente** vía
//! `OnceLock` — abrirla cuesta unos cuantos ms (carga de las series en
//! memoria), y como es read-only se puede leer en paralelo desde varios
//! cómputos.
//!
//! Partido del monolito `bridge.rs` (regla dura #1): `maps` (traducciones a
//! tipos eternales + símbolos), `compute` (sesión global + cómputo de cartas)
//! y `overlays` (capas del RenderModel + resúmenes de aspectos). Las
//! importaciones comunes viven aquí; cada submódulo abre con `use super::*`
//! (scope único original preservado).

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use cosmos_astrology::{
    all_lots, composite, directed_longitude, find_aspects, find_synastry_aspects, next_return,
    primary_direction::PrimaryDirection, secondary_progression, solar_arc_true, topocentric_ecliptic,
    Aspect, AspectKind as EAspectKind, BirthData, BodySet, ChartConfig,
    DirectionKey as EDirectionKey, HouseSystem as EHouseSystem, Houses as EHouses, NatalChart,
    OrbTable, Zodiac as EZodiac,
};
use cosmos_sky::{Ayanamsha, Body, EphemerisSession, Instant as ESInstant, Observer, SessionConfig};

use cosmos_model::{Chart, HouseSystem, StoredChartConfig, Zodiac};
use crate::dignity::essential_dignity;
use crate::{
    compute_gr_triggers, AspectSummary, EngineError, Geometry, Glyph, GrDirection, Layer,
    LayerKind, LineSeg, OverlayMeta, RenderModel, UranianGroup,
};

mod compute;
mod maps;
mod overlays;

pub(crate) use compute::*;
pub(crate) use maps::*;
pub(crate) use overlays::*;
