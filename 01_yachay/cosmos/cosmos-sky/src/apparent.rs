//! Apparent positions: ecliptic-of-date, equatorial-of-date, and (when an
//! observer is supplied) topocentric horizon coordinates.
//!
//! All angles are stored in radians and longitude is wrapped to `[0, 2π)`.
//! Helper accessors return the same values in degrees.

const TAU: f64 = std::f64::consts::TAU;

/// Apparent ecliptic-of-date longitude / latitude with geometric distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EclipticCoord {
    /// Ecliptic longitude λ, radians, `[0, 2π)`. This is the **tropical**
    /// longitude — to obtain the sidereal value subtract the ayanamsha
    /// (see [`crate::Ayanamsha`]).
    pub longitude_rad: f64,
    /// Ecliptic latitude β, radians, `[-π/2, π/2]`.
    pub latitude_rad: f64,
    /// Geometric distance from the observer (geocenter or topocentric
    /// origin) to the body, in kilometres. `0.0` for purely conceptual
    /// points (e.g. lunar nodes).
    pub distance_km: f64,
}

impl EclipticCoord {
    pub fn longitude_deg(&self) -> f64 {
        self.longitude_rad.to_degrees()
    }
    pub fn latitude_deg(&self) -> f64 {
        self.latitude_rad.to_degrees()
    }
    pub fn distance_au(&self) -> f64 {
        self.distance_km / cosmos_core::constants::AU_KM
    }
}

/// Apparent equatorial-of-date coordinates (TET frame).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquatorialCoord {
    /// Right ascension α, radians, `[0, 2π)`.
    pub right_ascension_rad: f64,
    /// Declination δ, radians, `[-π/2, π/2]`.
    pub declination_rad: f64,
    /// Geometric distance, km.
    pub distance_km: f64,
}

impl EquatorialCoord {
    pub fn right_ascension_deg(&self) -> f64 {
        self.right_ascension_rad.to_degrees()
    }
    pub fn declination_deg(&self) -> f64 {
        self.declination_rad.to_degrees()
    }
    /// Right ascension expressed in hours (`0..24`).
    pub fn right_ascension_hours(&self) -> f64 {
        self.right_ascension_rad.to_degrees() / 15.0
    }
}

/// Topocentric horizon coordinates (altitude / azimuth) at the supplied
/// observer's local clock. Geometric — atmospheric refraction is *not*
/// applied; callers who want refracted altitude can use
/// `cosmos_coords::refraction`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizonCoord {
    /// Altitude (elevation above horizon), radians.
    pub altitude_rad: f64,
    /// Azimuth, radians, N = 0, E = π/2 (modern astronomical convention).
    pub azimuth_rad: f64,
}

impl HorizonCoord {
    pub fn altitude_deg(&self) -> f64 {
        self.altitude_rad.to_degrees()
    }
    pub fn azimuth_deg(&self) -> f64 {
        self.azimuth_rad.to_degrees()
    }
    /// Same azimuth re-expressed in the Swiss Ephemeris convention
    /// (S = 0, W = π/2). Astrologers using Swiss-derived chart software
    /// often expect this form.
    pub fn azimuth_swiss_deg(&self) -> f64 {
        let v = self.azimuth_deg() - 180.0;
        if v < 0.0 {
            v + 360.0
        } else {
            v
        }
    }
}

/// Apparent rate of motion along the ecliptic of date.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EclipticVelocity {
    /// dλ/dt, radians per day. Negative when the body is retrograde.
    pub longitude_rate_rad_per_day: f64,
    /// dβ/dt, radians per day.
    pub latitude_rate_rad_per_day: f64,
    /// dr/dt, km per day.
    pub radial_rate_km_per_day: f64,
}

impl EclipticVelocity {
    pub fn longitude_rate_deg_per_day(&self) -> f64 {
        self.longitude_rate_rad_per_day.to_degrees()
    }
    pub fn is_retrograde(&self) -> bool {
        self.longitude_rate_rad_per_day < 0.0
    }
}

/// Complete apparent-position bundle returned by
/// [`crate::EphemerisSession::body_apparent`].
#[derive(Debug, Clone, Copy)]
pub struct ApparentPosition {
    /// Apparent ecliptic-of-date coordinates (tropical longitude).
    pub ecliptic_of_date: EclipticCoord,
    /// Apparent equatorial-of-date coordinates (TET frame).
    pub equatorial_of_date: EquatorialCoord,
    /// Topocentric horizon coordinates. `Some` only if an `Observer`
    /// was supplied to the call.
    pub topocentric_horizon: Option<HorizonCoord>,
    /// Apparent rate of motion along the ecliptic. Useful for retrograde
    /// detection and for sub-day interpolation in returns / progressions.
    pub ecliptic_velocity: EclipticVelocity,
}

impl ApparentPosition {
    /// Convenience: sidereal longitude under the supplied ayanamsha,
    /// wrapped to `[0, 2π)`. Pass `Some(Ayanamsha::Lahiri)` for Vedic
    /// charts.
    pub fn sidereal_longitude_rad(&self, ayanamsha_rad: f64) -> f64 {
        let v = self.ecliptic_of_date.longitude_rad - ayanamsha_rad;
        let v = v % TAU;
        if v < 0.0 {
            v + TAU
        } else {
            v
        }
    }

    pub fn sidereal_longitude_deg(&self, ayanamsha_rad: f64) -> f64 {
        self.sidereal_longitude_rad(ayanamsha_rad).to_degrees()
    }
}

/// Wrap a radian angle into `[0, 2π)`.
pub(crate) fn wrap_two_pi(x: f64) -> f64 {
    let v = x % TAU;
    if v < 0.0 {
        v + TAU
    } else {
        v
    }
}
