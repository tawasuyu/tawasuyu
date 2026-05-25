//! Solar Arc directions.
//!
//! Solar Arc adds a single angular increment — the arc the secondary-
//! progressed Sun has covered between birth and the target age — to
//! every planet *and* every house cusp uniformly. Because the same arc
//! is applied everywhere, the relative house position of each body is
//! preserved by construction; what changes are the absolute zodiac
//! positions and the angles.
//!
//! Two solar-arc conventions exist:
//!
//! * **Naibod**: the arc is the *mean* Sun's motion ≈ 0°59'08"/day.
//!   Always the same per year regardless of natal Sun's actual progress.
//! * **True solar arc** (a.k.a. "Sun's secondary progression"):
//!   the arc is the actual secondary-progressed Sun's longitude minus
//!   the natal Sun's longitude. Varies year-to-year.
//!
//! This module implements both; the helper `solar_arc` chooses
//! [`SolarArcMethod::TrueProgressedSun`] by default — that is what
//! Swiss Ephemeris reports.

use cosmos_sky::{Body, EphemerisSession};

use crate::angles::signed_delta_rad;
use crate::chart::{Angle, NatalChart};
use crate::error::{AstrologyError, AstrologyResult};
use crate::progression::{progress, ProgressedHouses, ProgressionMethod};
use crate::zodiac::SignedLongitude;

/// Which arc convention to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SolarArcMethod {
    /// Arc = secondary-progressed Sun's longitude − natal Sun's longitude.
    /// The arc varies between roughly 0°57' and 1°01' per year of life
    /// depending on the natal Sun's actual motion.
    #[default]
    TrueProgressedSun,
    /// Naibod: arc = 0°59'08.33"/year × age. Constant per year.
    Naibod,
}

/// Naibod constant: mean Sun motion in radians per year of life
/// (0°59'08.33" per day × 1 day/year of life via secondary mapping).
const NAIBOD_RAD_PER_YEAR: f64 = 0.017_202_376; // ≈ 0°59'08.33" in radians

/// A solar-arc-directed chart bundled with the natal it derives from.
#[derive(Debug, Clone)]
pub struct SolarArcChart {
    pub natal: NatalChart,
    /// All natal positions and cusps shifted forward by `arc_rad`.
    pub directed: NatalChart,
    pub arc_rad: f64,
    pub method: SolarArcMethod,
    pub target_age_years: f64,
}

impl SolarArcChart {
    pub fn arc_deg(&self) -> f64 {
        self.arc_rad.to_degrees()
    }
}

/// Compute a solar-arc directed chart at the requested age, using the
/// chosen arc convention.
pub fn solar_arc(
    natal: &NatalChart,
    session: &EphemerisSession,
    target_age_years: f64,
    method: SolarArcMethod,
) -> AstrologyResult<SolarArcChart> {
    let arc_rad = match method {
        SolarArcMethod::TrueProgressedSun => {
            // Run a secondary progression and read the Sun's longitude
            // delta. We need the Sun in the natal *and* progressed
            // charts, both in the same tropical/sidereal zodiac.
            let prog = progress(
                natal,
                session,
                target_age_years,
                ProgressionMethod::Secondary,
                ProgressedHouses::Progressed,
            )?;
            let natal_sun = natal
                .placement(Body::Sun)
                .ok_or_else(|| AstrologyError::BodyUnavailable(
                    "natal chart missing Sun".into(),
                ))?
                .longitude
                .longitude_rad();
            let prog_sun = prog
                .progressed()
                .placement(Body::Sun)
                .ok_or_else(|| AstrologyError::BodyUnavailable(
                    "progressed chart missing Sun".into(),
                ))?
                .longitude
                .longitude_rad();
            signed_delta_rad(prog_sun, natal_sun)
        }
        SolarArcMethod::Naibod => NAIBOD_RAD_PER_YEAR * target_age_years,
    };

    let directed = direct(natal, arc_rad);

    Ok(SolarArcChart {
        natal: natal.clone(),
        directed,
        arc_rad,
        method,
        target_age_years,
    })
}

/// Convenience: solar arc with the default (true progressed Sun) method.
pub fn solar_arc_true(
    natal: &NatalChart,
    session: &EphemerisSession,
    target_age_years: f64,
) -> AstrologyResult<SolarArcChart> {
    solar_arc(natal, session, target_age_years, SolarArcMethod::TrueProgressedSun)
}

/// Convenience: solar arc with the Naibod (mean-Sun) method.
pub fn solar_arc_naibod(
    natal: &NatalChart,
    target_age_years: f64,
) -> SolarArcChart {
    let arc_rad = NAIBOD_RAD_PER_YEAR * target_age_years;
    let directed = direct(natal, arc_rad);
    SolarArcChart {
        natal: natal.clone(),
        directed,
        arc_rad,
        method: SolarArcMethod::Naibod,
        target_age_years,
    }
}

/// Apply a uniform `arc_rad` shift to every angle, cusp, and body of
/// `natal`. The result inherits all kinematics (rates, retrograde) from
/// the natal chart — solar arc is a *symbolic* shift, not a real
/// physical motion.
fn direct(natal: &NatalChart, arc_rad: f64) -> NatalChart {
    use std::f64::consts::TAU;
    let wrap = |x: f64| {
        let v = x.rem_euclid(TAU);
        if v < 0.0 {
            v + TAU
        } else {
            v
        }
    };

    let mut directed = natal.clone();
    // Shift all four angles.
    directed.replace_angles_with(natal);
    let asc_new = wrap(natal.ascendant().longitude_rad() + arc_rad);
    let mc_new = wrap(natal.midheaven().longitude_rad() + arc_rad);
    directed.set_directed_angles(
        Angle::from_radians(asc_new),
        Angle::from_radians(mc_new),
        Angle::from_radians(wrap(asc_new + std::f64::consts::PI)),
        Angle::from_radians(wrap(mc_new + std::f64::consts::PI)),
    );
    // Shift every cusp.
    for c in directed.houses.cusps.iter_mut() {
        *c = wrap(*c + arc_rad);
    }
    // Shift Ascendant and Midheaven in the raw `Houses` view too.
    directed.houses.ascendant_rad = wrap(directed.houses.ascendant_rad + arc_rad);
    directed.houses.midheaven_rad = wrap(directed.houses.midheaven_rad + arc_rad);
    // Shift every placement's longitude. House numbers are invariant
    // under uniform rotation, so they don't need re-assignment.
    for p in directed.placements.iter_mut() {
        let new_lon_rad = wrap(p.longitude.longitude_rad() + arc_rad);
        p.longitude = SignedLongitude::from_radians(new_lon_rad);
    }
    directed
}

