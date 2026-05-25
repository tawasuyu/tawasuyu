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

use cosmos_sky::{Body, EphemerisSession, Instant};
use cosmos_validation::eclipses as ev_eclipses;

use crate::angles::unsigned_arc_deg;
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
    next_eclipse(session, after, max_synodic_months, EclipseFamily::Solar)
}

/// Find the next lunar eclipse after `after` and within
/// `max_synodic_months` lunar cycles.
pub fn next_lunar_eclipse(
    session: &EphemerisSession,
    after: Instant,
    max_synodic_months: usize,
) -> AstrologyResult<Option<Eclipse>> {
    next_eclipse(session, after, max_synodic_months, EclipseFamily::Lunar)
}

/// Shared scan path for both solar and lunar eclipses. The two
/// families only differ in (a) which validation routine drives the
/// shadow-geometry check and (b) which body (Sun for solar, Moon for
/// lunar) carries the ecliptic longitude reported as the eclipse
/// point.
fn next_eclipse(
    session: &EphemerisSession,
    after: Instant,
    max_synodic_months: usize,
    family: EclipseFamily,
) -> AstrologyResult<Option<Eclipse>> {
    let spk = require_spk(session)?;
    let jd_start = after.jd_tdb()?;
    let found = match family {
        EclipseFamily::Solar => ev_eclipses::next_solar_eclipse(
            spk,
            jd_start,
            max_synodic_months,
        )
        .map(|opt| opt.map(EclipseHit::Solar)),
        EclipseFamily::Lunar => ev_eclipses::next_lunar_eclipse(
            spk,
            jd_start,
            max_synodic_months,
        )
        .map(|opt| opt.map(EclipseHit::Lunar)),
    }
    .map_err(|e| AstrologyError::Sky(cosmos_sky::SkyError::Ephemeris(e)))?;

    let Some(hit) = found else {
        return Ok(None);
    };
    let jd_tdb = hit.jd_tdb();
    let instant = Instant::from_jd_tdb(jd_tdb)?;
    let longitude_body = match family {
        EclipseFamily::Solar => Body::Sun,
        EclipseFamily::Lunar => Body::Moon,
    };
    let snap = session
        .body_apparent(longitude_body, instant, None)
        .map_err(AstrologyError::Sky)?;
    let (solar_kind, lunar_kind) = match hit {
        EclipseHit::Solar((_, s)) => (Some(s.kind), None),
        EclipseHit::Lunar((_, s)) => (None, Some(s.kind)),
    };
    Ok(Some(Eclipse {
        family,
        solar_kind,
        lunar_kind,
        instant,
        eclipse_longitude_rad: snap.ecliptic_of_date.longitude_rad,
    }))
}

/// Internal tagged union over the two underlying eclipse snapshot types.
enum EclipseHit {
    Solar((f64, ev_eclipses::SolarEclipseSnapshot)),
    Lunar((f64, ev_eclipses::LunarEclipseSnapshot)),
}

impl EclipseHit {
    fn jd_tdb(&self) -> f64 {
        match self {
            EclipseHit::Solar((jd, _)) => *jd,
            EclipseHit::Lunar((jd, _)) => *jd,
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

    // Sweep solar and lunar independently as monotonic cursor walks:
    // each `next_*_eclipse` call advances past the prior find rather
    // than restarting from `after + N·month`. Two sweeps, never more
    // than (NUMEC_solar + NUMEC_lunar) underlying calls, no dedup.
    let mut all_eclipses = Vec::new();
    sweep_eclipses(session, after, max_synodic_months, EclipseFamily::Solar, &mut all_eclipses)?;
    sweep_eclipses(session, after, max_synodic_months, EclipseFamily::Lunar, &mut all_eclipses)?;
    all_eclipses.sort_by(|a, b| {
        a.instant
            .jd_utc()
            .partial_cmp(&b.instant.jd_utc())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Filter by natal proximity.
    let mut out = Vec::new();
    for ecl in all_eclipses {
        for &target in targets {
            let Some(target_lon_rad) = target.longitude_rad(natal) else {
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
) -> AstrologyResult<&cosmos_ephemeris::jpl::SpkFile> {
    session.require_spk().map_err(AstrologyError::Sky)
}

/// Walk a monotonic cursor through `max_synodic_months`-worth of
/// eclipses of the requested family, accumulating into `out`.
///
/// The previous implementation called `next_X_eclipse(session, cursor, 1)`
/// in a loop and advanced the cursor by ~29.53 days, which forced
/// `next_X_eclipse` to redo most of its internal sweep on every
/// iteration. By advancing the cursor past each *found* eclipse instead,
/// the total scan now does ~N underlying calls for N synodic months
/// instead of ~2N redundant ones.
fn sweep_eclipses(
    session: &EphemerisSession,
    after: Instant,
    max_synodic_months: usize,
    family: EclipseFamily,
    out: &mut Vec<Eclipse>,
) -> AstrologyResult<()> {
    let mut cursor = after;
    let mut budget = max_synodic_months;
    while budget > 0 {
        let Some(ecl) = next_eclipse(session, cursor, budget, family)? else {
            return Ok(());
        };
        let jd_after = ecl.instant.jd_utc() + 1.0;
        out.push(ecl);
        cursor = Instant::from_utc(after.utc().add_days(jd_after - after.jd_utc()));
        // Each found eclipse "consumes" one synodic month of budget.
        budget -= 1;
    }
    Ok(())
}

fn default_natal_targets(natal: &NatalChart) -> Vec<Significator> {
    crate::transits::default_natal_targets(natal)
}

