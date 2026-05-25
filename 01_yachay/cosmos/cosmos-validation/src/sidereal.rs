//! Sidereal-mode infrastructure (Phase 3, step 1).
//!
//! Given a tropical (true equator and equinox of date) Cartesian
//! position, this module converts it to:
//!   * **Apparent ecliptic longitude/latitude of date** (rotate the
//!     equatorial-of-date vector by the IAU 2006 mean obliquity
//!     `epsa(t)` about the x-axis, then take spherical coordinates).
//!   * **Sidereal longitude** under a chosen ayanamsha (Lahiri only,
//!     for now) = `tropical_longitude - ayanamsha`.
//!
//! The ayanamsha implementation uses the IAU 2006 general precession in
//! longitude `pA(T)` and is anchored at J2000.0 to Swiss Ephemeris'
//! reference value `23°51'11.6"`. Residual vs Swiss across 1900-2100 is
//! tens of arcseconds (Swiss applies additional small corrections for
//! Spica's mean longitude that we don't model here). Phase 5 of the v1
//! roadmap is the natural home for tightening this.

use cosmos_core::Vector3;
use cosmos_time::{NutationCalculator, TT};

/// IAU 2006 mean obliquity epsa(t), in radians. Same series as
/// `eternal-core::precession::PrecessionIAU2006::obliquity_from_t`,
/// reproduced here as a free function so we don't depend on a private
/// method.
pub fn mean_obliquity_iau2006(t_centuries: f64) -> f64 {
    const ARCSEC_TO_RAD: f64 = std::f64::consts::PI / (180.0 * 3600.0);
    let t = t_centuries;
    let arcsec = 84381.406
        + t * (-46.836769
            + t * (-0.0001831 + t * (0.002_003_40 + t * (-0.000_000_576 + t * -0.000_000_043_4))));
    arcsec * ARCSEC_TO_RAD
}

/// IAU 2006 general precession in longitude `pA(T)`, in arcseconds.
/// Series from Capitaine, Wallace, Chapront (2003) IAU 2006 (Eq. 42).
fn precession_pa_arcsec(t_centuries: f64) -> f64 {
    let t = t_centuries;
    // pA = 5028.796195 T + 1.1054348 T² - 0.000041938 T³ - 0.0000533 T⁴
    //      + 0.000000311 T⁵
    t * (5028.796_195
        + t * (1.105_434_8
            + t * (-0.000_041_938 + t * (-0.000_053_3 + t * 0.000_000_311))))
}

/// Selectable ayanamsha mode. Each variant differs from the others only
/// by a constant offset at J2000.0; the time-evolution is identical
/// (the IAU 2006 general precession in longitude `pA(T)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ayanamsha {
    Lahiri,
    FaganBradley,
    DeLuce,
    Raman,
    Ushashashi,
    Krishnamurti,
    DjwhalKhul,
    Yukteshwar,
}

impl Ayanamsha {
    /// J2000.0 anchor value, in degrees. Each value matches Swiss
    /// Ephemeris `swe.get_ayanamsa_ex_ut(2451545.0, FLG_SWIEPH)` to ten
    /// decimals.
    pub fn j2000_anchor_deg(self) -> f64 {
        match self {
            Ayanamsha::Lahiri => 23.853_222_486_0,
            Ayanamsha::FaganBradley => 24.736_430_126_8,
            Ayanamsha::DeLuce => 27.811_882_913_7,
            Ayanamsha::Raman => 22.406_921_172_6,
            Ayanamsha::Ushashashi => 20.053_671_160_1,
            Ayanamsha::Krishnamurti => 23.756_370_172_6,
            Ayanamsha::DjwhalKhul => 28.355_808_760_1,
            Ayanamsha::Yukteshwar => 22.474_933_160_1,
        }
    }
}

/// Compute the ayanamsha for the given mode at the given TT, in radians.
/// The time-evolution uses IAU 2006 general precession `pA(T)`; the
/// J2000 anchor differs per mode.
pub fn ayanamsha(mode: Ayanamsha, tt: &TT) -> f64 {
    const ARCSEC_TO_RAD: f64 = std::f64::consts::PI / (180.0 * 3600.0);
    const DEG_TO_RAD: f64 = std::f64::consts::PI / 180.0;

    let jd = tt.to_julian_date();
    let t = ((jd.jd1() - 2_451_545.0) + jd.jd2()) / 36_525.0;
    let delta_arcsec = precession_pa_arcsec(t);
    mode.j2000_anchor_deg() * DEG_TO_RAD + delta_arcsec * ARCSEC_TO_RAD
}

/// Convenience wrapper for the most common ayanamsha (Lahiri).
pub fn lahiri_ayanamsha(tt: &TT) -> f64 {
    ayanamsha(Ayanamsha::Lahiri, tt)
}

/// IAU 2006/2000A **true obliquity** of date: mean obliquity + nutation
/// in obliquity Δε. This is the rotation angle that takes the true
/// equator-of-date plane into the true ecliptic-of-date plane.
///
/// Returns `Err` only if the underlying nutation series returns an error
/// (out-of-range epoch in some eternal-time implementations).
pub fn true_obliquity_iau2006a(tt: &TT) -> Result<f64, String> {
    let jd = tt.to_julian_date();
    let t = ((jd.jd1() - 2_451_545.0) + jd.jd2()) / 36_525.0;
    let nut = tt
        .nutation_iau2006a()
        .map_err(|e| format!("nutation failed: {:?}", e))?;
    Ok(mean_obliquity_iau2006(t) + nut.nutation_obliquity())
}

/// Convert a Cartesian vector in **true equator and equinox of date**
/// (the TET frame our apparent-observer pipeline outputs) to the
/// **apparent ecliptic of date** Cartesian, by rotating about the
/// x-axis by the **true obliquity** (mean + Δε) at the same epoch.
pub fn tet_equatorial_to_ecliptic_of_date(v_tet: Vector3, tt: &TT) -> Vector3 {
    let eps_true = true_obliquity_iau2006a(tt).unwrap_or_else(|_| {
        // Fall back to mean obliquity if nutation evaluation fails. The
        // arc-second cost is acceptable as a worst case.
        let jd = tt.to_julian_date();
        let t = ((jd.jd1() - 2_451_545.0) + jd.jd2()) / 36_525.0;
        mean_obliquity_iau2006(t)
    });
    let (sin_e, cos_e) = libm::sincos(eps_true);
    Vector3::new(
        v_tet.x,
        v_tet.y * cos_e + v_tet.z * sin_e,
        -v_tet.y * sin_e + v_tet.z * cos_e,
    )
}

/// Decompose an ecliptic-of-date Cartesian into (longitude, latitude)
/// in radians.
pub fn ecliptic_lon_lat(v_ecl: Vector3) -> (f64, f64) {
    let lon = libm::atan2(v_ecl.y, v_ecl.x);
    let r_xy = libm::sqrt(v_ecl.x * v_ecl.x + v_ecl.y * v_ecl.y);
    let lat = libm::atan2(v_ecl.z, r_xy);
    let lon = if lon < 0.0 { lon + std::f64::consts::TAU } else { lon };
    (lon, lat)
}

/// Compute the sidereal ecliptic longitude (in radians) of a TET-frame
/// apparent Cartesian, under the requested ayanamsha mode.
pub fn sidereal_longitude(mode: Ayanamsha, v_tet: Vector3, tt: &TT) -> f64 {
    let v_ecl = tet_equatorial_to_ecliptic_of_date(v_tet, tt);
    let (lon_tropical, _) = ecliptic_lon_lat(v_ecl);
    let lon_sidereal = lon_tropical - ayanamsha(mode, tt);
    let two_pi = std::f64::consts::TAU;
    let lon_sidereal = lon_sidereal % two_pi;
    if lon_sidereal < 0.0 {
        lon_sidereal + two_pi
    } else {
        lon_sidereal
    }
}

/// Convenience wrapper for the most common sidereal longitude (Lahiri).
pub fn lahiri_sidereal_longitude(v_tet: Vector3, tt: &TT) -> f64 {
    sidereal_longitude(Ayanamsha::Lahiri, v_tet, tt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmos_time::julian::JulianDate;

    fn tt_from_jd(jd: f64) -> TT {
        TT::from_julian_date(JulianDate::new(jd, 0.0))
    }

    #[test]
    fn lahiri_at_j2000_matches_anchor() {
        let tt = tt_from_jd(2_451_545.0);
        let ay = lahiri_ayanamsha(&tt);
        // Swiss reference at J2000 to 10 decimals.
        let expected = 23.853_222_486_0_f64.to_radians();
        assert!(
            (ay - expected).abs() < 1e-12,
            "got {} rad, expected {} rad",
            ay,
            expected
        );
    }

    #[test]
    fn lahiri_increases_with_time() {
        let ay_1900 = lahiri_ayanamsha(&tt_from_jd(2_415_020.5));
        let ay_2000 = lahiri_ayanamsha(&tt_from_jd(2_451_545.0));
        let ay_2100 = lahiri_ayanamsha(&tt_from_jd(2_488_069.5));
        assert!(ay_1900 < ay_2000);
        assert!(ay_2000 < ay_2100);
    }

    #[test]
    fn obliquity_at_j2000_is_canonical() {
        let eps = mean_obliquity_iau2006(0.0);
        // IAU 2006: 84381.406" = 23.4392911° at J2000.
        let expected = (84381.406_f64 / 3600.0).to_radians();
        assert!((eps - expected).abs() < 1e-14);
    }
}
