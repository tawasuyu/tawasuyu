//! # eternal-sky
//!
//! Ergonomic façade over the `eternal-*` astronomy crates. Hides the
//! orchestration of time scales, ephemeris kernels, IAU rotations, and
//! topocentric reductions behind three high-level types:
//!
//! * [`Instant`] — a civil (UTC) moment in time, with on-demand conversions
//!   to TT / TDB / UT1 / JD-TDB and a bundled ΔT lookup.
//! * [`Observer`] — a geodetic location on the WGS-84 ellipsoid.
//! * [`EphemerisSession`] — an open handle to a planetary backend (JPL
//!   SPK kernels or VSOP2013/ELP-MPP02 analytical theories) that produces
//!   [`ApparentPosition`]s for any supported [`Body`].
//!
//! ```no_run
//! use cosmos_sky::{Body, EphemerisSession, Instant, Observer, SessionConfig};
//!
//! let session = EphemerisSession::open(SessionConfig::vsop2013())?;
//! let observer = Observer::from_degrees(10.4806, -66.9036, 900.0);
//! let when = Instant::from_civil_utc(1987, 3, 14, 9, 22, 0.0)?;
//!
//! let mars = session.body_apparent(Body::Mars, when, Some(&observer))?;
//! println!("Mars λ = {:.4}°  β = {:.4}°  alt = {:.2}°",
//!     mars.ecliptic_of_date.longitude_deg(),
//!     mars.ecliptic_of_date.latitude_deg(),
//!     mars.topocentric_horizon.unwrap().altitude_deg(),
//! );
//! # Ok::<_, cosmos_sky::SkyError>(())
//! ```
//!
//! The crate is a *thin* layer: every computation forwards to the same
//! validated routines used by `eternal-validation`'s regression harness.
//! Precision is identical; the only thing added is API ergonomics.

pub mod apparent;
pub mod body;
pub mod delta_t;
pub mod error;
pub mod event_search;
pub mod instant;
pub mod observer;
pub mod session;

pub use apparent::{
    ApparentPosition, EclipticCoord, EclipticVelocity, EquatorialCoord, HorizonCoord,
};
pub use body::Body;
pub use delta_t::delta_t_seconds;
pub use error::{SkyError, SkyResult};
pub use event_search::{find_all_roots, find_root, SearchOptions};
pub use instant::Instant;
pub use observer::Observer;
pub use session::{EphemerisSession, GeometricState, SessionBackend, SessionConfig};

// Direct re-exports of useful underlying types so callers don't need to
// import the lower-level crates for routine use.
pub use cosmos_time::{TDB, TT, UT1, UTC};
pub use cosmos_validation::sidereal::Ayanamsha;
