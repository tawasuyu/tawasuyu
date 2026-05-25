//! Rise / set / transit times (Phase 3, step 8).
//!
//! For a given body, observer, start time and direction (rise / set /
//! transit), find the next time the body crosses the requested
//! altitude (or maximises it for transit). Uses a coarse scan plus
//! Brent-style bisection on `altitude(t) − target`. The geometric
//! horizon is altitude = 0; for the standard refracted horizon used by
//! Swiss `rise_trans` add `−34'` (atmospheric refraction at horizon).
//!
//! The body's altitude is computed via the full apparent topocentric
//! pipeline (LT + LD + S + NPB + parallax). For the Sun and Moon this
//! is the dominant cost — caching apparent positions across the bisect
//! steps would speed it up but isn't necessary at the times-per-day
//! scale most callers operate at.

use crate::oracle::{Oracle, OracleError};
use crate::topocentric::{apparent_alt_az, Observer};

/// Minutes per day used to convert iteration tolerances.
const SECONDS_PER_DAY: f64 = 86_400.0;

/// Target altitude convention. Pick the one that matches your use case.
#[derive(Debug, Clone, Copy)]
pub enum HorizonTarget {
    /// True geometric horizon (altitude = 0).
    Geometric,
    /// Standard refracted-horizon convention used by Swiss `rise_trans`:
    /// −34 arcminutes for stars/planets, −50 arcminutes for the Sun (to
    /// account for the apparent disc radius), −0.583° for the Moon
    /// (taking into account the typical lunar parallax + refraction).
    /// We model only the refraction term here at −34′ — callers that
    /// need exact Swiss matching for solar / lunar limb timing should
    /// override.
    Refracted,
    /// Custom altitude in radians.
    Custom(f64),
}

impl HorizonTarget {
    pub fn altitude_rad(self) -> f64 {
        match self {
            HorizonTarget::Geometric => 0.0,
            HorizonTarget::Refracted => -(34.0 / 60.0_f64).to_radians(),
            HorizonTarget::Custom(a) => a,
        }
    }
}

/// Direction of the desired event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Rise,
    Set,
    Transit,
}

/// Find the next `event` for `body` after `jd_start_tdb`, observed from
/// `observer`. Returns `Ok(jd_tdb)` of the event or `Err` if none was
/// found within `search_days` (default 1.0 day for rise/set, 1.5 for
/// transit so we always bracket at least one).
pub fn find_next_event(
    oracle: &Oracle,
    body: i32,
    observer: &Observer,
    jd_start_tdb: f64,
    delta_t_seconds: f64,
    event: Event,
    target: HorizonTarget,
    search_days: f64,
) -> Result<f64, OracleError> {
    let mut alt_at = |jd: f64| -> Result<f64, OracleError> {
        let (alt, _az) = apparent_alt_az(oracle, body, jd, observer, delta_t_seconds)?;
        Ok(alt)
    };

    // Coarse scan: 5-minute steps. Plenty fine for Moon / Sun (which
    // move at most ~0.5° in 5 minutes); slightly oversampled for stars.
    const STEP_MIN: f64 = 5.0;
    let step_days = STEP_MIN * 60.0 / SECONDS_PER_DAY;
    let n_steps = (search_days / step_days) as usize + 1;

    match event {
        Event::Rise | Event::Set => {
            let target_alt = target.altitude_rad();
            let mut prev_jd = jd_start_tdb;
            let mut prev_alt = alt_at(prev_jd)? - target_alt;
            for i in 1..=n_steps {
                let jd = jd_start_tdb + (i as f64) * step_days;
                let alt = alt_at(jd)? - target_alt;
                let crosses = match event {
                    Event::Rise => prev_alt < 0.0 && alt >= 0.0,
                    Event::Set => prev_alt > 0.0 && alt <= 0.0,
                    Event::Transit => unreachable!(),
                };
                if crosses {
                    return bisect(&mut alt_at, prev_jd, jd, target_alt, 32);
                }
                prev_jd = jd;
                prev_alt = alt;
            }
            Err(OracleError::Inner(format!(
                "no {:?} found within {} days from JD {}",
                event, search_days, jd_start_tdb
            )))
        }
        Event::Transit => {
            // Transit = altitude is locally maximum. Find a sign change
            // in the derivative via simple finite differences across the
            // coarse scan, then bisect the derivative.
            let mut samples = Vec::with_capacity(n_steps + 1);
            for i in 0..=n_steps {
                let jd = jd_start_tdb + (i as f64) * step_days;
                samples.push((jd, alt_at(jd)?));
            }
            // Find a triple (a, b, c) with alt(b) > alt(a) and alt(b) > alt(c).
            for w in samples.windows(3) {
                let (a, alt_a) = w[0];
                let (_b, alt_b) = w[1];
                let (c, alt_c) = w[2];
                if alt_b > alt_a && alt_b > alt_c {
                    // Bracket: derivative is positive at midpoint of (a,b)
                    // and negative at midpoint of (b,c). Bisect.
                    return bisect_derivative(&mut alt_at, a, c, 24);
                }
            }
            Err(OracleError::Inner(format!(
                "no transit (local maximum) found within {} days from JD {}",
                search_days, jd_start_tdb
            )))
        }
    }
}

/// Bisect `alt_at(jd) − target` between `lo` and `hi` until convergence
/// or `max_iter`. Tolerance is 0.1 second of time.
fn bisect<F>(alt_at: &mut F, mut lo: f64, mut hi: f64, target: f64, max_iter: usize) -> Result<f64, OracleError>
where
    F: FnMut(f64) -> Result<f64, OracleError>,
{
    let mut f_lo = alt_at(lo)? - target;
    let mut f_hi = alt_at(hi)? - target;
    if f_lo * f_hi > 0.0 {
        return Err(OracleError::Inner(format!(
            "bisect: no sign change between JD {} (alt={}) and JD {} (alt={})",
            lo, f_lo, hi, f_hi
        )));
    }
    let tol_days = 0.1 / SECONDS_PER_DAY;
    for _ in 0..max_iter {
        let mid = 0.5 * (lo + hi);
        let f_mid = alt_at(mid)? - target;
        if f_mid.abs() < 1.0e-10 || (hi - lo) < tol_days {
            return Ok(mid);
        }
        if f_lo * f_mid < 0.0 {
            hi = mid;
            f_hi = f_mid;
        } else {
            lo = mid;
            f_lo = f_mid;
        }
    }
    let _ = f_hi;
    Ok(0.5 * (lo + hi))
}

/// Bisect to find the local maximum of altitude in `[lo, hi]` by
/// looking for a sign change in the derivative `(alt(t+δ) − alt(t−δ))`.
fn bisect_derivative<F>(alt_at: &mut F, mut lo: f64, mut hi: f64, max_iter: usize) -> Result<f64, OracleError>
where
    F: FnMut(f64) -> Result<f64, OracleError>,
{
    let dt = 30.0 / SECONDS_PER_DAY; // 30 s

    let derivative = |alt_at: &mut F, jd: f64| -> Result<f64, OracleError> {
        let plus = alt_at(jd + dt)?;
        let minus = alt_at(jd - dt)?;
        Ok(plus - minus)
    };

    let mut d_lo = derivative(alt_at, lo)?;
    let mut d_hi = derivative(alt_at, hi)?;
    if d_lo * d_hi > 0.0 {
        return Err(OracleError::Inner(format!(
            "bisect_derivative: no sign change between JD {} (d={}) and JD {} (d={})",
            lo, d_lo, hi, d_hi
        )));
    }
    let tol_days = 1.0 / SECONDS_PER_DAY;
    for _ in 0..max_iter {
        let mid = 0.5 * (lo + hi);
        let d_mid = derivative(alt_at, mid)?;
        if d_mid.abs() < 1.0e-12 || (hi - lo) < tol_days {
            return Ok(mid);
        }
        if d_lo * d_mid < 0.0 {
            hi = mid;
            d_hi = d_mid;
        } else {
            lo = mid;
            d_lo = d_mid;
        }
    }
    let _ = d_hi;
    Ok(0.5 * (lo + hi))
}

/// Convenience: find next rise of body above the geometric horizon.
pub fn next_rise(
    oracle: &Oracle,
    body: i32,
    observer: &Observer,
    jd_start_tdb: f64,
    delta_t_seconds: f64,
) -> Result<f64, OracleError> {
    find_next_event(
        oracle,
        body,
        observer,
        jd_start_tdb,
        delta_t_seconds,
        Event::Rise,
        HorizonTarget::Refracted,
        1.5,
    )
}

/// Convenience: find next set of body below the geometric horizon.
pub fn next_set(
    oracle: &Oracle,
    body: i32,
    observer: &Observer,
    jd_start_tdb: f64,
    delta_t_seconds: f64,
) -> Result<f64, OracleError> {
    find_next_event(
        oracle,
        body,
        observer,
        jd_start_tdb,
        delta_t_seconds,
        Event::Set,
        HorizonTarget::Refracted,
        1.5,
    )
}

/// Convenience: find next upper transit of body.
pub fn next_transit(
    oracle: &Oracle,
    body: i32,
    observer: &Observer,
    jd_start_tdb: f64,
    delta_t_seconds: f64,
) -> Result<f64, OracleError> {
    find_next_event(
        oracle,
        body,
        observer,
        jd_start_tdb,
        delta_t_seconds,
        Event::Transit,
        HorizonTarget::Geometric,
        1.5,
    )
}
