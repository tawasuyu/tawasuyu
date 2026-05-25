//! House systems and their cusps.
//!
//! Each variant of [`HouseSystem`] forwards to a Swiss-faithful
//! implementation living in `eternal-validation::houses`. Cusp arrays
//! are always in radians, indexed `0..12` where index `i` is the start
//! of house `i+1` (house 1 = Ascendant by convention).

use cosmos_validation::houses as ev_houses;

use crate::error::{AstrologyError, AstrologyResult};

/// Selectable house system. Geometric (Whole-Sign, Equal, Porphyry) are
/// defined everywhere on Earth; quadrant systems (Placidus, Koch,
/// Campanus) diverge inside the polar circle and return an error there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HouseSystem {
    /// Houses are the 30°-wide zodiac signs counted from the
    /// Ascendant's sign. The oldest documented system.
    WholeSign,
    /// Ascendant + N×30°. House 1 begins exactly at the Ascendant.
    Equal,
    /// Trisection of the diurnal and nocturnal semi-arcs measured along
    /// the ecliptic between the Ascendant and the MC.
    Porphyry,
    /// Iterative trisection of the diurnal semi-arc measured along
    /// each planet's hour circle. The Swiss / Astrodienst implementation.
    Placidus,
    /// Iterative trisection of the diurnal arc as projected on the
    /// ecliptic. Like Placidus, undefined inside the polar circle.
    Koch,
    /// Trisection of the celestial equator → great-circle horizons.
    Regiomontanus,
    /// Trisection of the prime vertical → great-circle horizons.
    Campanus,
    /// **Polich–Page (Topocentric)** — closed-form quadrant system
    /// derived from a topocentric "pole height" `atan(tan φ · n/3)`
    /// per intermediate cusp. Faster than Placidus (no iteration),
    /// agrees closely in mid-latitudes, and is the canonical system
    /// for primary-direction work in the GR school. Undefined inside
    /// the polar circle.
    PolichPage,
}

impl Default for HouseSystem {
    fn default() -> Self {
        HouseSystem::Placidus
    }
}

/// The four angles + twelve cusps of a chart. All values are in
/// radians; helpers in [`crate::SignedLongitude`] convert to degrees /
/// sign-decimal form for presentation.
#[derive(Debug, Clone, Copy)]
pub struct Houses {
    pub system: HouseSystem,
    pub ascendant_rad: f64,
    pub midheaven_rad: f64,
    /// `cusps[i]` = ecliptic longitude (radians) of the start of house
    /// `i + 1`. House 1 starts at `cusps[0]` = Ascendant by definition.
    pub cusps: [f64; 12],
}

impl Houses {
    /// Compute Asc/MC/cusps for a moment + observer, given the
    /// already-derived Local Apparent Sidereal Time and the true
    /// obliquity of date.
    pub fn compute(
        system: HouseSystem,
        last_rad: f64,
        lat_rad: f64,
        obliquity_rad: f64,
    ) -> AstrologyResult<Self> {
        let ascendant_rad = ev_houses::ascendant(last_rad, lat_rad, obliquity_rad);
        let midheaven_rad = ev_houses::midheaven(last_rad, obliquity_rad);
        let cusps = match system {
            HouseSystem::WholeSign => ev_houses::whole_sign_houses(ascendant_rad),
            HouseSystem::Equal => ev_houses::equal_houses(ascendant_rad),
            HouseSystem::Porphyry => ev_houses::porphyry_houses(last_rad, lat_rad, obliquity_rad),
            HouseSystem::Placidus => ev_houses::placidus_houses(last_rad, lat_rad, obliquity_rad)
                .map_err(AstrologyError::HouseSystemUnavailable)?,
            HouseSystem::Koch => ev_houses::koch_houses(last_rad, lat_rad, obliquity_rad)
                .map_err(AstrologyError::HouseSystemUnavailable)?,
            HouseSystem::Regiomontanus => {
                ev_houses::regiomontanus_houses(last_rad, lat_rad, obliquity_rad)
            }
            HouseSystem::Campanus => ev_houses::campanus_houses(last_rad, lat_rad, obliquity_rad)
                .map_err(AstrologyError::HouseSystemUnavailable)?,
            HouseSystem::PolichPage => {
                ev_houses::polich_page_houses(last_rad, lat_rad, obliquity_rad)
                    .map_err(AstrologyError::HouseSystemUnavailable)?
            }
        };

        Ok(Self {
            system,
            ascendant_rad,
            midheaven_rad,
            cusps,
        })
    }

    /// Find which house (1..=12) contains `longitude_rad`. Membership is
    /// `cusps[i] ≤ λ < cusps[(i+1) % 12]` modulo 2π.
    pub fn house_containing(&self, longitude_rad: f64) -> u8 {
        const TAU: f64 = std::f64::consts::TAU;
        let lon = longitude_rad.rem_euclid(TAU);
        for i in 0..12 {
            let start = self.cusps[i].rem_euclid(TAU);
            let end = self.cusps[(i + 1) % 12].rem_euclid(TAU);
            if start <= end {
                if lon >= start && lon < end {
                    return (i + 1) as u8;
                }
            } else {
                // Wraps past 0° — body is in this house if it sits on
                // either side of the wrap.
                if lon >= start || lon < end {
                    return (i + 1) as u8;
                }
            }
        }
        // Floating-point edge case: body lands exactly on the last
        // cusp. Attribute it to house 12.
        12
    }
}
