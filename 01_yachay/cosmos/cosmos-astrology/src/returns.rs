//! Planetary returns: find the instant at which a body's ecliptic
//! longitude crosses its natal value again.
//!
//! The solar return (Sun back to natal Sun) is the classical annual
//! "revolution"; the lunar return is the monthly one; and any planet
//! has its own synodic-style return cycle.
//!
//! All returns reduce to one primitive: bisect on
//! `f(t) = signed_angular_distance( body_longitude_at(t),  natal_longitude )`.
//! The bisector lives in [`cosmos_sky::find_root`]; this module just
//! wraps it with body-aware default search windows.

use cosmos_sky::{find_root, Body, EphemerisSession, Instant, SearchOptions};

use crate::angles::signed_delta_rad;
use crate::error::{AstrologyError, AstrologyResult};

/// Estimated synodic period of `body` around the geocenter, in days.
/// Used to pick a search window and a coarse-scan step. Values are
/// nominal — the search bracket adds slack so they need not be exact.
fn nominal_period_days(body: Body) -> f64 {
    match body {
        Body::Moon => 27.321_661, // sidereal month
        Body::Sun => 365.256_363, // sidereal year (≈ tropical year for return)
        Body::Mercury => 87.969,
        Body::Venus => 224.701,
        Body::Mars => 686.971,
        Body::Jupiter => 4_332.59,
        Body::Saturn => 10_759.22,
        Body::Uranus => 30_688.5,
        Body::Neptune => 60_182.0,
        Body::Pluto => 90_560.0,
        // Lunar nodes: 18.6-year cycle. Lilith: ~8.85-year cycle.
        Body::MeanNode | Body::TrueNode => 6_793.4,
        Body::MeanLilith | Body::TrueLilith => 3_232.6,
        // Asteroids (rough): Ceres 4.6 yr, others nearby.
        Body::Ceres => 1_681.6,
        Body::Pallas => 1_686.0,
        Body::Juno => 1_595.0,
        Body::Vesta => 1_325.0,
        // Centaurs / TNOs are very slow. Pick a conservative upper bound.
        _ => 100_000.0,
    }
}

/// Find the next instant at or after `after` where `body`'s apparent
/// ecliptic longitude (tropical, of date) equals `natal_longitude_rad`.
///
/// The search walks forward for up to ~1.5× the body's nominal synodic
/// period, which always brackets the next return. Pass a custom
/// `max_window_days` if you need a tighter or looser bound (e.g.
/// rectifying with a degenerate fit).
pub fn next_return(
    session: &EphemerisSession,
    body: Body,
    natal_longitude_rad: f64,
    after: Instant,
    max_window_days: Option<f64>,
) -> AstrologyResult<Instant> {
    let nominal = nominal_period_days(body);
    let window = max_window_days.unwrap_or(nominal * 1.5);
    let t1 = Instant::from_utc(after.utc().add_days(window));

    // Coarse-scan step: a fraction of the nominal period that resolves
    // a single revolution into ~60 samples (enough for monotone signed
    // delta but coarse enough not to slow outer-body searches).
    let step_seconds = (nominal * 86_400.0 / 60.0).max(60.0);
    let opts = SearchOptions {
        coarse_step_seconds: step_seconds,
        tolerance_seconds: 1.0,
        max_iterations: 80,
    };

    let result = find_root(
        after,
        t1,
        |t: Instant| {
            let pos = session.body_apparent(body, t, None)?;
            Ok(signed_delta_rad(
                pos.ecliptic_of_date.longitude_rad,
                natal_longitude_rad,
            ))
        },
        opts,
    )?;

    result.ok_or_else(|| {
        AstrologyError::BodyUnavailable(format!(
            "no return of {} to {:.4}° in {:.1} days after {}",
            body.name(),
            natal_longitude_rad.to_degrees(),
            window,
            after,
        ))
    })
}

