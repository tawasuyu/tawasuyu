//! Lunar phases: the angular relationship between Moon and Sun
//! expressed in the eight classical phases.
//!
//! The **phase angle** `p` is `Moon_longitude − Sun_longitude` wrapped
//! to `[0, 2π)`. At `p = 0` the Moon and Sun are conjunct (new moon);
//! at `p = π/2` first quarter; at `p = π` opposition (full moon); at
//! `p = 3π/2` last quarter. The intermediate "crescent" and "gibbous"
//! phases occupy the eighths between the four canonical instants.
//!
//! All phase finding reduces to a root-find on
//! `signed_delta(p − target)` and reuses [`cosmos_sky::find_root`].

use cosmos_sky::{find_root, Body, EphemerisSession, Instant, SearchOptions, SkyResult};

use crate::angles::signed_delta_rad;
use crate::error::{AstrologyError, AstrologyResult};

const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

/// One of the four canonical lunar phases (the boundary instants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LunarPhase {
    NewMoon,
    FirstQuarter,
    FullMoon,
    LastQuarter,
}

impl LunarPhase {
    pub fn target_angle_rad(self) -> f64 {
        match self {
            LunarPhase::NewMoon => 0.0,
            LunarPhase::FirstQuarter => PI / 2.0,
            LunarPhase::FullMoon => PI,
            LunarPhase::LastQuarter => 3.0 * PI / 2.0,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            LunarPhase::NewMoon => "New Moon",
            LunarPhase::FirstQuarter => "First Quarter",
            LunarPhase::FullMoon => "Full Moon",
            LunarPhase::LastQuarter => "Last Quarter",
        }
    }
}

/// The full eight-phase classification used for "the Moon was waxing
/// gibbous when you were born" descriptions. Boundaries are at the
/// four canonical instants; the four "between" phases occupy the
/// 45°-wide bands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LunationPhase {
    NewMoon,
    WaxingCrescent,
    FirstQuarter,
    WaxingGibbous,
    FullMoon,
    WaningGibbous,
    LastQuarter,
    WaningCrescent,
}

impl LunationPhase {
    pub fn name(self) -> &'static str {
        match self {
            LunationPhase::NewMoon => "New Moon",
            LunationPhase::WaxingCrescent => "Waxing Crescent",
            LunationPhase::FirstQuarter => "First Quarter",
            LunationPhase::WaxingGibbous => "Waxing Gibbous",
            LunationPhase::FullMoon => "Full Moon",
            LunationPhase::WaningGibbous => "Waning Gibbous",
            LunationPhase::LastQuarter => "Last Quarter",
            LunationPhase::WaningCrescent => "Waning Crescent",
        }
    }
}

/// Compute the phase angle (Moon − Sun longitude, mod 2π) at `t`.
pub fn phase_angle_at(session: &EphemerisSession, t: Instant) -> SkyResult<f64> {
    let sun = session.body_apparent(Body::Sun, t, None)?;
    let moon = session.body_apparent(Body::Moon, t, None)?;
    let diff = moon.ecliptic_of_date.longitude_rad - sun.ecliptic_of_date.longitude_rad;
    Ok(diff.rem_euclid(TAU))
}

/// Phase angle at `t` in degrees.
pub fn phase_angle_at_deg(session: &EphemerisSession, t: Instant) -> SkyResult<f64> {
    Ok(phase_angle_at(session, t)?.to_degrees())
}

/// Classify the phase angle into one of eight lunation phases. Bands
/// are 45° wide; the canonical instants (0°, 90°, 180°, 270°) fall at
/// the band boundaries — they're classified into the *waxing* side by
/// convention (so an exact 90° is `FirstQuarter`, not `WaxingCrescent`).
pub fn classify_lunation_phase(phase_angle_rad: f64) -> LunationPhase {
    let p = phase_angle_rad.rem_euclid(TAU);
    let deg = p.to_degrees();
    // Band boundaries: 0, 45, 90, 135, 180, 225, 270, 315.
    if deg < 22.5 || deg >= 337.5 {
        LunationPhase::NewMoon
    } else if deg < 67.5 {
        LunationPhase::WaxingCrescent
    } else if deg < 112.5 {
        LunationPhase::FirstQuarter
    } else if deg < 157.5 {
        LunationPhase::WaxingGibbous
    } else if deg < 202.5 {
        LunationPhase::FullMoon
    } else if deg < 247.5 {
        LunationPhase::WaningGibbous
    } else if deg < 292.5 {
        LunationPhase::LastQuarter
    } else {
        LunationPhase::WaningCrescent
    }
}

/// Mean synodic month in days. Used to convert a phase delta into a
/// time estimate for the bisector.
const SYNODIC_MONTH_DAYS: f64 = 29.530_588_85;

/// Find the next instant after `after` at which the lunation reaches
/// the canonical `phase`. Returns `Ok(None)` if the estimated time
/// exceeds `max_window_days`.
///
/// Strategy: a single coarse bisection over the whole cycle would trip
/// over the `phase mod 2π` discontinuity (signed_delta jumps by 2π
/// when the phase angle wraps at 0/2π, which the bisector
/// misinterprets as a zero crossing). We dodge that by:
///
/// 1. Sampling the current phase angle at `after`.
/// 2. Computing the *forward* angular distance to the target
///    (`delta_phase = (target − current) mod 2π`).
/// 3. Estimating the time of perfection as
///    `Δt ≈ delta_phase × synodic_month / 2π`.
/// 4. Bisecting in a ±2-day window around that estimate — short
///    enough that `signed_delta` stays monotonic.
pub fn next_lunar_phase(
    session: &EphemerisSession,
    phase: LunarPhase,
    after: Instant,
    max_window_days: f64,
) -> AstrologyResult<Option<Instant>> {
    let target = phase.target_angle_rad();
    let current = phase_angle_at(session, after).map_err(AstrologyError::Sky)?;
    let delta_phase = (target - current).rem_euclid(TAU);
    let estimated_delta_days =
        delta_phase / TAU * SYNODIC_MONTH_DAYS;
    if estimated_delta_days > max_window_days {
        return Ok(None);
    }
    let center = Instant::from_utc(after.utc().add_days(estimated_delta_days));
    let lo = Instant::from_utc(center.utc().add_days(-2.0));
    let hi = Instant::from_utc(center.utc().add_days(2.0));

    let opts = SearchOptions {
        coarse_step_seconds: 3.0 * 3600.0, // 3 h
        tolerance_seconds: 30.0,
        max_iterations: 80,
    };

    find_root(
        lo,
        hi,
        |t: Instant| {
            let p = phase_angle_at(session, t)?;
            Ok(signed_delta_rad(p, target))
        },
        opts,
    )
    .map_err(AstrologyError::Sky)
}

/// Find the next of *any* canonical phase. Returns `(Instant, LunarPhase)`
/// with the phase identity for the event found.
///
/// The four phases are 90° apart on the phase angle, so we can pick
/// the next target in a single computation from the current phase
/// angle — no need to bisect for all four and discard three.
pub fn next_canonical_phase(
    session: &EphemerisSession,
    after: Instant,
    max_window_days: f64,
) -> AstrologyResult<Option<(Instant, LunarPhase)>> {
    let current = phase_angle_at(session, after).map_err(AstrologyError::Sky)?;
    // Phase quadrants on the unit cycle: New @ 0°, FQ @ 90°, Full @ 180°,
    // LQ @ 270°. The "next" target is the next 90°-multiple boundary
    // strictly *ahead* of `current`. floor(current/90°) + 1 gives the
    // index of that boundary mod 4.
    let quadrant_index = (current.to_degrees() / 90.0).floor() as i32;
    let next_index = ((quadrant_index + 1) % 4 + 4) % 4;
    let phase = match next_index {
        0 => LunarPhase::NewMoon,
        1 => LunarPhase::FirstQuarter,
        2 => LunarPhase::FullMoon,
        _ => LunarPhase::LastQuarter,
    };
    Ok(next_lunar_phase(session, phase, after, max_window_days)?.map(|t| (t, phase)))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_lunation_phase_bands() {
        assert_eq!(classify_lunation_phase(0.0), LunationPhase::NewMoon);
        assert_eq!(
            classify_lunation_phase((22.0_f64).to_radians()),
            LunationPhase::NewMoon
        );
        assert_eq!(
            classify_lunation_phase((23.0_f64).to_radians()),
            LunationPhase::WaxingCrescent
        );
        assert_eq!(
            classify_lunation_phase((90.0_f64).to_radians()),
            LunationPhase::FirstQuarter
        );
        assert_eq!(
            classify_lunation_phase((180.0_f64).to_radians()),
            LunationPhase::FullMoon
        );
        assert_eq!(
            classify_lunation_phase((270.0_f64).to_radians()),
            LunationPhase::LastQuarter
        );
        assert_eq!(
            classify_lunation_phase((340.0_f64).to_radians()),
            LunationPhase::NewMoon
        );
    }
}
