//! Lunar eclipse detection (Phase 3, step 9).
//!
//! Geometric approach: at any given epoch, compute the Moon's position
//! relative to the anti-Sun direction (which is where Earth's shadow
//! is centred), then compare the angular separation against the umbral
//! and penumbral radii at the Moon's distance. A "next eclipse" search
//! steps through full moons and tests each one.
//!
//! This is observer-independent — lunar eclipses look the same from any
//! point on Earth's night side. Solar eclipses (which need a topocentric
//! treatment) are a separate piece.

use cosmos_core::constants::AU_KM;
use cosmos_core::Vector3;
use cosmos_ephemeris::jpl::SpkFile;

use crate::oracle::OracleError;

const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

/// Astronomical body radii in km. Reference: IAU 2015 nominal.
const R_SUN_KM: f64 = 695_700.0;
const R_EARTH_KM: f64 = 6_378.137;
const R_MOON_KM: f64 = 1_737.4;

/// Atmospheric enlargement factor on Earth's shadow at lunar distance.
/// Convention used by NASA / Espenak (Chauvenet's 1/50 rule expressed
/// as a 1.02 multiplier on the umbra and penumbra radii). Swiss
/// applies an equivalent atmospheric correction internally.
const ATM_ENLARGEMENT: f64 = 1.02;

/// Type of lunar eclipse at the queried instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LunarEclipseKind {
    /// No eclipse — Moon is outside the penumbra.
    None,
    /// Penumbral — at least part of Moon is in the penumbral shadow.
    Penumbral,
    /// Partial umbral — at least part of Moon is in the umbral shadow,
    /// but not the entire disc.
    Partial,
    /// Total — entire lunar disc is within the umbral shadow.
    Total,
}

/// Geometric description of a lunar-eclipse-instant snapshot.
#[derive(Debug, Clone, Copy)]
pub struct LunarEclipseSnapshot {
    pub kind: LunarEclipseKind,
    /// Angular distance from anti-Sun to Moon centre, radians.
    pub gamma_rad: f64,
    /// Earth's umbral angular radius at the Moon's distance, radians.
    pub umbra_radius_rad: f64,
    /// Earth's penumbral angular radius at the Moon's distance, radians.
    pub penumbra_radius_rad: f64,
    /// Moon's apparent angular semi-diameter, radians.
    pub moon_radius_rad: f64,
}

impl LunarEclipseSnapshot {
    /// Penumbral magnitude (1.0 = entire Moon disc just inside the
    /// penumbra; 0.0 = Moon disc just touching the penumbra edge from
    /// outside; negative = Moon disc clear of penumbra).
    pub fn penumbral_magnitude(&self) -> f64 {
        (self.penumbra_radius_rad + self.moon_radius_rad - self.gamma_rad)
            / (2.0 * self.moon_radius_rad)
    }

    /// Umbral magnitude (analogous to penumbral, scaled to umbra).
    pub fn umbral_magnitude(&self) -> f64 {
        (self.umbra_radius_rad + self.moon_radius_rad - self.gamma_rad)
            / (2.0 * self.moon_radius_rad)
    }
}

/// Evaluate the lunar-eclipse geometry at the given TDB instant.
/// Returns the snapshot. The `kind` field is `None` if there is no
/// eclipse.
///
/// Convention: the Sun's position is light-time-corrected (the shadow
/// at observation time t points away from where the Sun WAS 8 min
/// ago); the Moon is at its **geometric** position at time t (the
/// shadow physically intersects the Moon's true location, not its
/// apparent location 1.3 s earlier). This matches Swiss `swe.lun_eclipse`.
pub fn lunar_eclipse_at(spk: &SpkFile, jd_tdb: f64) -> Result<LunarEclipseSnapshot, OracleError> {
    let sun_pos = astrometric_geocentric_km(spk, 10, jd_tdb)?;
    let moon_pos = geocentric_position_km(spk, 301, jd_tdb)?;

    let sun_dist = magnitude(&sun_pos);
    let moon_dist = magnitude(&moon_pos);

    // Anti-Sun direction (where Earth's shadow points) is exactly
    // opposite the Sun direction, as a unit vector.
    let anti_sun = Vector3::new(-sun_pos.x / sun_dist, -sun_pos.y / sun_dist, -sun_pos.z / sun_dist);
    let moon_unit = Vector3::new(moon_pos.x / moon_dist, moon_pos.y / moon_dist, moon_pos.z / moon_dist);

    // Angular distance Moon to anti-Sun direction.
    let dot = anti_sun.x * moon_unit.x + anti_sun.y * moon_unit.y + anti_sun.z * moon_unit.z;
    let dot = dot.clamp(-1.0, 1.0);
    let gamma_rad = libm::acos(dot);

    // Shadow geometry. The standard formulas (Meeus 54.1):
    //   π_M = asin(R_earth / d_moon)   — lunar parallax
    //   π_S = asin(R_earth / d_sun)    — solar parallax
    //   s_S = asin(R_sun / d_sun)      — solar semi-diameter
    //   s_M = asin(R_moon / d_moon)    — lunar semi-diameter
    //   umbra_radius_at_moon    = π_M + π_S − s_S
    //   penumbra_radius_at_moon = π_M + π_S + s_S
    let pi_m = libm::asin(R_EARTH_KM / moon_dist);
    let pi_s = libm::asin(R_EARTH_KM / sun_dist);
    let s_s = libm::asin(R_SUN_KM / sun_dist);
    let s_m = libm::asin(R_MOON_KM / moon_dist);

    let umbra_radius_rad = (pi_m + pi_s - s_s) * ATM_ENLARGEMENT;
    let penumbra_radius_rad = (pi_m + pi_s + s_s) * ATM_ENLARGEMENT;

    // Classify.
    let kind = if gamma_rad + s_m < umbra_radius_rad {
        LunarEclipseKind::Total
    } else if gamma_rad - s_m < umbra_radius_rad {
        LunarEclipseKind::Partial
    } else if gamma_rad - s_m < penumbra_radius_rad {
        LunarEclipseKind::Penumbral
    } else {
        LunarEclipseKind::None
    };

    Ok(LunarEclipseSnapshot {
        kind,
        gamma_rad,
        umbra_radius_rad,
        penumbra_radius_rad,
        moon_radius_rad: s_m,
    })
}

/// Find the next lunar eclipse maximum (any kind) starting at `jd_start`
/// (TDB). Returns `(jd_tdb_max, snapshot)` or `None` if no eclipse is
/// found within `max_synodic_months` lunar cycles.
pub fn next_lunar_eclipse(
    spk: &SpkFile,
    jd_start_tdb: f64,
    max_synodic_months: usize,
) -> Result<Option<(f64, LunarEclipseSnapshot)>, OracleError> {
    let mut jd = jd_start_tdb;
    for _ in 0..max_synodic_months {
        let full_moon = next_full_moon_tdb(spk, jd)?;
        // Eclipse maximum is approximately at full moon, but the actual
        // minimum-gamma instant can be offset by tens of minutes. Find
        // it by minimising gamma around the full moon.
        let max_jd = refine_eclipse_max(spk, full_moon, 6.0 / 24.0)?;
        let snap = lunar_eclipse_at(spk, max_jd)?;
        if snap.kind != LunarEclipseKind::None {
            return Ok(Some((max_jd, snap)));
        }
        jd = full_moon + 27.0;
    }
    Ok(None)
}

/// Find the next instant of full moon (Sun-Moon ecliptic longitude
/// difference = 180°) on or after `jd_start_tdb`. Uses ICRF Cartesian
/// without explicitly building ecliptic — equivalently, finds the
/// instant when (Moon · -Sun) is maximised (i.e., they are most
/// anti-parallel). Implementation: scan + bisection on the sign-changed
/// time-derivative of cos(Moon, anti-Sun).
fn next_full_moon_tdb(spk: &SpkFile, jd_start_tdb: f64) -> Result<f64, OracleError> {
    let f = |jd: f64| -> Result<f64, OracleError> {
        // Returns Moon-Sun heliocentric longitude difference, mod 360°,
        // mapped to (-180°, +180°]. Full moon ⇔ 0° here.
        let moon = geocentric_position_km(spk, 301, jd)?;
        let sun = geocentric_position_km(spk, 10, jd)?;
        let lon_moon = libm::atan2(moon.y, moon.x);
        let lon_sun = libm::atan2(sun.y, sun.x);
        let mut diff = (lon_moon - lon_sun - PI).rem_euclid(TAU);
        if diff > PI {
            diff -= TAU;
        }
        Ok(diff)
    };

    // Coarse scan: 6-hour steps over up to 32 days (one synodic month).
    const STEP_HOURS: f64 = 6.0;
    let step_days = STEP_HOURS / 24.0;
    let n_steps = ((30.0 + STEP_HOURS) / step_days) as usize;

    let mut prev_jd = jd_start_tdb;
    let mut prev_f = f(prev_jd)?;
    for i in 1..=n_steps {
        let jd = jd_start_tdb + (i as f64) * step_days;
        let cur_f = f(jd)?;
        // Full moon when f changes sign from negative to positive (Moon
        // catching up to anti-Sun direction). Reject the wrap from +π to
        // −π by requiring |jump| < π.
        if (cur_f - prev_f).abs() < PI && prev_f < 0.0 && cur_f >= 0.0 {
            return bisect_root(&f, prev_jd, jd, 32);
        }
        prev_jd = jd;
        prev_f = cur_f;
    }
    Err(OracleError::Inner(
        "no full moon found within ~30 days from start".into(),
    ))
}

/// Refine the time of minimum gamma (= eclipse maximum) by parabolic
/// interpolation around an estimated time, sampling within ±half_window
/// days.
fn refine_eclipse_max(spk: &SpkFile, jd_estimate: f64, half_window_days: f64) -> Result<f64, OracleError> {
    let g = |jd: f64| -> Result<f64, OracleError> {
        Ok(lunar_eclipse_at(spk, jd)?.gamma_rad)
    };

    // 3-point parabolic minimisation, refined by golden-section
    // bracketing if the parabolic step lands outside the bracket.
    let mut a = jd_estimate - half_window_days;
    let mut b = jd_estimate;
    let mut c = jd_estimate + half_window_days;
    let mut g_a = g(a)?;
    let mut g_b = g(b)?;
    let mut g_c = g(c)?;

    for _ in 0..40 {
        // Parabolic step: minimum of fitted quadratic.
        let denom = (b - a) * (g_b - g_c) - (b - c) * (g_b - g_a);
        let numer = (b - a) * (b - a) * (g_b - g_c) - (b - c) * (b - c) * (g_b - g_a);
        let next = if denom.abs() > 1.0e-30 {
            b - 0.5 * numer / denom
        } else {
            0.5 * (a + c)
        };
        let next = next.clamp(a + 1.0e-6, c - 1.0e-6);
        let g_next = g(next)?;
        if (next - b).abs() < 1.0 / 86_400.0 {
            return Ok(next);
        }
        // Update bracket.
        if next < b {
            if g_next < g_b {
                c = b;
                g_c = g_b;
                b = next;
                g_b = g_next;
            } else {
                a = next;
                g_a = g_next;
            }
        } else if g_next < g_b {
            a = b;
            g_a = g_b;
            b = next;
            g_b = g_next;
        } else {
            c = next;
            g_c = g_next;
        }
    }
    Ok(b)
}

/// Geocentric Cartesian position (km) of a body in ICRF, using
/// (body wrt EMB) − (Earth wrt EMB) chain.
fn geocentric_position_km(spk: &SpkFile, body: i32, jd_tdb: f64) -> Result<Vector3, OracleError> {
    if body == 301 {
        let (m, _) = spk.compute_state(301, 3, jd_tdb)?;
        let (e, _) = spk.compute_state(399, 3, jd_tdb)?;
        return Ok(Vector3::new(m[0] - e[0], m[1] - e[1], m[2] - e[2]));
    }
    if body == 10 {
        // Sun wrt Earth body. SPK has Sun wrt SSB and we have to compute
        // Earth wrt SSB via the EMB chain.
        let (s, _) = spk.compute_state(10, 0, jd_tdb)?;
        let (e_emb, _) = spk.compute_state(399, 3, jd_tdb)?;
        let (emb_ssb, _) = spk.compute_state(3, 0, jd_tdb)?;
        let earth_ssb = [
            e_emb[0] + emb_ssb[0],
            e_emb[1] + emb_ssb[1],
            e_emb[2] + emb_ssb[2],
        ];
        return Ok(Vector3::new(
            s[0] - earth_ssb[0],
            s[1] - earth_ssb[1],
            s[2] - earth_ssb[2],
        ));
    }
    // Generic case: use direct SPK call (caller supplies SSB or EMB chain).
    let (p, _) = spk.compute_state(body, 0, jd_tdb)?;
    Ok(Vector3::new(p[0], p[1], p[2]))
}

fn magnitude(v: &Vector3) -> f64 {
    libm::sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
}

/// Astrometric (LT-corrected) geocentric position of `body` at obs
/// time `jd_tdb`. Iterates τ over the body-Earth distance until the
/// emit time stabilises; for the Sun τ ≈ 8.3 min, for the Moon τ ≈ 1.3 s.
fn astrometric_geocentric_km(spk: &SpkFile, body: i32, jd_obs: f64) -> Result<Vector3, OracleError> {
    const C_AU_PER_DAY: f64 = 173.144_632_684_669_3;
    const AU_KM_LOCAL: f64 = AU_KM;

    // Earth at observation time (geocentric origin).
    let earth_obs_km = earth_ssb_position(spk, jd_obs)?;
    // Initial τ from the geometric body-Earth distance at obs time.
    let mut tau_days = {
        let body_obs = body_ssb_position(spk, body, jd_obs)?;
        let dx = body_obs.x - earth_obs_km.x;
        let dy = body_obs.y - earth_obs_km.y;
        let dz = body_obs.z - earth_obs_km.z;
        let dist_km = libm::sqrt(dx * dx + dy * dy + dz * dz);
        (dist_km / AU_KM_LOCAL) / C_AU_PER_DAY
    };
    let mut body_emit = Vector3::zeros();
    for _ in 0..6 {
        let jd_emit = jd_obs - tau_days;
        body_emit = body_ssb_position(spk, body, jd_emit)?;
        let dx = body_emit.x - earth_obs_km.x;
        let dy = body_emit.y - earth_obs_km.y;
        let dz = body_emit.z - earth_obs_km.z;
        let dist_km = libm::sqrt(dx * dx + dy * dy + dz * dz);
        let new_tau = (dist_km / AU_KM_LOCAL) / C_AU_PER_DAY;
        let converged = (new_tau - tau_days).abs() < 1.0e-15;
        tau_days = new_tau;
        if converged {
            break;
        }
    }
    Ok(Vector3::new(
        body_emit.x - earth_obs_km.x,
        body_emit.y - earth_obs_km.y,
        body_emit.z - earth_obs_km.z,
    ))
}

fn earth_ssb_position(spk: &SpkFile, jd_tdb: f64) -> Result<Vector3, OracleError> {
    let (e, _) = spk.compute_state(399, 3, jd_tdb).map_err(OracleError::from)?;
    let (emb, _) = spk.compute_state(3, 0, jd_tdb).map_err(OracleError::from)?;
    Ok(Vector3::new(e[0] + emb[0], e[1] + emb[1], e[2] + emb[2]))
}

fn body_ssb_position(spk: &SpkFile, body: i32, jd_tdb: f64) -> Result<Vector3, OracleError> {
    if body == 301 {
        let (m, _) = spk.compute_state(301, 3, jd_tdb).map_err(OracleError::from)?;
        let (emb, _) = spk.compute_state(3, 0, jd_tdb).map_err(OracleError::from)?;
        return Ok(Vector3::new(m[0] + emb[0], m[1] + emb[1], m[2] + emb[2]));
    }
    let (p, _) = spk.compute_state(body, 0, jd_tdb).map_err(OracleError::from)?;
    Ok(Vector3::new(p[0], p[1], p[2]))
}

fn bisect_root<F>(f: &F, mut lo: f64, mut hi: f64, max_iter: usize) -> Result<f64, OracleError>
where
    F: Fn(f64) -> Result<f64, OracleError>,
{
    let mut f_lo = f(lo)?;
    for _ in 0..max_iter {
        let mid = 0.5 * (lo + hi);
        let f_mid = f(mid)?;
        if (hi - lo) < 1.0 / 86_400.0 {
            return Ok(mid);
        }
        if f_lo.signum() == f_mid.signum() {
            lo = mid;
            f_lo = f_mid;
        } else {
            hi = mid;
        }
    }
    Ok(0.5 * (lo + hi))
}

// AU_KM is referenced in this module's public API surface. Keeping
// the import live so future code that needs km↔AU conversion has it.
#[allow(dead_code)]
const _AU_KM: f64 = AU_KM;

// =============================================================================
// Solar eclipses (local — observer-specific)
// =============================================================================

use crate::topocentric::Observer;

#[derive(Debug, Clone, Copy)]
pub struct LocalSolarEclipseSnapshot {
    pub kind: SolarEclipseKind,
    /// Angle between topocentric Sun and Moon directions, radians.
    pub angular_separation_rad: f64,
    /// Apparent angular semi-diameter of the Sun as seen from observer.
    pub sun_radius_rad: f64,
    /// Apparent angular semi-diameter of the Moon as seen from observer.
    pub moon_radius_rad: f64,
    /// Eclipse magnitude — fraction of Sun diameter covered, on the
    /// astronomical convention (1.0 = total at second contact, > 1.0
    /// inside totality, 0.0 = bare contact, < 0.0 = no contact).
    pub magnitude: f64,
    /// Fraction of the Sun's *area* covered by the Moon's disc. Useful
    /// for irradiance estimates; 1.0 = totality.
    pub fraction_area_covered: f64,
}

/// Evaluate the local-solar-eclipse geometry at the given TDB instant
/// for the requested observer. Uses the same astrometric pipeline as
/// the global solar eclipse (LT-corrected Sun and Moon) plus the
/// observer's WGS-84 position rotated into the apparent TET frame.
pub fn local_solar_eclipse_at(
    spk: &SpkFile,
    jd_tdb: f64,
    observer: &Observer,
    delta_t_seconds: f64,
) -> Result<LocalSolarEclipseSnapshot, OracleError> {
    let (sun_topo, moon_topo) = topocentric_sun_moon_tet(spk, jd_tdb, observer, delta_t_seconds)?;

    let sun_dist_km = magnitude(&sun_topo);
    let moon_dist_km = magnitude(&moon_topo);

    let dot = sun_topo.x * moon_topo.x + sun_topo.y * moon_topo.y + sun_topo.z * moon_topo.z;
    let cos_sep = (dot / (sun_dist_km * moon_dist_km)).clamp(-1.0, 1.0);
    let separation = libm::acos(cos_sep);

    let sun_radius = libm::asin(R_SUN_KM / sun_dist_km);
    let moon_radius = libm::asin(R_MOON_KM / moon_dist_km);

    // Classification: standard "two-disc overlap" geometry.
    let kind = if separation > sun_radius + moon_radius {
        SolarEclipseKind::None
    } else if separation + moon_radius <= sun_radius {
        // Moon fully inside Sun's disc → annular ring visible.
        SolarEclipseKind::Annular
    } else if separation + sun_radius <= moon_radius {
        // Sun fully covered by Moon → total.
        SolarEclipseKind::Total
    } else {
        SolarEclipseKind::Partial
    };

    // Magnitude: fraction of Sun's diameter that is occulted by the Moon.
    let magnitude = (sun_radius + moon_radius - separation) / (2.0 * sun_radius);
    let fraction_area_covered = disc_overlap_area_fraction(separation, sun_radius, moon_radius);

    Ok(LocalSolarEclipseSnapshot {
        kind,
        angular_separation_rad: separation,
        sun_radius_rad: sun_radius,
        moon_radius_rad: moon_radius,
        magnitude,
        fraction_area_covered,
    })
}

/// Find the next solar eclipse **visible** from `observer` starting at
/// `jd_start_tdb` — the Sun must be above the horizon at the moment of
/// maximum eclipse. Returns the event maximum (TDB) and snapshot, or
/// `None` if no eclipse is found within `max_synodic_months` lunar
/// cycles.
///
/// "Visible" means the Sun's centre is above the geometric horizon
/// (altitude > 0) at the local maximum. Eclipses that touch the
/// observer's location while the Sun is below the horizon are skipped.
pub fn next_local_solar_eclipse(
    spk: &SpkFile,
    jd_start_tdb: f64,
    observer: &Observer,
    delta_t_seconds: f64,
    max_synodic_months: usize,
) -> Result<Option<(f64, LocalSolarEclipseSnapshot)>, OracleError> {
    let mut jd = jd_start_tdb;
    for _ in 0..max_synodic_months {
        let new_moon = next_new_moon_tdb(spk, jd)?;
        let max_jd = refine_local_eclipse_max(spk, observer, delta_t_seconds, new_moon, 6.0 / 24.0)?;
        let snap = local_solar_eclipse_at(spk, max_jd, observer, delta_t_seconds)?;
        if snap.kind != SolarEclipseKind::None
            && sun_is_above_horizon(spk, max_jd, observer, delta_t_seconds)?
        {
            return Ok(Some((max_jd, snap)));
        }
        jd = new_moon + 27.0;
    }
    Ok(None)
}

/// Returns true if the Sun is above the geometric horizon for `observer`
/// at the given TDB instant. Computes the altitude of the topocentric
/// apparent Sun and tests > 0.
fn sun_is_above_horizon(
    spk: &SpkFile,
    jd_tdb: f64,
    observer: &Observer,
    delta_t_seconds: f64,
) -> Result<bool, OracleError> {
    use cosmos_time::julian::JulianDate;
    use cosmos_time::scales::conversions::ToUT1WithDeltaT;
    use cosmos_time::scales::ToTTFromTDB;
    use cosmos_time::sidereal::GAST;
    use cosmos_time::TDB;

    let (sun_topo_tet, _) = topocentric_sun_moon_tet(spk, jd_tdb, observer, delta_t_seconds)?;

    let tt = TDB::from_julian_date(JulianDate::new(jd_tdb, 0.0))
        .to_tt_greenwich()
        .map_err(|e| OracleError::Inner(format!("TDB→TT: {:?}", e)))?;
    let ut1 = tt
        .to_ut1_with_delta_t(delta_t_seconds)
        .map_err(|e| OracleError::Inner(format!("TT→UT1: {:?}", e)))?;
    let location = cosmos_core::Location::from_degrees(
        observer.lat_rad.to_degrees(),
        observer.lon_rad.to_degrees(),
        observer.elev_m,
    )
    .map_err(|e| OracleError::Inner(format!("Location: {:?}", e)))?;
    let gast = GAST::from_ut1_and_tt(&ut1, &tt)
        .map_err(|e| OracleError::Inner(format!("GAST: {:?}", e)))?;
    let last_rad = gast.to_last(&location).angle().radians();

    let (alt, _) = crate::topocentric::alt_az_from_topocentric(
        [sun_topo_tet.x, sun_topo_tet.y, sun_topo_tet.z],
        observer.lat_rad,
        last_rad,
    );
    Ok(alt > 0.0)
}

/// Refine the time of minimum topocentric Sun-Moon separation around
/// `jd_estimate`, sampling within ±half_window days. Same parabolic
/// minimisation as the global-eclipse path.
fn refine_local_eclipse_max(
    spk: &SpkFile,
    observer: &Observer,
    delta_t_seconds: f64,
    jd_estimate: f64,
    half_window_days: f64,
) -> Result<f64, OracleError> {
    let g = |jd: f64| -> Result<f64, OracleError> {
        Ok(local_solar_eclipse_at(spk, jd, observer, delta_t_seconds)?.angular_separation_rad)
    };

    let mut a = jd_estimate - half_window_days;
    let mut b = jd_estimate;
    let mut c = jd_estimate + half_window_days;
    let mut g_a = g(a)?;
    let mut g_b = g(b)?;
    let mut g_c = g(c)?;

    for _ in 0..40 {
        let denom = (b - a) * (g_b - g_c) - (b - c) * (g_b - g_a);
        let numer = (b - a) * (b - a) * (g_b - g_c) - (b - c) * (b - c) * (g_b - g_a);
        let next = if denom.abs() > 1.0e-30 {
            b - 0.5 * numer / denom
        } else {
            0.5 * (a + c)
        };
        let next = next.clamp(a + 1.0e-6, c - 1.0e-6);
        let g_next = g(next)?;
        if (next - b).abs() < 1.0 / 86_400.0 {
            return Ok(next);
        }
        if next < b {
            if g_next < g_b {
                c = b;
                g_c = g_b;
                b = next;
                g_b = g_next;
            } else {
                a = next;
                g_a = g_next;
            }
        } else if g_next < g_b {
            a = b;
            g_a = g_b;
            b = next;
            g_b = g_next;
        } else {
            c = next;
            g_c = g_next;
        }
    }
    Ok(b)
}

/// Compute topocentric (observer-relative) apparent Sun and Moon
/// vectors in the TET frame.
fn topocentric_sun_moon_tet(
    spk: &SpkFile,
    jd_tdb: f64,
    observer: &Observer,
    delta_t_seconds: f64,
) -> Result<(Vector3, Vector3), OracleError> {
    use cosmos_time::julian::JulianDate;
    use cosmos_time::scales::conversions::ToUT1WithDeltaT;
    use cosmos_time::scales::ToTTFromTDB;
    use cosmos_time::{NutationCalculator, TDB};

    // Apparent geocentric Sun and Moon in ICRF (km).
    let sun_geo_icrf = astrometric_geocentric_km(spk, 10, jd_tdb)?;
    let moon_geo_icrf = astrometric_geocentric_km(spk, 301, jd_tdb)?;

    // Time-scale chain.
    let tt = TDB::from_julian_date(JulianDate::new(jd_tdb, 0.0))
        .to_tt_greenwich()
        .map_err(|e| OracleError::Inner(format!("TDB→TT: {:?}", e)))?;
    let ut1 = tt
        .to_ut1_with_delta_t(delta_t_seconds)
        .map_err(|e| OracleError::Inner(format!("TT→UT1: {:?}", e)))?;

    // Rotate Sun and Moon to TET via the IAU 2006/2000A NPB matrix.
    let nut = tt
        .nutation_iau2006a()
        .map_err(|e| OracleError::Inner(format!("nutation: {:?}", e)))?;
    let tt_jd = tt.to_julian_date();
    let t_centuries =
        cosmos_core::utils::jd_to_centuries(tt_jd.jd1(), tt_jd.jd2());
    let npb = cosmos_core::precession::PrecessionIAU2006::new().npb_matrix_iau2006a(
        t_centuries,
        nut.nutation_longitude(),
        nut.nutation_obliquity(),
    );
    let sun_tet = npb * sun_geo_icrf;
    let moon_tet = npb * moon_geo_icrf;

    // Observer in TET via the existing topocentric helper.
    let observer_tet = crate::topocentric::observer_position_tet_km(observer, &ut1, &tt)?;

    let sun_topo = Vector3::new(
        sun_tet.x - observer_tet.x,
        sun_tet.y - observer_tet.y,
        sun_tet.z - observer_tet.z,
    );
    let moon_topo = Vector3::new(
        moon_tet.x - observer_tet.x,
        moon_tet.y - observer_tet.y,
        moon_tet.z - observer_tet.z,
    );
    Ok((sun_topo, moon_topo))
}

/// Area of the lens-shaped overlap between two discs of radii `r1` and
/// `r2` whose centres are at angular distance `d`, expressed as a
/// fraction of disc-1's area. Used to compute the fraction of the
/// Sun's disc the Moon covers during a partial eclipse.
fn disc_overlap_area_fraction(d: f64, r1: f64, r2: f64) -> f64 {
    if d >= r1 + r2 {
        return 0.0;
    }
    if d + r2 <= r1 {
        return (r2 / r1).powi(2); // disc 2 fully inside disc 1 — annular
    }
    if d + r1 <= r2 {
        return 1.0; // disc 1 fully inside disc 2 — total
    }
    let r1_sq = r1 * r1;
    let r2_sq = r2 * r2;
    let d_sq = d * d;
    let a1 = r1_sq * libm::acos(((d_sq + r1_sq - r2_sq) / (2.0 * d * r1)).clamp(-1.0, 1.0));
    let a2 = r2_sq * libm::acos(((d_sq + r2_sq - r1_sq) / (2.0 * d * r2)).clamp(-1.0, 1.0));
    let triangle = 0.5
        * libm::sqrt(
            ((-d + r1 + r2) * (d + r1 - r2) * (d - r1 + r2) * (d + r1 + r2)).max(0.0),
        );
    let overlap = a1 + a2 - triangle;
    overlap / (std::f64::consts::PI * r1_sq)
}

// =============================================================================
// Solar eclipses (global)
// =============================================================================

/// Type of solar eclipse (global view — anywhere on Earth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolarEclipseKind {
    /// No eclipse globally on Earth.
    None,
    /// Only the penumbra touches Earth — observers see a partial
    /// eclipse from somewhere, but no total / annular track.
    Partial,
    /// The Moon's umbra reaches Earth: total eclipse track exists.
    Total,
    /// The Moon's umbra apex is short of Earth, so the antumbra (Sun's
    /// rim visible around the Moon) touches Earth: annular track.
    Annular,
    /// Eclipse begins / ends as annular but is total in the middle of
    /// the track (or vice versa) — a "hybrid" or annular-total eclipse.
    /// We don't separate this from `Total` in v1.
    Hybrid,
}

#[derive(Debug, Clone, Copy)]
pub struct SolarEclipseSnapshot {
    pub kind: SolarEclipseKind,
    /// Perpendicular distance from Earth's centre to the Sun-Moon
    /// shadow axis, in km.
    pub axis_distance_km: f64,
    /// Signed umbra radius at Earth's distance from the Moon, km.
    /// Positive = umbra reaches Earth (total geometry possible).
    /// Negative = umbra apex short of Earth → antumbra (annular).
    pub umbra_radius_at_earth_km: f64,
    pub penumbra_radius_at_earth_km: f64,
}

/// Evaluate the solar-eclipse geometry at the given TDB instant.
/// "Global" means it tests whether *any* point on Earth experiences
/// the eclipse — not whether a specific observer does.
pub fn solar_eclipse_at(spk: &SpkFile, jd_tdb: f64) -> Result<SolarEclipseSnapshot, OracleError> {
    let sun = astrometric_geocentric_km(spk, 10, jd_tdb)?;
    let moon = astrometric_geocentric_km(spk, 301, jd_tdb)?;

    // Moon→Sun vector (line from Moon along which the shadow extends
    // toward Earth and beyond).
    let moon_to_sun = Vector3::new(sun.x - moon.x, sun.y - moon.y, sun.z - moon.z);
    let d_ms = magnitude(&moon_to_sun);
    let d_em = magnitude(&moon);

    // Project Earth's centre (origin) perpendicular to the Sun-Moon
    // line. The parametric line is `P(t) = moon + t · (sun − moon)`;
    // the foot of the perpendicular has `t = −(moon · (sun−moon)) / |moon→sun|²`.
    let dot = moon.x * moon_to_sun.x + moon.y * moon_to_sun.y + moon.z * moon_to_sun.z;
    // |Earth − foot|² = d_em² − (dot / d_ms)²
    let axis_distance_sq = d_em * d_em - (dot / d_ms) * (dot / d_ms);
    let axis_distance = if axis_distance_sq > 0.0 {
        libm::sqrt(axis_distance_sq)
    } else {
        0.0
    };

    // Umbra / penumbra radii at Earth's distance from the Moon.
    let umbra_radius_at_earth =
        R_MOON_KM - d_em * (R_SUN_KM - R_MOON_KM) / d_ms;
    let penumbra_radius_at_earth =
        R_MOON_KM + d_em * (R_SUN_KM + R_MOON_KM) / d_ms;

    let kind = if axis_distance > R_EARTH_KM + penumbra_radius_at_earth {
        SolarEclipseKind::None
    } else if axis_distance > R_EARTH_KM + umbra_radius_at_earth.abs() {
        SolarEclipseKind::Partial
    } else if umbra_radius_at_earth > 0.0 {
        SolarEclipseKind::Total
    } else {
        SolarEclipseKind::Annular
    };

    Ok(SolarEclipseSnapshot {
        kind,
        axis_distance_km: axis_distance,
        umbra_radius_at_earth_km: umbra_radius_at_earth,
        penumbra_radius_at_earth_km: penumbra_radius_at_earth,
    })
}

/// Find the next global solar eclipse maximum (any kind) starting at
/// `jd_start_tdb`. Returns `(jd_tdb_max, snapshot)` or `None` if no
/// eclipse is found within `max_synodic_months` lunar cycles.
pub fn next_solar_eclipse(
    spk: &SpkFile,
    jd_start_tdb: f64,
    max_synodic_months: usize,
) -> Result<Option<(f64, SolarEclipseSnapshot)>, OracleError> {
    let mut jd = jd_start_tdb;
    for _ in 0..max_synodic_months {
        let new_moon = next_new_moon_tdb(spk, jd)?;
        let max_jd = refine_solar_eclipse_max(spk, new_moon, 6.0 / 24.0)?;
        let snap = solar_eclipse_at(spk, max_jd)?;
        if snap.kind != SolarEclipseKind::None {
            return Ok(Some((max_jd, snap)));
        }
        jd = new_moon + 27.0;
    }
    Ok(None)
}

/// Find the next instant of new moon (Sun-Moon ecliptic longitude
/// difference = 0°) on or after `jd_start_tdb`. Equivalent to the
/// previous full-moon search but with target angle 0 instead of π.
fn next_new_moon_tdb(spk: &SpkFile, jd_start_tdb: f64) -> Result<f64, OracleError> {
    let f = |jd: f64| -> Result<f64, OracleError> {
        let moon = geocentric_position_km(spk, 301, jd)?;
        let sun = geocentric_position_km(spk, 10, jd)?;
        let lon_moon = libm::atan2(moon.y, moon.x);
        let lon_sun = libm::atan2(sun.y, sun.x);
        let mut diff = (lon_moon - lon_sun).rem_euclid(TAU);
        if diff > PI {
            diff -= TAU;
        }
        Ok(diff)
    };

    const STEP_HOURS: f64 = 6.0;
    let step_days = STEP_HOURS / 24.0;
    let n_steps = ((30.0 + STEP_HOURS) / step_days) as usize;

    let mut prev_jd = jd_start_tdb;
    let mut prev_f = f(prev_jd)?;
    for i in 1..=n_steps {
        let jd = jd_start_tdb + (i as f64) * step_days;
        let cur_f = f(jd)?;
        if (cur_f - prev_f).abs() < PI && prev_f < 0.0 && cur_f >= 0.0 {
            return bisect_root(&f, prev_jd, jd, 32);
        }
        prev_jd = jd;
        prev_f = cur_f;
    }
    Err(OracleError::Inner(
        "no new moon found within ~30 days from start".into(),
    ))
}

/// Refine the time of minimum axis-distance (= solar eclipse maximum).
fn refine_solar_eclipse_max(
    spk: &SpkFile,
    jd_estimate: f64,
    half_window_days: f64,
) -> Result<f64, OracleError> {
    let g = |jd: f64| -> Result<f64, OracleError> {
        Ok(solar_eclipse_at(spk, jd)?.axis_distance_km)
    };

    let mut a = jd_estimate - half_window_days;
    let mut b = jd_estimate;
    let mut c = jd_estimate + half_window_days;
    let mut g_a = g(a)?;
    let mut g_b = g(b)?;
    let mut g_c = g(c)?;

    for _ in 0..40 {
        let denom = (b - a) * (g_b - g_c) - (b - c) * (g_b - g_a);
        let numer = (b - a) * (b - a) * (g_b - g_c) - (b - c) * (b - c) * (g_b - g_a);
        let next = if denom.abs() > 1.0e-30 {
            b - 0.5 * numer / denom
        } else {
            0.5 * (a + c)
        };
        let next = next.clamp(a + 1.0e-6, c - 1.0e-6);
        let g_next = g(next)?;
        if (next - b).abs() < 1.0 / 86_400.0 {
            return Ok(next);
        }
        if next < b {
            if g_next < g_b {
                c = b;
                g_c = g_b;
                b = next;
                g_b = g_next;
            } else {
                a = next;
                g_a = g_next;
            }
        } else if g_next < g_b {
            a = b;
            g_a = g_b;
            b = next;
            g_b = g_next;
        } else {
            c = next;
            g_c = g_next;
        }
    }
    Ok(b)
}
