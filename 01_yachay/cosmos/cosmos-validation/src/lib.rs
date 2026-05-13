//! Validation harness for eternal-ephemeris.
//!
//! Loads ground-truth state vectors (typically fetched from JPL Horizons or
//! computed by Swiss Ephemeris), runs the equivalent query through the local
//! backends, and reports per-fixture errors in physical units (km, km/s) plus
//! angular separation in milli-arcseconds.
//!
//! The crate is intentionally lightweight and dev-only (`publish = false`).
//! Its job is to be the thermometer that gates every change in
//! eternal-ephemeris during the road to v1.0.

pub mod asteroids;
pub mod delta_t;
pub mod eclipses;
pub mod fixed_stars;
pub mod fixture;
pub mod houses;
pub mod lunar;
pub mod oracle;
pub mod report;
pub mod rise_set;
pub mod sidereal;
pub mod topocentric;

#[cfg(feature = "fetch")]
pub mod horizons;

pub use fixture::{BackendKind, Corrections, Fixture, FixtureSet, Frame, Source, Tolerance};
pub use sidereal::{ayanamsha, lahiri_ayanamsha, lahiri_sidereal_longitude, Ayanamsha};
pub use oracle::{Backend, Oracle, OracleError, StateKmS};
pub use report::{ErrorReport, ReportTable};
