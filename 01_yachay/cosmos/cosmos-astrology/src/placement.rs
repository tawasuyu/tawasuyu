//! A single body's placement in a chart: its zodiac position, house,
//! retrograde flag, and (optionally) topocentric horizon coordinates.

use cosmos_sky::{ApparentPosition, Body, HorizonCoord};

use crate::zodiac::SignedLongitude;

#[derive(Debug, Clone, Copy)]
pub struct BodyPlacement {
    pub body: Body,
    /// Tropical or sidereal longitude depending on the chart's [`crate::Zodiac`].
    pub longitude: SignedLongitude,
    /// Ecliptic latitude, radians (`[-π/2, π/2]`).
    pub latitude_rad: f64,
    /// Geometric distance from the observer (geocenter or topocentric
    /// origin) to the body, in km. `0.0` for purely conceptual points
    /// (lunar nodes, Lilith).
    pub distance_km: f64,
    /// Apparent ecliptic longitude rate, radians per day. Signed:
    /// negative means retrograde. Carried forward so the aspect engine
    /// can decide applying vs separating without re-querying the
    /// ephemeris.
    pub longitude_rate_rad_per_day: f64,
    /// Apparent right ascension of date, radians, `[0, 2π)`. Required
    /// for mundane and primary-direction work.
    pub right_ascension_rad: f64,
    /// Apparent declination of date, radians, `[-π/2, π/2]`.
    pub declination_rad: f64,
    /// 1..=12. Computed against the chart's chosen house system.
    pub house_number: u8,
    /// Topocentric horizon coordinates if an Observer was supplied to
    /// the apparent computation.
    pub horizon: Option<HorizonCoord>,
}

impl BodyPlacement {
    /// `true` if `dλ/dt < 0` at the chart's epoch — i.e. the body is
    /// moving retrograde relative to its mean direction.
    #[inline]
    pub fn is_retrograde(&self) -> bool {
        self.longitude_rate_rad_per_day < 0.0
    }

    pub(crate) fn from_apparent(
        body: Body,
        apparent: &ApparentPosition,
        longitude_for_zodiac_rad: f64,
        house_number: u8,
    ) -> Self {
        Self {
            body,
            longitude: SignedLongitude::from_radians(longitude_for_zodiac_rad),
            latitude_rad: apparent.ecliptic_of_date.latitude_rad,
            distance_km: apparent.ecliptic_of_date.distance_km,
            longitude_rate_rad_per_day: apparent.ecliptic_velocity.longitude_rate_rad_per_day,
            right_ascension_rad: apparent.equatorial_of_date.right_ascension_rad,
            declination_rad: apparent.equatorial_of_date.declination_rad,
            house_number,
            horizon: apparent.topocentric_horizon,
        }
    }
}
