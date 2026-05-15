//! Shared angle helpers used across the astrology layer.
//!
//! Pulled out so each forecasting module (aspects, returns, transits,
//! synastry, composite, solar arc, lunar phase, eclipses) can use the
//! same wrap/delta math without each defining its own private copy.

const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

/// Signed angular delta `a − b` in radians, normalised to `[-π, π]`.
#[inline]
pub(crate) fn signed_delta_rad(a: f64, b: f64) -> f64 {
    let mut d = a - b;
    while d > PI {
        d -= TAU;
    }
    while d < -PI {
        d += TAU;
    }
    d
}

/// Signed angular delta `a − b` in degrees, normalised to `[-180°, 180°]`.
#[inline]
pub(crate) fn signed_delta_deg(a_deg: f64, b_deg: f64) -> f64 {
    let mut d = a_deg - b_deg;
    while d > 180.0 {
        d -= 360.0;
    }
    while d < -180.0 {
        d += 360.0;
    }
    d
}

/// Unsigned angular distance in `[0°, 180°]`.
#[inline]
pub(crate) fn unsigned_arc_deg(a_deg: f64, b_deg: f64) -> f64 {
    let mut d = (a_deg - b_deg).rem_euclid(360.0);
    if d > 180.0 {
        d = 360.0 - d;
    }
    d
}

/// Wrap an angle (radians) into `[0, 2π)`.
#[inline]
pub(crate) fn wrap_two_pi(x: f64) -> f64 {
    let v = x.rem_euclid(TAU);
    if v < 0.0 {
        v + TAU
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_delta_deg_wraps_to_shorter_arc() {
        assert!((signed_delta_deg(350.0, 10.0) + 20.0).abs() < 1e-12);
        assert!((signed_delta_deg(10.0, 350.0) - 20.0).abs() < 1e-12);
    }

    #[test]
    fn signed_delta_rad_matches_deg_form() {
        let a = 350.0_f64.to_radians();
        let b = 10.0_f64.to_radians();
        let d = signed_delta_rad(a, b);
        assert!((d + 20.0_f64.to_radians()).abs() < 1e-12);
    }

    #[test]
    fn unsigned_arc_deg_picks_shorter_distance() {
        assert!((unsigned_arc_deg(350.0, 10.0) - 20.0).abs() < 1e-12);
        assert!((unsigned_arc_deg(10.0, 350.0) - 20.0).abs() < 1e-12);
        assert!((unsigned_arc_deg(0.0, 180.0) - 180.0).abs() < 1e-12);
    }

    #[test]
    fn wrap_two_pi_normalises() {
        assert!((wrap_two_pi(0.0) - 0.0).abs() < 1e-12);
        assert!((wrap_two_pi(-PI) - PI).abs() < 1e-12);
        assert!((wrap_two_pi(3.0 * TAU) - 0.0).abs() < 1e-12);
    }
}
