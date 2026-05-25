//! Transits: aspects between bodies in the *sky right now* (or at any
//! chosen instant) and points in a natal chart.
//!
//! A transit is the most common kind of forecasting an astrologer
//! consults. It asks: "of all the angular relationships that the
//! transiting planets currently form with my natal points, which ones
//! are within orb?" — and, by extension, "when will the next exact
//! contact happen?"
//!
//! Two modes are exposed:
//!
//! * [`find_current_transits`] — a snapshot of every aspect a list of
//!   transiting bodies makes with every body or angle in `natal`, at a
//!   single instant.
//! * [`find_next_exact_transit`] — root-finds the next time a specific
//!   transiting body's longitude is exactly N degrees from a specific
//!   natal longitude, where N is the [`AspectKind`]'s exact angle.

use cosmos_sky::{find_root, Body, EphemerisSession, Instant, SearchOptions};

use crate::angles::signed_delta_deg;
use crate::aspect::{AspectKind, OrbTable};
use crate::chart::NatalChart;
use crate::error::{AstrologyError, AstrologyResult};
use crate::primary_direction::Significator;

/// One aspect formed by a transiting body to a natal target.
#[derive(Debug, Clone, Copy)]
pub struct TransitAspect {
    /// The body currently moving in the sky.
    pub transiting: Body,
    /// The natal point being aspected.
    pub natal_target: Significator,
    pub kind: AspectKind,
    /// Signed delta from exact, degrees. Same convention as the aspect
    /// engine: positive = past exact, negative = short of exact.
    pub orb_signed_deg: f64,
    pub allowed_orb_deg: f64,
    /// `true` if the transiting body's longitude is closing toward the
    /// exact angle. Uses the transiting body's longitude rate; natal
    /// targets are treated as fixed (rate = 0).
    pub applying: bool,
    pub instant: Instant,
}

impl TransitAspect {
    pub fn orb_abs_deg(&self) -> f64 {
        self.orb_signed_deg.abs()
    }
}

/// Snapshot every transit aspect at `at`. Returns aspects sorted by
/// orb (tightest first).
///
/// `transiting_bodies` controls which sky positions to consider — pass
/// e.g. all major planets, or a subset for a quick scan. `targets`
/// controls which natal points are valid significators; pass
/// `natal_targets_default(&natal)` to use every body in the chart plus
/// the four angles.
pub fn find_current_transits(
    natal: &NatalChart,
    session: &EphemerisSession,
    at: Instant,
    transiting_bodies: &[Body],
    targets: &[Significator],
    orbs: &OrbTable,
    aspect_kinds: &[AspectKind],
) -> AstrologyResult<Vec<TransitAspect>> {
    let snapshot = orbs.snapshot();
    let mut out =
        Vec::with_capacity(transiting_bodies.len() * targets.len() * aspect_kinds.len());

    for &transiting in transiting_bodies {
        let pos = session
            .body_apparent(transiting, at, None)
            .map_err(AstrologyError::Sky)?;
        let t_lon_deg = pos.ecliptic_of_date.longitude_deg();
        let t_rate_deg_per_day =
            pos.ecliptic_velocity.longitude_rate_rad_per_day.to_degrees();

        for &target in targets {
            let target_lon_deg = target.longitude_deg(natal);
            let target_lon_deg = match target_lon_deg {
                Some(v) => v,
                None => continue,
            };
            for &kind in aspect_kinds {
                let allowed = snapshot.orb_for(
                    transiting,
                    body_for_significator(target, transiting),
                    kind,
                );
                if allowed <= 0.0 {
                    continue;
                }
                let raw = signed_delta_deg(t_lon_deg, target_lon_deg);
                let separation = raw.abs();
                let exact = kind.exact_angle_deg();
                let diff = separation - exact;
                if diff.abs() > allowed {
                    continue;
                }
                // Applying: target is fixed, so d(separation)/dt has
                // the sign of `raw × transiting_rate`. Closing means
                // (sep − exact) and dsep/dt have opposite signs.
                let dsep_dt = if raw >= 0.0 {
                    t_rate_deg_per_day
                } else {
                    -t_rate_deg_per_day
                };
                let applying = if diff > 0.0 {
                    dsep_dt < 0.0
                } else if diff < 0.0 {
                    dsep_dt > 0.0
                } else {
                    false
                };
                out.push(TransitAspect {
                    transiting,
                    natal_target: target,
                    kind,
                    orb_signed_deg: diff,
                    allowed_orb_deg: allowed,
                    applying,
                    instant: at,
                });
            }
        }
    }

    out.sort_by(|a, b| {
        a.orb_abs_deg()
            .partial_cmp(&b.orb_abs_deg())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

/// Default target set for transit queries: every body present in
/// `natal.placements` (deduplicated, including the four lunar nodes
/// only once) plus the four cardinal angles.
pub fn default_natal_targets(natal: &NatalChart) -> Vec<Significator> {
    let mut out: Vec<Significator> = Vec::new();
    let mut seen: Vec<Body> = Vec::new();
    for p in &natal.placements {
        if !seen.contains(&p.body) {
            out.push(Significator::Body(p.body));
            seen.push(p.body);
        }
    }
    out.push(Significator::Ascendant);
    out.push(Significator::Midheaven);
    out.push(Significator::Descendant);
    out.push(Significator::ImumCoeli);
    out
}

/// Find the next instant at or after `after` when `transiting`'s
/// ecliptic longitude is exactly `aspect_kind.exact_angle_deg()`
/// degrees from `natal_target_longitude_rad`. Returns `Ok(None)` if no
/// crossing occurs within `max_window_days`.
pub fn find_next_exact_transit(
    session: &EphemerisSession,
    transiting: Body,
    natal_target_longitude_rad: f64,
    aspect_kind: AspectKind,
    after: Instant,
    max_window_days: f64,
) -> AstrologyResult<Option<Instant>> {
    let target_rad = natal_target_longitude_rad;
    let exact_offset_rad = aspect_kind.exact_angle_deg().to_radians();

    let f = |t: Instant| -> cosmos_sky::SkyResult<f64> {
        let pos = session.body_apparent(transiting, t, None)?;
        let lon = pos.ecliptic_of_date.longitude_rad;
        // The aspect can perfect on either side of the target by the
        // exact offset. Return the signed "distance to nearest exact",
        // which crosses zero at perfection. We use the minimum of the
        // two possible crossings for monotonicity inside a single
        // aspect window — but the coarse scan handles either branch.
        Ok(signed_min_distance(lon - target_rad, exact_offset_rad))
    };

    let nominal_step_seconds = nominal_transit_step_seconds(transiting);
    let opts = SearchOptions {
        coarse_step_seconds: nominal_step_seconds,
        tolerance_seconds: 1.0,
        max_iterations: 80,
    };

    let t1 = Instant::from_utc(after.utc().add_days(max_window_days));
    find_root(after, t1, f, opts).map_err(AstrologyError::Sky)
}

/// Coarse-scan step for a transiting body — fast bodies need finer
/// sampling so the bisector brackets a single perfection per orbit.
fn nominal_transit_step_seconds(body: Body) -> f64 {
    use Body::*;
    match body {
        Moon => 3_600.0,                  // 1 h (Moon moves ~0.5°/h)
        Mercury | Venus | Sun => 21_600.0, // 6 h
        Mars => 43_200.0,                 // 12 h
        Jupiter | Saturn => 86_400.0,     // 1 d
        Uranus | Neptune | Pluto => 86_400.0 * 5.0, // 5 d
        _ => 86_400.0,
    }
}

/// Body to use as the "significator side" of the OrbTable lookup. For
/// `Significator::Body(b)` it's `b`; for angles we re-use the
/// transiting body's own multiplier so the result is symmetric.
fn body_for_significator(sig: Significator, fallback: Body) -> Body {
    match sig {
        Significator::Body(b) => b,
        _ => fallback,
    }
}

/// For an aspect that perfects when `(actual − target)` equals either
/// `+exact_offset` or `−exact_offset` (the two branches of the same
/// aspect family), return the smaller signed distance to perfection.
fn signed_min_distance(raw_diff_rad: f64, exact_offset_rad: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    let mut d = raw_diff_rad.rem_euclid(TAU);
    if d > PI {
        d -= TAU;
    }
    let plus = d - exact_offset_rad;
    let minus = d + exact_offset_rad;
    if plus.abs() <= minus.abs() {
        plus
    } else {
        minus
    }
}
