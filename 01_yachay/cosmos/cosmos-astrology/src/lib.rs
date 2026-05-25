//! # eternal-astrology
//!
//! The astrology-specific layer built on top of [`eternal-sky`](`cosmos_sky`).
//!
//! ## What this crate is
//!
//! A typed pipeline that turns a moment of birth and a place into a
//! `NatalChart`: the four angles, twelve house cusps in the user's
//! chosen system, and every requested body placed in its sign and house
//! with retrograde flag.
//!
//! Every number this crate emits is traceable, by construction, to the
//! same validated routines that gate the regression harness of the
//! underlying astronomy crates — there is no parallel implementation of
//! ephemerides, time scales, or rotation matrices here. The astrology
//! layer is *interpretation-free*: it computes the traditional
//! astrological constructs (signs, houses, lots, retrogradation,
//! sidereal modes) with astronomical precision and does **not** make
//! claims about what those constructs mean for the person concerned.
//!
//! ## Disclaimer
//!
//! Astrology is a symbolic system with deep cultural and personal
//! significance for many people. This crate computes its traditional
//! constructs faithfully but takes no position on whether those
//! constructs describe, predict, or explain anything about an
//! individual's life. Treat the output as a *language*, not as data.
//!
//! ## Quick start
//!
//! ```no_run
//! use cosmos_astrology::{BirthData, ChartConfig, HouseSystem, NatalChart, Zodiac};
//! use cosmos_sky::{EphemerisSession, Instant, Observer, SessionConfig};
//!
//! let session = EphemerisSession::open(SessionConfig::vsop2013())?;
//! let birth = BirthData::new(
//!     Instant::from_civil_local(1987, 3, 14, 5, 22, 0.0, -240)?,
//!     Observer::from_degrees(10.4806, -66.9036, 900.0),
//! ).with_name("Subject A");
//!
//! let config = ChartConfig {
//!     house_system: HouseSystem::Placidus,
//!     zodiac: Zodiac::Tropical,
//!     ..ChartConfig::default()
//! };
//!
//! let chart = NatalChart::compute(&birth, &config, &session)?;
//! println!("Ascendant in {:?} {:.2}°",
//!     chart.ascendant().sign(),
//!     chart.ascendant().degree_in_sign(),
//! );
//! # Ok::<_, cosmos_astrology::AstrologyError>(())
//! ```

pub mod angles;
pub mod aspect;
pub mod birth_data;
pub mod chart;
pub mod chart_config;
pub mod composite;
pub mod eclipses;
pub mod error;
pub mod house_system;
pub mod lots;
pub mod lunar_phase;
pub mod mundane;
pub mod placement;
pub mod primary_direction;
pub mod profections;
pub mod progression;
pub mod returns;
pub mod solar_arc;
pub mod stations;
pub mod synastry;
pub mod topocentric;
pub mod transits;
pub mod zodiac;

pub use aspect::{find_aspects, find_aspects_filtered, Aspect, AspectKind, OrbTable};
pub use birth_data::{BirthData, TimeCertainty};
pub use chart::{Angle, NatalChart};
pub use chart_config::{BodySet, ChartConfig};
pub use composite::{angular_midpoint_rad, composite, CompositeChart, CompositePlacement};
pub use eclipses::{
    eclipses_on_natal, next_lunar_eclipse, next_solar_eclipse, Eclipse, EclipseFamily,
    LunarEclipseKind, NatalEclipse, SolarEclipseKind,
};
pub use error::{AstrologyError, AstrologyResult};
pub use house_system::{HouseSystem, Houses};
pub use placement::BodyPlacement;
pub use topocentric::topocentric_ecliptic;
pub use progression::{
    minor_progression, progress, progressed_instant, secondary_progression, tertiary_progression,
    ProgressedChart, ProgressedHouses, ProgressionMethod,
};
pub use primary_direction::{
    all_directions, all_directions_with_aspects, direct, direct_to_aspect, directed_longitude,
    directions_to_angles, Direction, DirectionKey, DirectionMethod, PrimaryDirection, Significator,
};
pub use lots::{all_lots, compute_lot, custom_lot, Lot, LotName, LotPoint, Sect};
pub use lunar_phase::{
    classify_lunation_phase, next_canonical_phase, next_lunar_phase, phase_angle_at,
    phase_angle_at_deg, LunarPhase, LunationPhase,
};
pub use profections::{
    annual_profection, modern_ruler, monthly_profection, profection_at, traditional_ruler,
    AnnualProfection, MonthlyProfection, ProfectionHouses,
};
pub use returns::next_return;
pub use solar_arc::{solar_arc, solar_arc_naibod, solar_arc_true, SolarArcChart, SolarArcMethod};
pub use stations::{all_stations, next_station, Station, StationKind};
pub use synastry::{find_synastry_aspects, SynastryAspect};
pub use transits::{
    default_natal_targets, find_current_transits, find_next_exact_transit, TransitAspect,
};
pub use zodiac::{Sign, SignedLongitude, Zodiac};

pub use cosmos_sky::Ayanamsha;
