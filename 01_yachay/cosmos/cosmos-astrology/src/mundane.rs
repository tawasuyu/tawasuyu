//! Mundane (Placidus-quadrant) helpers.
//!
//! These functions answer the **third-dimensional** questions about a
//! body that the ecliptic projection erases:
//!
//! * **Ascensional Difference (AD)** — how much earlier or later a body
//!   crosses the horizon compared to a hypothetical body on the celestial
//!   equator. Formula: `sin AD = tan δ · tan φ`.
//! * **Diurnal / Nocturnal Semi-Arc** — the equatorial-degree distance
//!   the body travels between horizon and meridian. `DSA = 90° + AD`,
//!   `NSA = 90° − AD`.
//! * **Hour Angle (H)** — the equatorial angle between the body and the
//!   local meridian. `H = RAMC − RA(body)`, normalised to `[-π, π]`.
//! * **Mundane Position (m)** — a continuous coordinate in `[0, 4)` that
//!   wraps the Placidus quadrant structure:
//!     * `m = 0`: rising point (eastern horizon)
//!     * `m = 1`: upper meridian (MC)
//!     * `m = 2`: setting point (western horizon)
//!     * `m = 3`: lower meridian (IC)
//!
//!   House cusps land at the natural `m = k/3` boundaries (cusp 11 at
//!   `m = 2/3`, cusp 12 at `m = 1/3`, etc., in the Placidus model).
//!
//! Latitudes near the polar circle (|φ| ≥ 90° − |δ|) make AD undefined
//! — the body never sets or never rises. The helpers return `f64::NAN`
//! in that regime instead of panicking; the primary-direction layer
//! handles the NaN by surfacing an `HouseSystemUnavailable`-style error.

use std::f64::consts::{PI, TAU};

/// Wrap an angle into `[-π, π]`.
#[inline]
fn wrap_pi(x: f64) -> f64 {
    let mut v = x.rem_euclid(TAU);
    if v > PI {
        v -= TAU;
    }
    v
}

/// Ascensional Difference (AD), radians. `sin AD = tan δ · tan φ`.
/// Returns `NAN` when the body never crosses the horizon (always above
/// or always below at the observer's latitude).
pub fn ascensional_difference_rad(declination_rad: f64, latitude_rad: f64) -> f64 {
    let s = libm::tan(declination_rad) * libm::tan(latitude_rad);
    if !(-1.0..=1.0).contains(&s) {
        f64::NAN
    } else {
        libm::asin(s)
    }
}

/// Diurnal Semi-Arc (DSA), radians. Time from rising to upper meridian
/// expressed as an equatorial angle. `DSA = π/2 + AD`.
pub fn diurnal_semi_arc_rad(declination_rad: f64, latitude_rad: f64) -> f64 {
    let ad = ascensional_difference_rad(declination_rad, latitude_rad);
    if ad.is_nan() {
        f64::NAN
    } else {
        std::f64::consts::FRAC_PI_2 + ad
    }
}

/// Nocturnal Semi-Arc (NSA), radians. `NSA = π/2 − AD`.
pub fn nocturnal_semi_arc_rad(declination_rad: f64, latitude_rad: f64) -> f64 {
    let ad = ascensional_difference_rad(declination_rad, latitude_rad);
    if ad.is_nan() {
        f64::NAN
    } else {
        std::f64::consts::FRAC_PI_2 - ad
    }
}

/// Signed hour angle `H = RAMC − RA`, normalised to `[-π, π]`. Negative
/// values are east of the meridian (pre-culmination), positive values
/// west (post-culmination).
pub fn signed_hour_angle_rad(ramc_rad: f64, right_ascension_rad: f64) -> f64 {
    wrap_pi(ramc_rad - right_ascension_rad)
}

/// Returns `true` if the body sits above the local horizon at the
/// given natal time. The criterion is `|H| ≤ DSA`.
pub fn is_above_horizon(hour_angle_rad: f64, diurnal_semi_arc_rad: f64) -> bool {
    if diurnal_semi_arc_rad.is_nan() {
        // Pole / circumpolar case: handle by examining the sign of
        // `tan δ · tan φ`. Skip for now and assume below.
        return false;
    }
    hour_angle_rad.abs() <= diurnal_semi_arc_rad
}

/// Compute the continuous Placidus mundane position `m ∈ [0, 4)` given
/// the body's signed hour angle and its DSA + NSA.
///
/// Boundary mapping:
///   `m = 0`  → rising (east horizon, `H = -DSA`)
///   `m = 1`  → MC      (`H = 0`)
///   `m = 2`  → setting (west horizon, `H = +DSA`)
///   `m = 3`  → IC      (`H = ±π`)
///
/// Within each quadrant the mapping is linear in hour angle.
pub fn mundane_position(
    hour_angle_rad: f64,
    diurnal_semi_arc_rad: f64,
    nocturnal_semi_arc_rad: f64,
) -> f64 {
    let h = wrap_pi(hour_angle_rad);
    let dsa = diurnal_semi_arc_rad;
    let nsa = nocturnal_semi_arc_rad;
    if dsa.is_nan() || nsa.is_nan() {
        return f64::NAN;
    }
    if h.abs() <= dsa {
        // Above horizon: m ∈ [0, 2], m = 1 + H/DSA.
        return 1.0 + h / dsa;
    }
    // Below horizon.
    if h > dsa {
        // West side: m ∈ (2, 3], H = DSA + (m - 2) · NSA → m = 2 + (H - DSA)/NSA.
        2.0 + (h - dsa) / nsa
    } else {
        // East side: m ∈ (3, 4), H = -π + (m - 3) · NSA → m = 3 + (H + π)/NSA.
        3.0 + (h + PI) / nsa
    }
}

/// Inverse of `mundane_position`: given an `m` and the body's
/// `(DSA, NSA)`, return the hour angle the body would have at that
/// mundane position. Used by the primary-direction code to compute the
/// arc the promissor must rotate to *reach* a target mundane position.
pub fn hour_angle_for_mundane(
    m: f64,
    diurnal_semi_arc_rad: f64,
    nocturnal_semi_arc_rad: f64,
) -> f64 {
    let dsa = diurnal_semi_arc_rad;
    let nsa = nocturnal_semi_arc_rad;
    let m = m.rem_euclid(4.0);
    if m <= 2.0 {
        // Above horizon.
        (m - 1.0) * dsa
    } else if m <= 3.0 {
        dsa + (m - 2.0) * nsa
    } else {
        -PI + (m - 3.0) * nsa
    }
}

/// Wrap `m` into `[0, 4)`.
#[inline]
pub fn wrap_mundane(m: f64) -> f64 {
    let v = m.rem_euclid(4.0);
    if v < 0.0 {
        v + 4.0
    } else {
        v
    }
}

/// Compute the natal mundane position of a body, given the chart's
/// RAMC and the body's RA/Dec, with the observer's latitude. This is
/// the one-stop helper the primary-direction layer calls.
pub fn natal_mundane_position(
    ramc_rad: f64,
    body_right_ascension_rad: f64,
    body_declination_rad: f64,
    latitude_rad: f64,
) -> f64 {
    let dsa = diurnal_semi_arc_rad(body_declination_rad, latitude_rad);
    let nsa = nocturnal_semi_arc_rad(body_declination_rad, latitude_rad);
    let h = signed_hour_angle_rad(ramc_rad, body_right_ascension_rad);
    mundane_position(h, dsa, nsa)
}

/// Convenience: wrap a difference in mundane coordinates so it lies in
/// `[0, 4)` (mod 4 — equivalent to mod 360° rotation).
pub fn wrap_mundane_diff(diff: f64) -> f64 {
    wrap_mundane(diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ad_zero_for_equator_body_or_zero_latitude() {
        // δ = 0 → AD = 0 regardless of φ.
        for phi_deg in [-60.0, -10.0, 0.0, 25.0, 60.0] {
            let phi = (phi_deg as f64).to_radians();
            let ad = ascensional_difference_rad(0.0, phi);
            assert!(ad.abs() < 1e-12, "AD(δ=0, φ={}) = {}", phi_deg, ad);
        }
        // φ = 0 → AD = 0 regardless of δ.
        for dec_deg in [-23.0, -5.0, 0.0, 5.0, 23.0] {
            let dec = (dec_deg as f64).to_radians();
            let ad = ascensional_difference_rad(dec, 0.0);
            assert!(ad.abs() < 1e-12, "AD(δ={}, φ=0) = {}", dec_deg, ad);
        }
    }

    #[test]
    fn dsa_plus_nsa_equals_pi() {
        let dec = 15.0_f64.to_radians();
        let phi = 40.0_f64.to_radians();
        let s = diurnal_semi_arc_rad(dec, phi) + nocturnal_semi_arc_rad(dec, phi);
        assert!((s - PI).abs() < 1e-12);
    }

    #[test]
    fn hour_angle_wraps_correctly() {
        // RAMC = 350°, RA = 10°: simple diff would be 340°, wrapped → -20°.
        let h = signed_hour_angle_rad(350.0_f64.to_radians(), 10.0_f64.to_radians());
        assert!((h.to_degrees() - (-20.0)).abs() < 1e-9);
    }

    #[test]
    fn mundane_position_at_meridian_is_one() {
        // Body on meridian → H = 0 → m = 1.
        let dec = 10.0_f64.to_radians();
        let phi = 30.0_f64.to_radians();
        let dsa = diurnal_semi_arc_rad(dec, phi);
        let nsa = nocturnal_semi_arc_rad(dec, phi);
        let m = mundane_position(0.0, dsa, nsa);
        assert!((m - 1.0).abs() < 1e-12);
    }

    #[test]
    fn mundane_position_at_horizon_is_zero_or_two() {
        let dec = 10.0_f64.to_radians();
        let phi = 30.0_f64.to_radians();
        let dsa = diurnal_semi_arc_rad(dec, phi);
        let nsa = nocturnal_semi_arc_rad(dec, phi);
        // East horizon: H = -DSA → m = 0.
        let m_east = mundane_position(-dsa, dsa, nsa);
        assert!(m_east.abs() < 1e-12, "east horizon m = {}", m_east);
        // West horizon: H = +DSA → m = 2.
        let m_west = mundane_position(dsa, dsa, nsa);
        assert!((m_west - 2.0).abs() < 1e-12);
    }

    #[test]
    fn mundane_position_roundtrip() {
        let dec = (-12.0_f64).to_radians();
        let phi = 45.0_f64.to_radians();
        let dsa = diurnal_semi_arc_rad(dec, phi);
        let nsa = nocturnal_semi_arc_rad(dec, phi);
        for h_deg in [-100.0_f64, -60.0, -10.0, 0.0, 30.0, 80.0, 130.0, 175.0] {
            let h = h_deg.to_radians();
            let m = mundane_position(h, dsa, nsa);
            let h_back = hour_angle_for_mundane(m, dsa, nsa);
            // Allow ±π/360 (= 0.5°) for boundary cases.
            let diff = wrap_pi(h_back - h).abs();
            assert!(diff < 1e-9, "round-trip failed at H={}° m={} diff={}", h_deg, m, diff);
        }
    }
}
