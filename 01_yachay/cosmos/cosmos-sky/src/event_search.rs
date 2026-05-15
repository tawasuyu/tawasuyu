//! Generic root-finder over time, used by every "find when does X
//! happen" query in the astrology layer.
//!
//! Given a continuous function `f(t)` defined on instants, locate the
//! first instant in `[t0, t1]` at which `f` crosses zero. The strategy
//! is a coarse scan (linear sampling at a configurable step) to bracket
//! the first sign change, followed by bisection to the requested
//! tolerance in seconds.
//!
//! This is intentionally low-level: callers wrap it for specific
//! semantics (planetary returns, exact aspects, retrograde stations).
//! Wrap-around quantities like ecliptic longitude differences need to
//! be passed already normalised to `[-π, π]` — the bisector treats
//! discontinuities as sign changes.

use crate::error::{SkyError, SkyResult};
use crate::instant::Instant;

const SECONDS_PER_DAY: f64 = 86_400.0;

/// Tuning knobs for [`find_root`].
#[derive(Debug, Clone, Copy)]
pub struct SearchOptions {
    /// Coarse-scan step in seconds. The scan walks the window in steps
    /// of this size, looking for the first sign change. Pick a value
    /// smaller than the *typical* half-period of the quantity you're
    /// chasing: 3600 s (1 h) suits the Moon and aspects to it, 86400 s
    /// (1 day) suits planetary returns / outer-body aspects, 60 s suits
    /// fine eclipse timing.
    pub coarse_step_seconds: f64,
    /// Bisection tolerance in seconds. The result is returned once the
    /// bracket shrinks below this width.
    pub tolerance_seconds: f64,
    /// Maximum bisection iterations. Bisection halves the bracket each
    /// step, so 64 iterations resolves `2^64`× → far more than any
    /// double-precision time arithmetic supports.
    pub max_iterations: usize,
}

impl SearchOptions {
    /// Defaults tuned for "events on the scale of an astrological day"
    /// (lunar transits, aspects involving the Moon): coarse 1 h,
    /// tolerance 1 s.
    pub const HOURLY: Self = Self {
        coarse_step_seconds: 3600.0,
        tolerance_seconds: 1.0,
        max_iterations: 64,
    };

    /// Defaults tuned for slow events (planetary returns, outer-body
    /// aspects): coarse 1 d, tolerance 1 s.
    pub const DAILY: Self = Self {
        coarse_step_seconds: SECONDS_PER_DAY,
        tolerance_seconds: 1.0,
        max_iterations: 64,
    };

    /// Defaults tuned for tight events (eclipses, exact ingresses):
    /// coarse 1 min, tolerance 0.1 s.
    pub const PRECISE: Self = Self {
        coarse_step_seconds: 60.0,
        tolerance_seconds: 0.1,
        max_iterations: 80,
    };
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self::HOURLY
    }
}

/// Find the first instant `t ∈ [t0, t1]` at which `f(t)` crosses zero.
/// Returns `Ok(None)` if no sign change is detected on the coarse grid;
/// otherwise returns the bisected zero with width `< tolerance_seconds`.
///
/// `f` is called many times (one per coarse-scan step, then 1 per
/// bisection iteration). If `f` does expensive work — e.g. opens
/// kernels, allocates — wrap it in a memoising closure or capture an
/// already-opened `EphemerisSession` by reference.
pub fn find_root<F>(t0: Instant, t1: Instant, mut f: F, opts: SearchOptions) -> SkyResult<Option<Instant>>
where
    F: FnMut(Instant) -> SkyResult<f64>,
{
    let jd_start = t0.jd_utc();
    let jd_end = t1.jd_utc();
    if jd_end <= jd_start {
        return Err(SkyError::InvalidCivilTime(format!(
            "find_root window [t0, t1] must be ordered; got jd={}..{}",
            jd_start, jd_end
        )));
    }

    let step_days = opts.coarse_step_seconds / SECONDS_PER_DAY;
    let mut prev_jd = jd_start;
    let mut prev_f = f(t0)?;
    let mut next_jd = jd_start + step_days;
    while next_jd <= jd_end + step_days * 0.5 {
        let next_jd_clamped = next_jd.min(jd_end);
        let next_instant = Instant::from_jd_tdb(next_jd_clamped + 0.0).or_else(|_| {
            // Fall back through UTC if the TDB path errors near limits.
            Ok::<_, SkyError>(advance_utc(t0, next_jd_clamped - jd_start))
        })?;
        let next_f = f(next_instant)?;
        if prev_f == 0.0 {
            return Ok(Some(advance_utc(t0, prev_jd - jd_start)));
        }
        if prev_f.signum() != next_f.signum() {
            let lo = advance_utc(t0, prev_jd - jd_start);
            let hi = advance_utc(t0, next_jd_clamped - jd_start);
            return Ok(Some(bisect(lo, hi, prev_f, next_f, &mut f, opts)?));
        }
        prev_jd = next_jd_clamped;
        prev_f = next_f;
        next_jd += step_days;
    }
    Ok(None)
}

/// Coarsely scan the window and return every detected sign change,
/// bisected to tolerance. Useful when more than one event is expected
/// (e.g. transit + return in the same year).
pub fn find_all_roots<F>(t0: Instant, t1: Instant, mut f: F, opts: SearchOptions) -> SkyResult<Vec<Instant>>
where
    F: FnMut(Instant) -> SkyResult<f64>,
{
    let mut out = Vec::new();
    let jd_start = t0.jd_utc();
    let jd_end = t1.jd_utc();
    let step_days = opts.coarse_step_seconds / SECONDS_PER_DAY;
    let mut prev_jd = jd_start;
    let mut prev_f = f(t0)?;
    let mut next_jd = jd_start + step_days;
    while next_jd <= jd_end + step_days * 0.5 {
        let next_jd_clamped = next_jd.min(jd_end);
        let next_instant = advance_utc(t0, next_jd_clamped - jd_start);
        let next_f = f(next_instant)?;
        if prev_f.signum() != next_f.signum() && prev_f != 0.0 {
            let lo = advance_utc(t0, prev_jd - jd_start);
            let hi = next_instant;
            let root = bisect(lo, hi, prev_f, next_f, &mut f, opts)?;
            out.push(root);
        }
        prev_jd = next_jd_clamped;
        prev_f = next_f;
        next_jd += step_days;
    }
    Ok(out)
}

fn bisect<F>(
    mut lo: Instant,
    mut hi: Instant,
    mut f_lo: f64,
    _f_hi: f64,
    f: &mut F,
    opts: SearchOptions,
) -> SkyResult<Instant>
where
    F: FnMut(Instant) -> SkyResult<f64>,
{
    let tol_days = opts.tolerance_seconds / SECONDS_PER_DAY;
    for _ in 0..opts.max_iterations {
        let lo_jd = lo.jd_utc();
        let hi_jd = hi.jd_utc();
        if (hi_jd - lo_jd) < tol_days {
            return Ok(midpoint(lo, hi));
        }
        let mid = midpoint(lo, hi);
        let f_mid = f(mid)?;
        if f_mid == 0.0 {
            return Ok(mid);
        }
        if f_lo.signum() != f_mid.signum() {
            hi = mid;
        } else {
            lo = mid;
            f_lo = f_mid;
        }
    }
    Ok(midpoint(lo, hi))
}

fn midpoint(lo: Instant, hi: Instant) -> Instant {
    let lo_jd = lo.jd_utc();
    let hi_jd = hi.jd_utc();
    advance_utc(lo, (hi_jd - lo_jd) * 0.5)
}

fn advance_utc(base: Instant, days_delta: f64) -> Instant {
    let new_utc = base.utc().add_days(days_delta);
    Instant::from_utc(new_utc)
}
