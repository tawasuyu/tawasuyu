//! Eclipses, surfaced as an astrology-layer concern: find the next
//! eclipse and report its ecliptic longitude so callers can ask
//! "is this eclipse on one of my natal points?".
//!
//! The geometric eclipse machinery lives in `eternal-validation::eclipses`
//! and requires an SPK planetary kernel. This module wraps those
//! routines, computes the eclipse longitude (Sun's longitude for a
//! solar eclipse — the Sun is what gets eclipsed; Moon's longitude for
//! a lunar eclipse), and exposes a helper that filters eclipses by
//! proximity to any natal significator.

use eternal_sky::{Body, EphemerisSession, Instant};
use eternal_validation::eclipses as ev_eclipses;

use crate::chart::NatalChart;
use crate::error::{AstrologyError, AstrologyResult};
use crate::primary_direction::Significator;

pub use ev_eclipses::{LunarEclipseKind, SolarEclipseKind};

/// Family identifier — whether the eclipse occurs at conjunction
/// (solar) or opposition (lunar) of Sun and Moon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EclipseFamily {
    Solar,
    Lunar,
}

/// One eclipse event, with its detailed sub-classification and the
/// ecliptic longitude (of date) at which it falls.
#[derive(Debug, Clone, Copy)]
pub struct Eclipse {
    pub family: EclipseFamily,
    /// Solar sub-kind (Total / Partial / Annular / Hybrid / None) if
    /// the event is solar; `None` otherwise.
    pub solar_kind: Option<SolarEclipseKind>,
    /// Lunar sub-kind (Total / Partial / Penumbral / None) if the
    /// event is lunar; `None` otherwise.
    pub lunar_kind: Option<LunarEclipseKind>,
    pub instant: Instant,
    /// Ecliptic-of-date longitude where the eclipse falls (radians).
    /// For a solar eclipse this is the Sun's apparent ecliptic
    /// longitude at maximum; for a lunar eclipse, the Moon's.
    pub eclipse_longitude_rad: f64,
}

/// Eclipse falling within orb of a natal significator.
#[derive(Debug, Clone, Copy)]
pub struct NatalEclipse {
    pub eclipse: Eclipse,
    pub natal_target: Significator,
    pub natal_longitude_rad: f64,
    /// Unsigned angular distance between eclipse longitude and natal
    /// target longitude (degrees).
    pub orb_deg: f64,
}

/// Find the next solar eclipse after `after` and within
/// `max_synodic_months` lunar cycles.
pub fn next_solar_eclipse(
    session: &EphemerisSession,
    after: Instant,
    max_synodic_months: usize,
) -> AstrologyResult<Option<Eclipse>> {
    let spk = require_spk(session)?;
    let jd_start = after.jd_tdb()?;
    let next = ev_eclipses::next_solar_eclipse(spk, jd_start, max_synodic_months)
        .map_err(|e| AstrologyError::Sky(eternal_sky::SkyError::Ephemeris(e)))?;
    match next {
        None => Ok(None),
        Some((jd_tdb, snap)) => {
            let instant = Instant::from_jd_tdb(jd_tdb)?;
            // Solar eclipse longitude = Sun's apparent ecliptic longitude.
            let sun = session
                .body_apparent(Body::Sun, instant, None)
                .map_err(AstrologyError::Sky)?;
            Ok(Some(Eclipse {
                family: EclipseFamily::Solar,
                solar_kind: Some(snap.kind),
                lunar_kind: None,
                instant,
                eclipse_longitude_rad: sun.ecliptic_of_date.longitude_rad,
            }))
        }
    }
}

/// Find the next lunar eclipse after `after` and within
/// `max_synodic_months` lunar cycles.
pub fn next_lunar_eclipse(
    session: &EphemerisSession,
    after: Instant,
    max_synodic_months: usize,
) -> AstrologyResult<Option<Eclipse>> {
    let spk = require_spk(session)?;
    let jd_start = after.jd_tdb()?;
    let next = ev_eclipses::next_lunar_eclipse(spk, jd_start, max_synodic_months)
        .map_err(|e| AstrologyError::Sky(eternal_sky::SkyError::Ephemeris(e)))?;
    match next {
        None => Ok(None),
        Some((jd_tdb, snap)) => {
            let instant = Instant::from_jd_tdb(jd_tdb)?;
            // Lunar eclipse longitude = Moon's apparent ecliptic longitude.
            let moon = session
                .body_apparent(Body::Moon, instant, None)
                .map_err(AstrologyError::Sky)?;
            Ok(Some(Eclipse {
                family: EclipseFamily::Lunar,
                solar_kind: None,
                lunar_kind: Some(snap.kind),
                instant,
                eclipse_longitude_rad: moon.ecliptic_of_date.longitude_rad,
            }))
        }
    }
}

/// Find every eclipse (solar + lunar interleaved) within the next
/// `max_synodic_months` synodic months that falls within `orb_deg` of
/// any natal target in `targets`. If `targets` is `None`, every natal
/// body plus the four angles is used.
///
/// SPK backend required (the underlying eclipse routines need a
/// planetary kernel for the Sun and Moon positions).
pub fn eclipses_on_natal(
    natal: &NatalChart,
    session: &EphemerisSession,
    after: Instant,
    max_synodic_months: usize,
    orb_deg: f64,
    targets: Option<&[Significator]>,
) -> AstrologyResult<Vec<NatalEclipse>> {
    let default_targets;
    let targets = match targets {
        Some(t) => t,
        None => {
            default_targets = default_natal_targets(natal);
            &default_targets
        }
    };

    // Build a list of every eclipse in the window. We scan solar and
    // lunar independently, then merge in chronological order.
    let mut all_eclipses = Vec::new();
    let mut cursor = after;
    for _ in 0..max_synodic_months {
        let solar = next_solar_eclipse(session, cursor, 1)?;
        if let Some(e) = solar {
            all_eclipses.push(e);
        }
        let lunar = next_lunar_eclipse(session, cursor, 1)?;
        if let Some(e) = lunar {
            all_eclipses.push(e);
        }
        // Advance the cursor by ~one synodic month to keep moving.
        cursor = Instant::from_utc(cursor.utc().add_days(29.530_588));
    }
    all_eclipses.sort_by(|a, b| {
        a.instant
            .jd_utc()
            .partial_cmp(&b.instant.jd_utc())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all_eclipses.dedup_by(|a, b| (a.instant.jd_utc() - b.instant.jd_utc()).abs() < 0.1);

    // Filter by natal proximity.
    let mut out = Vec::new();
    for ecl in all_eclipses {
        for &target in targets {
            let Some(target_lon_rad) = significator_longitude_rad(natal, target) else {
                continue;
            };
            let orb = unsigned_arc_deg(
                ecl.eclipse_longitude_rad.to_degrees(),
                target_lon_rad.to_degrees(),
            );
            if orb <= orb_deg {
                out.push(NatalEclipse {
                    eclipse: ecl,
                    natal_target: target,
                    natal_longitude_rad: target_lon_rad,
                    orb_deg: orb,
                });
            }
        }
    }

    out.sort_by(|a, b| {
        a.orb_deg
            .partial_cmp(&b.orb_deg)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

// ─── Helpers ──────────────────────────────────────────────────────────

fn require_spk(
    session: &EphemerisSession,
) -> AstrologyResult<&eternal_ephemeris::jpl::SpkFile> {
    session.oracle().spk().ok_or_else(|| {
        AstrologyError::Sky(eternal_sky::SkyError::Ephemeris(
            eternal_validation::oracle::OracleError::Inner(
                "eclipses require an SPK planetary kernel; the session was \
                 opened with the analytical VSOP2013 backend"
                    .into(),
            ),
        ))
    })
}

fn default_natal_targets(natal: &NatalChart) -> Vec<Significator> {
    crate::transits::default_natal_targets(natal)
}

fn significator_longitude_rad(natal: &NatalChart, sig: Significator) -> Option<f64> {
    match sig {
        Significator::Body(b) => Some(natal.placement(b)?.longitude.longitude_rad()),
        Significator::Ascendant => Some(natal.ascendant().longitude_rad()),
        Significator::Midheaven => Some(natal.midheaven().longitude_rad()),
        Significator::Descendant => Some(natal.descendant().longitude_rad()),
        Significator::ImumCoeli => Some(natal.imum_coeli().longitude_rad()),
    }
}

fn unsigned_arc_deg(a_deg: f64, b_deg: f64) -> f64 {
    let mut d = (a_deg - b_deg).rem_euclid(360.0);
    if d > 180.0 {
        d = 360.0 - d;
    }
    d
}
