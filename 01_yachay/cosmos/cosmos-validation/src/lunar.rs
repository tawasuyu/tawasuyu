//! Lunar nodes and Lilith (Phase 3, step 3).
//!
//! Today only the *mean* node and *mean* lunar apogee (Lilith) are
//! exposed. The true (osculating) node and true Lilith are follow-ups
//! that need either an osculating-orbit fit to DE441 or Swiss's
//! moon-perigee data. Both modes are heavily used by astrologers, so
//! the public API names use the explicit `mean_` prefix to leave room
//! for the true variants.

use cosmos_core::nutation::IERS2010FundamentalArgs;
use cosmos_core::Vector3;
use cosmos_ephemeris::jpl::SpkFile;
use cosmos_time::{NutationCalculator, TT};

use crate::oracle::OracleError;
use crate::sidereal::{ecliptic_lon_lat, tet_equatorial_to_ecliptic_of_date};

/// Mean inclination of the lunar orbit to the ecliptic, in degrees.
/// Used by the apogee→ecliptic projection. Matches Swiss `MOON_MEAN_INCL`.
const MOON_MEAN_INCLINATION_DEG: f64 = 5.145_396;

const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

fn t_centuries(tt: &TT) -> f64 {
    let jd = tt.to_julian_date();
    ((jd.jd1() - 2_451_545.0) + jd.jd2()) / 36_525.0
}

fn wrap_two_pi(x: f64) -> f64 {
    let mut y = x.rem_euclid(TAU);
    if y >= TAU {
        y -= TAU;
    }
    y
}

/// Mean longitude of the **ascending** lunar node Ω, in radians,
/// referred to the **true ecliptic of date** (i.e. mean dynamics + the
/// nutation-in-longitude offset Δψ added so the value matches the
/// frame Swiss returns from `swe.calc(SE_MEAN_NODE)`).
///
/// Direction: the mean node MOVES RETROGRADE (westward) at about
/// 19.341° per Julian year, completing one cycle in ~18.61 years.
///
/// To recover the *truly* mean value (referred to the mean ecliptic
/// of date — what most textbooks call Ω̄), use [`mean_lunar_node_no_nutation`].
pub fn mean_lunar_node(tt: &TT) -> f64 {
    let bare = mean_lunar_node_no_nutation(tt);
    let dpsi = nutation_longitude(tt);
    wrap_two_pi(bare + dpsi)
}

/// Mean longitude of the ascending lunar node referred to the **mean
/// ecliptic of date** (no nutation). Pure IAU 2000A polynomial.
pub fn mean_lunar_node_no_nutation(tt: &TT) -> f64 {
    let t = t_centuries(tt);
    wrap_two_pi(t.moon_ascending_node_longitude())
}

/// Mean longitude of the **descending** lunar node = mean node + 180°.
pub fn mean_lunar_node_descending(tt: &TT) -> f64 {
    wrap_two_pi(mean_lunar_node(tt) + PI)
}

/// Mean longitude of the lunar perigee Γ, referred to the mean ecliptic
/// and equinox of date. Series from Brown's lunar theory / ELP-style
/// polynomial (Meeus 47.7):
///
///   Γ = 83.3532465° + 4069.0137287° T − 0.01032° T² − T³/80053
///
/// (all in degrees, T in Julian centuries from J2000 TT).
pub fn mean_lunar_perigee(tt: &TT) -> f64 {
    let t = t_centuries(tt);
    let deg = 83.353_246_5
        + t * (4069.013_728_7 + t * (-0.010_32 + t * (-1.0 / 80_053.0 + t * 0.0)));
    wrap_two_pi(deg.to_radians())
}

/// Mean longitude of the lunar apogee — the "mean Black Moon Lilith"
/// of astrology. Computed exactly the way Swiss does it (without the
/// per-century correction tables, which are zero across 0–3000 AD):
///
///   1. Compute mean apogee in the **orbital plane** as
///      `(SWELP − MP) + 180°` where SWELP is the Moon's mean longitude
///      and MP its mean anomaly. Practically: `mean_perigee + 180°`.
///   2. Subtract the mean ascending node longitude (no nutation).
///   3. Project from the inclined orbital plane to the ecliptic by
///      rotating the (lon, 0, 1) Cartesian by `−5.145396°` around X.
///   4. Add the node longitude back.
///   5. Add the nutation-in-longitude offset Δψ to land in the same
///      true-ecliptic-of-date frame Swiss reports.
///
/// Returns radians. Validated against Swiss `SE_MEAN_APOG` to a few
/// arcseconds across 1900–2100; the residual is the per-century apsis
/// correction (zeros in modern era for Swiss, omitted here too).
pub fn mean_lilith(tt: &TT) -> f64 {
    let perigee_orbital = mean_lunar_perigee(tt);
    let apogee_orbital = wrap_two_pi(perigee_orbital + PI);
    let node = mean_lunar_node_no_nutation(tt);
    let from_node = wrap_two_pi(apogee_orbital - node);

    // Rotate (cos λ, sin λ, 0) by −inclination around X axis, then take
    // the new longitude.
    let (sin_l, cos_l) = libm::sincos(from_node);
    let inc = MOON_MEAN_INCLINATION_DEG.to_radians();
    let (sin_i, cos_i) = libm::sincos(inc);
    // Rotation by −i around X: y' = y cos i + z sin i, z' = −y sin i + z cos i
    // With z = 0 initially: y' = y cos i, z' = −y sin i.
    let new_x = cos_l;
    let new_y = sin_l * cos_i;
    let lon_proj = libm::atan2(new_y, new_x);
    let lilith = wrap_two_pi(lon_proj + node + nutation_longitude(tt));
    lilith
}

/// Helper: nutation in longitude Δψ at the given epoch (radians).
fn nutation_longitude(tt: &TT) -> f64 {
    tt.nutation_iau2006a()
        .map(|n| n.nutation_longitude())
        .unwrap_or(0.0)
}

/// Constant μ = G(M_earth + M_moon) in km³/s², used by the osculating
/// Keplerian element extraction.
const MU_EARTH_MOON_KM3_S2: f64 =
    cosmos_core::constants::GM_EARTH_KM3S2 + cosmos_core::constants::GM_MOON_KM3S2;

/// Compute the Moon's geocentric state in the **ecliptic-of-date frame**
/// from a DE-class SPK kernel, in km and km/s. Internally uses the
/// `(301 wrt 3) − (399 wrt 3)` chain to avoid the SSB hop, then rotates
/// ICRF → TET via the IAU 2006/2000A NPB matrix and TET equatorial →
/// ecliptic-of-date by the true obliquity.
fn moon_state_ecl_of_date(
    spk: &SpkFile,
    tt: &TT,
    jd_tdb: f64,
) -> Result<(Vector3, Vector3), OracleError> {
    use cosmos_core::utils::jd_to_centuries;
    use cosmos_time::NutationCalculator;

    let (moon_emb_pos, moon_emb_vel) = spk
        .compute_state(301, 3, jd_tdb)
        .map_err(OracleError::from)?;
    let (earth_emb_pos, earth_emb_vel) = spk
        .compute_state(399, 3, jd_tdb)
        .map_err(OracleError::from)?;

    let pos_icrf = Vector3::new(
        moon_emb_pos[0] - earth_emb_pos[0],
        moon_emb_pos[1] - earth_emb_pos[1],
        moon_emb_pos[2] - earth_emb_pos[2],
    );
    let vel_icrf = Vector3::new(
        moon_emb_vel[0] - earth_emb_vel[0],
        moon_emb_vel[1] - earth_emb_vel[1],
        moon_emb_vel[2] - earth_emb_vel[2],
    );

    // Rotate ICRF → TET via NPB.
    let nut = tt
        .nutation_iau2006a()
        .map_err(|e| OracleError::Inner(format!("nutation: {:?}", e)))?;
    let tt_jd = tt.to_julian_date();
    let t_centuries = jd_to_centuries(tt_jd.jd1(), tt_jd.jd2());
    let npb = cosmos_core::precession::PrecessionIAU2006::new().npb_matrix_iau2006a(
        t_centuries,
        nut.nutation_longitude(),
        nut.nutation_obliquity(),
    );
    let pos_tet = npb * pos_icrf;
    let vel_tet = npb * vel_icrf;

    // TET equatorial → ecliptic of date (rotate around X by true obliquity).
    let pos_ecl = tet_equatorial_to_ecliptic_of_date(pos_tet, tt);
    let vel_ecl = tet_equatorial_to_ecliptic_of_date(vel_tet, tt);

    Ok((pos_ecl, vel_ecl))
}

/// **True (osculating) lunar ascending node**, ecliptic longitude in
/// radians, referred to the true ecliptic of date. Computed from the
/// Moon's instantaneous geocentric position + velocity:
///
///   h = r × v        (specific angular momentum)
///   Ω = atan2(h_x, −h_y)
///
/// Validated against Swiss `SE_TRUE_NODE` (see `lunar-check` CLI). The
/// osculating node oscillates around the mean by up to ±1.5° with a
/// period of 173 days (the Moon's draconic-vs-sidereal-month
/// interaction).
pub fn true_lunar_node_geocentric(spk: &SpkFile, tt: &TT, jd_tdb: f64) -> Result<f64, OracleError> {
    let (r, v) = moon_state_ecl_of_date(spk, tt, jd_tdb)?;
    let h = cross(r, v);
    Ok(wrap_two_pi(libm::atan2(h.x, -h.y)))
}

/// **True (osculating) Lilith** — ecliptic longitude of the empty
/// focus of the Moon's instantaneous Keplerian ellipse, in radians.
/// Uses the eccentricity vector
///
///   e_vec = (v × h) / μ − r̂
///
/// with μ = G(M_earth + M_moon). The empty focus is at the direction
/// opposite to perihelion → `lilith = atan2(e_y, e_x) + π`.
///
/// Validated against Swiss `SE_OSCU_APOG`.
pub fn true_lilith_geocentric(spk: &SpkFile, tt: &TT, jd_tdb: f64) -> Result<f64, OracleError> {
    let (r, v) = moon_state_ecl_of_date(spk, tt, jd_tdb)?;
    let h = cross(r, v);
    let v_cross_h = cross(v, h);
    let r_mag = libm::sqrt(r.x * r.x + r.y * r.y + r.z * r.z);
    let e_vec = Vector3::new(
        v_cross_h.x / MU_EARTH_MOON_KM3_S2 - r.x / r_mag,
        v_cross_h.y / MU_EARTH_MOON_KM3_S2 - r.y / r_mag,
        v_cross_h.z / MU_EARTH_MOON_KM3_S2 - r.z / r_mag,
    );
    let perigee_lon = libm::atan2(e_vec.y, e_vec.x);
    Ok(wrap_two_pi(perigee_lon + PI))
}

/// **True (osculating) lunar perigee** — opposite of true Lilith.
pub fn true_lunar_perigee_geocentric(
    spk: &SpkFile,
    tt: &TT,
    jd_tdb: f64,
) -> Result<f64, OracleError> {
    Ok(wrap_two_pi(true_lilith_geocentric(spk, tt, jd_tdb)? + PI))
}

#[inline]
fn cross(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

/// Project a TET-frame Cartesian onto its ecliptic-of-date longitude.
/// Re-exported so `lunar-check` can verify the geometry independently
/// of `lunar_check`-internal helpers.
pub fn ecliptic_lon_of_date(v_tet: Vector3, tt: &TT) -> f64 {
    let v_ecl = tet_equatorial_to_ecliptic_of_date(v_tet, tt);
    let (lon, _) = ecliptic_lon_lat(v_ecl);
    lon
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmos_time::julian::JulianDate;

    fn tt_from_jd(jd: f64) -> TT {
        TT::from_julian_date(JulianDate::new(jd, 0.0))
    }

    #[test]
    fn mean_node_at_j2000_matches_iau_constant() {
        // IAU 2000A polynomial value (no nutation) at J2000.0 = 125.04455501°.
        let node = mean_lunar_node_no_nutation(&tt_from_jd(2_451_545.0));
        let expected = 125.044_555_01_f64.to_radians();
        assert!(
            (node - expected).abs() < 1.0e-7,
            "got {} ({}°), expected {} ({}°)",
            node,
            node.to_degrees(),
            expected,
            expected.to_degrees()
        );
    }

    #[test]
    fn mean_node_with_nutation_matches_swiss_at_j2000() {
        // Swiss SE_MEAN_NODE at J2000.0 (TT) = 125.0406854°. Includes Δψ.
        let node = mean_lunar_node(&tt_from_jd(2_451_545.0));
        let expected = 125.040_685_4_f64.to_radians();
        assert!(
            (node - expected).abs() < 1.0e-6,
            "got {}°, expected {}°",
            node.to_degrees(),
            expected.to_degrees()
        );
    }

    #[test]
    fn mean_node_retrogrades() {
        let n_now = mean_lunar_node(&tt_from_jd(2_451_545.0));
        let n_later = mean_lunar_node(&tt_from_jd(2_451_545.0 + 365.25));
        // Mean node retrogrades by ~19.34° per year — so n_later < n_now
        // (modulo wrap). The shortest signed difference should be ~−19°.
        let diff = (n_later - n_now + 3.0 * PI).rem_euclid(TAU) - PI;
        let diff_deg = diff.to_degrees();
        assert!(
            (-21.0..-17.0).contains(&diff_deg),
            "expected ~−19°/yr retrograde, got {}°",
            diff_deg
        );
    }

    #[test]
    fn mean_lilith_at_j2000_matches_swiss() {
        // Swiss SE_MEAN_APOG at J2000.0 (TT) = 263.4642498°.
        let lilith = mean_lilith(&tt_from_jd(2_451_545.0));
        let expected = 263.464_249_8_f64.to_radians();
        assert!(
            (lilith - expected).abs() < 5.0e-6,
            "got {}°, expected {}°",
            lilith.to_degrees(),
            expected.to_degrees()
        );
    }
}
