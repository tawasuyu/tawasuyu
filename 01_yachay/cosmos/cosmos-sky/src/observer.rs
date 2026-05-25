//! Geodetic observer location on the WGS-84 ellipsoid.
//!
//! Thin re-export of `cosmos_validation::topocentric::Observer` with an
//! optional human-readable `name` attached. The underlying struct already
//! stores latitude/longitude in radians plus elevation in metres.
//!
//! In a future revision the time-invariant pre-computed quantities
//! (ITRS Cartesian, `sin/cos(lat)`, `sin/cos(lon)`) will be cached here
//! to make rectification loops cheaper. For now we forward to the
//! existing struct so behaviour stays bit-identical to the validation
//! pipeline.

pub use cosmos_validation::topocentric::Observer;

/// Named observer, useful for logs and chart metadata. Wraps an
/// `Observer` with a free-form label.
#[derive(Debug, Clone)]
pub struct NamedObserver {
    pub observer: Observer,
    pub name: String,
}

impl NamedObserver {
    pub fn new(name: impl Into<String>, observer: Observer) -> Self {
        Self {
            name: name.into(),
            observer,
        }
    }

    pub fn from_degrees(
        name: impl Into<String>,
        lat_deg: f64,
        lon_deg: f64,
        elev_m: f64,
    ) -> Self {
        Self {
            name: name.into(),
            observer: Observer::from_degrees(lat_deg, lon_deg, elev_m),
        }
    }
}
