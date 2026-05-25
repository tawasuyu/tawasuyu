//! Planetary stations: the instants when a body's ecliptic longitude
//! rate `dλ/dt` crosses zero, marking the transition between direct and
//! retrograde motion.
//!
//! Reduces to one call into [`cosmos_sky::find_root`] on the apparent
//! longitude rate exposed by [`cosmos_sky::ApparentPosition::ecliptic_velocity`].
//! Use [`next_station`] for the next station after a given instant, or
//! [`all_stations`] for every station inside a window.

use cosmos_sky::{find_all_roots, find_root, Body, EphemerisSession, Instant, SearchOptions};

use crate::error::{AstrologyError, AstrologyResult};

/// Direction of the transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationKind {
    /// Direct → retrograde. The body was moving forward and is now
    /// about to move backward.
    Retrograde,
    /// Retrograde → direct. The body finishes the retrograde phase
    /// and resumes forward motion.
    Direct,
}

#[derive(Debug, Clone, Copy)]
pub struct Station {
    pub body: Body,
    pub instant: Instant,
    pub kind: StationKind,
}

/// Find the next station of `body` at or after `after`. Returns
/// `Ok(None)` if no sign-change in the longitude rate is detected
/// within `max_window_days`.
pub fn next_station(
    session: &EphemerisSession,
    body: Body,
    after: Instant,
    max_window_days: f64,
) -> AstrologyResult<Option<Station>> {
    let t1 = Instant::from_utc(after.utc().add_days(max_window_days));
    let opts = station_search_options(body);

    let zero = find_root(
        after,
        t1,
        |t: Instant| {
            let pos = session.body_apparent(body, t, None)?;
            Ok(pos.ecliptic_velocity.longitude_rate_rad_per_day)
        },
        opts,
    )
    .map_err(AstrologyError::Sky)?;

    match zero {
        None => Ok(None),
        Some(t_zero) => {
            let kind = classify_station(session, body, t_zero)?;
            Ok(Some(Station {
                body,
                instant: t_zero,
                kind,
            }))
        }
    }
}

/// Find every station of `body` in `[after, after + max_window_days]`.
/// Useful for plotting retrograde periods or summarising a year.
pub fn all_stations(
    session: &EphemerisSession,
    body: Body,
    after: Instant,
    max_window_days: f64,
) -> AstrologyResult<Vec<Station>> {
    let t1 = Instant::from_utc(after.utc().add_days(max_window_days));
    let opts = station_search_options(body);

    let zeros = find_all_roots(
        after,
        t1,
        |t: Instant| {
            let pos = session.body_apparent(body, t, None)?;
            Ok(pos.ecliptic_velocity.longitude_rate_rad_per_day)
        },
        opts,
    )
    .map_err(AstrologyError::Sky)?;

    let mut out = Vec::with_capacity(zeros.len());
    for t in zeros {
        let kind = classify_station(session, body, t)?;
        out.push(Station {
            body,
            instant: t,
            kind,
        });
    }
    Ok(out)
}

/// Coarse-scan / tolerance defaults for each body, tuned so the scan
/// brackets a single station without missing one. Fast bodies (Moon,
/// Mercury, Venus) need finer sampling near zero-rate; outer planets
/// can use a daily step.
fn station_search_options(body: Body) -> SearchOptions {
    use Body::*;
    let coarse_step_seconds = match body {
        // Moon never stations (always direct), but we still keep a
        // sensible step in case callers feed it in.
        Moon => 3_600.0 * 6.0,
        Mercury | Venus => 3_600.0 * 6.0,
        Mars => 86_400.0,
        Jupiter | Saturn | Uranus | Neptune | Pluto => 86_400.0,
        _ => 86_400.0,
    };
    SearchOptions {
        coarse_step_seconds,
        tolerance_seconds: 30.0, // 30 s is well below any meaningful astrological resolution
        max_iterations: 80,
    }
}

/// Sample the rate slightly before `t` to decide the direction of the
/// crossing: a positive rate before zero ⇒ direct → retrograde
/// (Retrograde station); negative before zero ⇒ retrograde → direct.
fn classify_station(
    session: &EphemerisSession,
    body: Body,
    t: Instant,
) -> AstrologyResult<StationKind> {
    let probe = Instant::from_utc(t.utc().add_days(-0.5)); // half a day earlier
    let pos = session.body_apparent(body, probe, None).map_err(AstrologyError::Sky)?;
    let rate = pos.ecliptic_velocity.longitude_rate_rad_per_day;
    Ok(if rate > 0.0 {
        StationKind::Retrograde
    } else {
        StationKind::Direct
    })
}
