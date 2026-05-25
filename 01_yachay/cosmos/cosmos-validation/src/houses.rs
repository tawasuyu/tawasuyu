//! House systems (Phase 3, step 2).
//!
//! Computes the Ascendant, the Midheaven, and the twelve house cusps
//! for a given moment + observer geographic location. v1 wires the two
//! systems whose formulas are closed-form:
//!
//!   * **Whole Sign** — Cusps are the 0° boundaries of the twelve
//!     zodiac signs counted from the sign that contains the Ascendant.
//!   * **Equal** — Cusps are the Ascendant + N×30° for N = 0..12.
//!
//! Placidus (and Koch, Regiomontanus, Campanus) are time-based systems
//! that need an iterative diurnal-arc solver. Their inclusion is a
//! follow-up in Phase 3 step 3; the input to those will be the same
//! `(last_rad, lat_rad, obliquity_rad)` triple this module already
//! consumes.
//!
//! All angles in / out of the public API are radians; the observer
//! latitude is geocentric for v1 (not geodetic), which costs ~10
//! arcsec near the poles. Topocentric work item moves to geodetic +
//! parallax.

use std::f64::consts::{PI, TAU};

/// Apparent ecliptic longitude of the Ascendant (the eastern horizon),
/// in radians, in the range `[0, 2π)`. Standard formula used by Swiss
/// Ephemeris and Meeus (Astronomical Algorithms ch. 14, eq. 14.4):
/// `λ_asc = atan2(-cos H, sin H · cos ε + tan φ · sin ε)` where `H` is
/// the Local Apparent Sidereal Time expressed as an angle. The two
/// `atan2` solutions are 180° apart; we pick the one east of the MC
/// so the Ascendant always sits above the horizon.
pub fn ascendant(last_rad: f64, lat_rad: f64, obliquity_rad: f64) -> f64 {
    let (sin_h, cos_h) = libm::sincos(last_rad);
    let (sin_e, cos_e) = libm::sincos(obliquity_rad);
    let tan_phi = libm::tan(lat_rad);
    let raw = libm::atan2(-cos_h, sin_h * cos_e + tan_phi * sin_e);
    let asc = wrap_two_pi(raw);

    // `atan2` returns one of two solutions 180° apart. The actual rising
    // point is the one east of the MC — i.e. `(Asc - MC) mod 360°` must
    // sit in `(0°, 180°)`. Flip by 180° otherwise.
    let mc = midheaven(last_rad, obliquity_rad);
    let diff = (asc - mc).rem_euclid(TAU);
    if diff > PI {
        wrap_two_pi(asc + PI)
    } else {
        asc
    }
}

/// Apparent ecliptic longitude of the Midheaven (the meridian-piercing
/// point on the ecliptic above the horizon):
/// `λ_mc = atan2(sin H, cos H · cos ε)`.
pub fn midheaven(last_rad: f64, obliquity_rad: f64) -> f64 {
    let (sin_h, cos_h) = libm::sincos(last_rad);
    let (_, cos_e) = libm::sincos(obliquity_rad);
    wrap_two_pi(libm::atan2(sin_h, cos_h * cos_e))
}

/// Twelve Whole-Sign house cusps, in radians, in chart order
/// (cusp[0] = house 1, cusp[1] = house 2, …, cusp[11] = house 12).
pub fn whole_sign_houses(asc_rad: f64) -> [f64; 12] {
    let asc_deg = asc_rad.to_degrees();
    let sign_index = (asc_deg / 30.0).floor() as i32;
    let mut out = [0.0; 12];
    for i in 0..12 {
        let deg = ((sign_index + i as i32) as f64) * 30.0;
        out[i] = wrap_two_pi(deg.to_radians());
    }
    out
}

/// Twelve Equal-house cusps. House 1 = Ascendant exactly; each
/// subsequent cusp is 30° later along the ecliptic.
pub fn equal_houses(asc_rad: f64) -> [f64; 12] {
    let mut out = [0.0; 12];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = wrap_two_pi(asc_rad + (i as f64) * (PI / 6.0));
    }
    out
}

/// Twelve Koch house cusps. Direct port of `swehouse.c case 'K'`.
/// Like Placidus, this fails inside the polar circle.
pub fn koch_houses(
    last_rad: f64,
    lat_rad: f64,
    obliquity_rad: f64,
) -> Result<[f64; 12], &'static str> {
    let armc = last_rad.to_degrees();
    let lat = lat_rad.to_degrees();
    let eps = obliquity_rad.to_degrees();

    if lat.abs() >= 90.0 - eps {
        return Err("Koch undefined inside the polar circle");
    }

    let sine = sind(eps);
    let cose = cosd(eps);
    let tanfi = tand(lat);

    let asc = ascendant(last_rad, lat_rad, obliquity_rad).to_degrees();
    let mc = midheaven(last_rad, obliquity_rad).to_degrees();

    let mut sina = sind(mc) * sine / cosd(lat);
    if sina > 1.0 {
        sina = 1.0;
    }
    if sina < -1.0 {
        sina = -1.0;
    }
    let cosa = libm::sqrt(1.0 - sina * sina);
    let c = atand(tanfi / cosa);
    let ad3 = asind(sind(c) * sina) / 3.0;

    let cusp_11 = asc1(armc + 30.0 - 2.0 * ad3, lat, sine, cose);
    let cusp_12 = asc1(armc + 60.0 - ad3, lat, sine, cose);
    let cusp_2 = asc1(armc + 120.0 + ad3, lat, sine, cose);
    let cusp_3 = asc1(armc + 150.0 + 2.0 * ad3, lat, sine, cose);

    Ok(quadrant_cusps(asc, mc, cusp_11, cusp_12, cusp_2, cusp_3))
}

/// Twelve Regiomontanus house cusps. Direct port of `swehouse.c case 'R'`.
pub fn regiomontanus_houses(last_rad: f64, lat_rad: f64, obliquity_rad: f64) -> [f64; 12] {
    let armc = last_rad.to_degrees();
    let lat = lat_rad.to_degrees();
    let eps = obliquity_rad.to_degrees();

    let sine = sind(eps);
    let cose = cosd(eps);
    let tanfi = tand(lat);

    let asc = ascendant(last_rad, lat_rad, obliquity_rad).to_degrees();
    let mc = midheaven(last_rad, obliquity_rad).to_degrees();

    let fh1 = atand(tanfi * 0.5);
    let fh2 = atand(tanfi * cosd(30.0));

    let cusp_11 = asc1(30.0 + armc, fh1, sine, cose);
    let cusp_12 = asc1(60.0 + armc, fh2, sine, cose);
    let cusp_2 = asc1(120.0 + armc, fh2, sine, cose);
    let cusp_3 = asc1(150.0 + armc, fh1, sine, cose);

    quadrant_cusps(asc, mc, cusp_11, cusp_12, cusp_2, cusp_3)
}

/// Twelve Campanus house cusps. Direct port of `swehouse.c case 'C'`.
/// Returns `Err` if cos(lat) is exactly zero (true poles).
pub fn campanus_houses(
    last_rad: f64,
    lat_rad: f64,
    obliquity_rad: f64,
) -> Result<[f64; 12], &'static str> {
    let armc = last_rad.to_degrees();
    let lat = lat_rad.to_degrees();
    let eps = obliquity_rad.to_degrees();

    let sine = sind(eps);
    let cose = cosd(eps);

    let asc = ascendant(last_rad, lat_rad, obliquity_rad).to_degrees();
    let mc = midheaven(last_rad, obliquity_rad).to_degrees();

    let fh1 = asind(sind(lat) / 2.0);
    let fh2 = asind(libm::sqrt(3.0) / 2.0 * sind(lat));
    let cosfi = cosd(lat);

    if cosfi == 0.0 {
        return Err("Campanus undefined exactly at the geographic pole");
    }

    let xh1 = atand(libm::sqrt(3.0) / cosfi);
    let xh2 = atand(1.0 / libm::sqrt(3.0) / cosfi);

    let cusp_11 = asc1(armc + 90.0 - xh1, fh1, sine, cose);
    let cusp_12 = asc1(armc + 90.0 - xh2, fh2, sine, cose);
    let cusp_2 = asc1(armc + 90.0 + xh2, fh2, sine, cose);
    let cusp_3 = asc1(armc + 90.0 + xh1, fh1, sine, cose);

    Ok(quadrant_cusps(asc, mc, cusp_11, cusp_12, cusp_2, cusp_3))
}

/// Twelve Porphyry house cusps. Trisects each meridian-to-horizon
/// quadrant. Always defined; falls back gracefully near the poles.
pub fn porphyry_houses(last_rad: f64, lat_rad: f64, obliquity_rad: f64) -> [f64; 12] {
    let asc = ascendant(last_rad, lat_rad, obliquity_rad).to_degrees();
    let mc = midheaven(last_rad, obliquity_rad).to_degrees();
    let mut acmc = difdeg2n(asc, mc);
    let asc_used = if acmc < 0.0 {
        // Within polar circle: ASC swaps to the other side.
        let asc2 = degnorm(asc + 180.0);
        acmc = difdeg2n(asc2, mc);
        asc2
    } else {
        asc
    };
    let cusp_11 = degnorm(mc + acmc / 3.0);
    let cusp_12 = degnorm(mc + acmc / 3.0 * 2.0);
    let cusp_2 = degnorm(asc_used + (180.0 - acmc) / 3.0);
    let cusp_3 = degnorm(asc_used + (180.0 - acmc) / 3.0 * 2.0);
    quadrant_cusps(asc_used, mc, cusp_11, cusp_12, cusp_2, cusp_3)
}

/// Build the 12-cusp array from the four angles plus the four
/// intermediate cusps (cusps 11, 12, 2, 3 in degrees). Cusps 5, 6, 8, 9
/// are derived by 180° opposition.
fn quadrant_cusps(
    asc_deg: f64,
    mc_deg: f64,
    cusp_11: f64,
    cusp_12: f64,
    cusp_2: f64,
    cusp_3: f64,
) -> [f64; 12] {
    let to_rad = |d: f64| degnorm(d).to_radians();
    [
        to_rad(asc_deg),
        to_rad(cusp_2),
        to_rad(cusp_3),
        to_rad(mc_deg + 180.0),
        to_rad(cusp_11 + 180.0),
        to_rad(cusp_12 + 180.0),
        to_rad(asc_deg + 180.0),
        to_rad(cusp_2 + 180.0),
        to_rad(cusp_3 + 180.0),
        to_rad(mc_deg),
        to_rad(cusp_11),
        to_rad(cusp_12),
    ]
}

/// Twelve Placidus house cusps. Direct port of the iterative algorithm
/// from Swiss Ephemeris `swehouse.c` (Aloistr / Astrodienst, AGPL-3),
/// adapted to idiomatic Rust. Cusps 1, 4, 7, 10 are Asc/IC/Desc/MC; the
/// intermediate cusps come from a 1/3- and 2/3-of-semi-arc fixed-point
/// iteration. Returns `Err` inside the polar circle (|φ| ≥ 90° − ε)
/// where the algorithm diverges; callers there typically fall back to
/// Porphyry or another time-independent system.
pub fn placidus_houses(
    last_rad: f64,
    lat_rad: f64,
    obliquity_rad: f64,
) -> Result<[f64; 12], &'static str> {
    let armc_deg = last_rad.to_degrees();
    let lat_deg = lat_rad.to_degrees();
    let eps_deg = obliquity_rad.to_degrees();

    if lat_deg.abs() >= 90.0 - eps_deg {
        return Err("Placidus undefined inside the polar circle");
    }

    let sine = sind(eps_deg);
    let cose = cosd(eps_deg);
    let tane = tand(eps_deg);
    let tanfi = tand(lat_deg);

    // Pole heights for the f₁ = a/3 and f₂ = 2a/3 decompositions.
    let a = asind(tanfi * tane);
    let fh1 = atand(sind(a / 3.0) / tane);
    let fh2 = atand(sind(a * 2.0 / 3.0) / tane);

    let asc_deg = asc_meeus_to_degnorm(armc_deg, lat_deg, sine, cose);
    let mc_deg = mc_meeus_to_degnorm(armc_deg, eps_deg);

    let cusp_11 = placidus_iter(armc_deg + 30.0, fh1, sine, cose, tanfi, 3.0)?;
    let cusp_12 = placidus_iter(armc_deg + 60.0, fh2, sine, cose, tanfi, 1.5)?;
    let cusp_2 = placidus_iter(armc_deg + 120.0, fh2, sine, cose, tanfi, 1.5)?;
    let cusp_3 = placidus_iter(armc_deg + 150.0, fh1, sine, cose, tanfi, 3.0)?;

    let to_rad = |d: f64| (degnorm(d)).to_radians();

    Ok([
        to_rad(asc_deg),
        to_rad(cusp_2),
        to_rad(cusp_3),
        to_rad(mc_deg + 180.0),
        to_rad(cusp_11 + 180.0),
        to_rad(cusp_12 + 180.0),
        to_rad(asc_deg + 180.0),
        to_rad(cusp_2 + 180.0),
        to_rad(cusp_3 + 180.0),
        to_rad(mc_deg),
        to_rad(cusp_11),
        to_rad(cusp_12),
    ])
}

/// Wrapper around `ascendant` that returns degrees-normalised value, to
/// keep the Swiss-faithful `placidus_houses` body in degree space.
fn asc_meeus_to_degnorm(armc_deg: f64, lat_deg: f64, _sine: f64, _cose: f64) -> f64 {
    degnorm(
        ascendant(
            armc_deg.to_radians(),
            lat_deg.to_radians(),
            // sine/cose already encode eps, but `ascendant` re-derives it
            // from the radians-form. We can pass anything consistent —
            // recompute from sine.
            libm::asin(_sine),
        )
        .to_degrees(),
    )
}

fn mc_meeus_to_degnorm(armc_deg: f64, eps_deg: f64) -> f64 {
    degnorm(midheaven(armc_deg.to_radians(), eps_deg.to_radians()).to_degrees())
}

/// Placidus inner loop, faithful to `swehouse.c`.
fn placidus_iter(
    rectasc_deg: f64,
    f0_deg: f64,
    sine: f64,
    cose: f64,
    tanfi: f64,
    divisor: f64,
) -> Result<f64, &'static str> {
    const VERY_SMALL: f64 = 1.0e-10;
    const VERY_SMALL_PLAC_ITER: f64 = 1.0e-12;
    const NITER_MAX: i32 = 100;

    let rectasc = degnorm(rectasc_deg);
    // Initial declination estimate from the f₀-pole-height ascendant.
    let mut tant = tand(asind(sine * sind(asc1(rectasc, f0_deg, sine, cose))));
    if tant.abs() < VERY_SMALL {
        return Ok(rectasc);
    }
    let mut f = atand(sind(asind(tanfi * tant) / divisor) / tant);
    let mut cusp = asc1(rectasc, f, sine, cose);
    let mut cuspsv = 0.0;
    for i in 1..=NITER_MAX {
        tant = tand(asind(sine * sind(cusp)));
        if tant.abs() < VERY_SMALL {
            return Ok(rectasc);
        }
        f = atand(sind(asind(tanfi * tant) / divisor) / tant);
        cusp = asc1(rectasc, f, sine, cose);
        if i > 1 && difdeg2n(cusp, cuspsv).abs() < VERY_SMALL_PLAC_ITER {
            return Ok(cusp);
        }
        cuspsv = cusp;
    }
    Err("Placidus iteration did not converge")
}

/// Swiss Asc1: returns the ecliptic longitude of the great circle
/// (defined by pole height `f`) intersected with the ecliptic, given
/// the equatorial coordinate `x1` along the equator. Quadrant-aware
/// wrapper around `asc2`.
fn asc1(x1_deg: f64, f_deg: f64, sine: f64, cose: f64) -> f64 {
    const VERY_SMALL: f64 = 1.0e-10;
    let x1 = degnorm(x1_deg);
    let n = ((x1 / 90.0).floor() as i32) + 1;
    if (90.0 - f_deg).abs() < VERY_SMALL {
        return 180.0;
    }
    if (90.0 + f_deg).abs() < VERY_SMALL {
        return 0.0;
    }
    let mut ass = match n {
        1 => asc2(x1, f_deg, sine, cose),
        2 => 180.0 - asc2(180.0 - x1, -f_deg, sine, cose),
        3 => 180.0 + asc2(x1 - 180.0, -f_deg, sine, cose),
        _ => 360.0 - asc2(360.0 - x1, f_deg, sine, cose),
    };
    ass = degnorm(ass);
    for snap in [90.0, 180.0, 270.0, 360.0] {
        if (ass - snap).abs() < VERY_SMALL {
            ass = if snap == 360.0 { 0.0 } else { snap };
        }
    }
    ass
}

fn asc2(x_deg: f64, f_deg: f64, sine: f64, cose: f64) -> f64 {
    const VERY_SMALL: f64 = 1.0e-10;
    let mut ass = -tand(f_deg) * sine + cose * cosd(x_deg);
    if ass.abs() < VERY_SMALL {
        ass = 0.0;
    }
    let mut sinx = sind(x_deg);
    if sinx.abs() < VERY_SMALL {
        sinx = 0.0;
    }
    let mut out;
    if sinx == 0.0 {
        out = if ass < 0.0 { -VERY_SMALL } else { VERY_SMALL };
    } else if ass == 0.0 {
        out = if sinx < 0.0 { -90.0 } else { 90.0 };
    } else {
        out = atand(sinx / ass);
    }
    if out < 0.0 {
        out += 180.0;
    }
    out
}

#[inline]
fn sind(d: f64) -> f64 {
    libm::sin(d.to_radians())
}
#[inline]
fn cosd(d: f64) -> f64 {
    libm::cos(d.to_radians())
}
#[inline]
fn tand(d: f64) -> f64 {
    libm::tan(d.to_radians())
}
#[inline]
fn asind(x: f64) -> f64 {
    libm::asin(x).to_degrees()
}
#[inline]
fn atand(x: f64) -> f64 {
    libm::atan(x).to_degrees()
}

fn degnorm(d: f64) -> f64 {
    let mut x = d.rem_euclid(360.0);
    if x >= 360.0 {
        x -= 360.0;
    }
    x
}

/// Signed difference (a − b) wrapped to (−180°, +180°].
fn difdeg2n(a: f64, b: f64) -> f64 {
    let mut d = (a - b) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d <= -180.0 {
        d += 360.0;
    }
    d
}

fn wrap_two_pi(x: f64) -> f64 {
    let mut y = x.rem_euclid(TAU);
    // `rem_euclid` can return TAU exactly when the addition of TAU to a
    // negative-near-zero remainder rounds up at IEEE-754 precision.
    if y >= TAU {
        y -= TAU;
    }
    y
}

/// Twelve cusps del sistema **Polich-Page (Topocentric)**, formulado
/// por Wendel Polich y A. Page en 1961. A diferencia de Placidus —
/// que itera sobre el semi-arco diurno — Polich-Page tiene forma
/// cerrada: cada cusp intermedia usa una "altitud polar de la casa"
/// `F_n = atan(tan φ · n/3)` y se proyecta sobre la eclíptica con la
/// misma fórmula tipo asc/MC. Los cusps angulares (1, 4, 7, 10)
/// coinciden con ASC/IC/DESC/MC. Falla dentro del círculo polar
/// igual que Placidus.
///
/// Referencia: Polich & Page, *Topocentric System*, 1961.
pub fn polich_page_houses(
    last_rad: f64,
    lat_rad: f64,
    obliquity_rad: f64,
) -> Result<[f64; 12], &'static str> {
    let armc_deg = last_rad.to_degrees();
    let lat_deg = lat_rad.to_degrees();
    let eps_deg = obliquity_rad.to_degrees();

    if lat_deg.abs() >= 90.0 - eps_deg {
        return Err("Polich-Page undefined inside the polar circle");
    }

    let asc_deg = asc_meeus_to_degnorm(armc_deg, lat_deg, sind(eps_deg), cosd(eps_deg));
    let mc_deg = mc_meeus_to_degnorm(armc_deg, eps_deg);

    // Cusp intermedia para n signos desde MC. n=1,2,4,5 dan 11,12,2,3;
    // las opuestas (5,6,8,9) se derivan por +180°.
    let intermediate = |n_signs: f64| -> f64 {
        let f_rad = libm::atan(libm::tan(lat_rad) * n_signs / 3.0);
        let h_deg = armc_deg + n_signs * 30.0;
        let h_rad = h_deg.to_radians();
        let raw = libm::atan2(
            libm::sin(h_rad),
            libm::cos(h_rad) * cosd(eps_deg) - libm::tan(f_rad) * sind(eps_deg),
        );
        degnorm(raw.to_degrees())
    };

    let cusp_11 = intermediate(1.0);
    let cusp_12 = intermediate(2.0);
    let cusp_2 = intermediate(4.0);
    let cusp_3 = intermediate(5.0);

    let to_rad = |d: f64| (degnorm(d)).to_radians();

    Ok([
        to_rad(asc_deg),
        to_rad(cusp_2),
        to_rad(cusp_3),
        to_rad(mc_deg + 180.0),
        to_rad(cusp_11 + 180.0),
        to_rad(cusp_12 + 180.0),
        to_rad(asc_deg + 180.0),
        to_rad(cusp_2 + 180.0),
        to_rad(cusp_3 + 180.0),
        to_rad(mc_deg),
        to_rad(cusp_11),
        to_rad(cusp_12),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_sign_starts_at_asc_sign_cusp() {
        // Ascendant at 23.5° Cancer = 90° + 23.5° = 113.5°.
        let asc = 113.5_f64.to_radians();
        let cusps = whole_sign_houses(asc);
        // House 1 should be the start of Cancer = 90°.
        assert!((cusps[0].to_degrees() - 90.0).abs() < 1e-9);
        assert!((cusps[1].to_degrees() - 120.0).abs() < 1e-9);
        assert!((cusps[6].to_degrees() - 270.0).abs() < 1e-9);
    }

    #[test]
    fn equal_houses_step_30_deg() {
        let asc = 113.5_f64.to_radians();
        let cusps = equal_houses(asc);
        assert!((cusps[0] - asc).abs() < 1e-12);
        for i in 1..12 {
            let expected = wrap_two_pi(asc + (i as f64) * (PI / 6.0));
            assert!((cusps[i] - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn polich_page_angular_cusps_match_asc_mc() {
        // En cualquier latitud no-polar, cusp 1 = ASC, cusp 10 = MC,
        // cusp 4 = IC (MC+180), cusp 7 = DESC (ASC+180).
        let last = 120.0_f64.to_radians();
        let lat = 40.0_f64.to_radians();
        let eps = 23.4_f64.to_radians();
        let cusps = polich_page_houses(last, lat, eps).unwrap();
        let asc = ascendant(last, lat, eps);
        let mc = midheaven(last, eps);
        assert!((cusps[0] - asc).abs() < 1e-9);
        assert!((cusps[9] - mc).abs() < 1e-9);
        assert!((cusps[6] - wrap_two_pi(asc + PI)).abs() < 1e-9);
        assert!((cusps[3] - wrap_two_pi(mc + PI)).abs() < 1e-9);
    }

    #[test]
    fn polich_page_opposite_cusps_are_symmetric() {
        // Cada cusp i (i = 1..=12) tiene su opuesta en cusp i+6 = i+180°.
        let last = 85.0_f64.to_radians();
        let lat = -33.0_f64.to_radians();
        let eps = 23.4_f64.to_radians();
        let cusps = polich_page_houses(last, lat, eps).unwrap();
        for i in 0..6 {
            let opposite = wrap_two_pi(cusps[i] + PI);
            assert!(
                (cusps[i + 6] - opposite).abs() < 1e-9,
                "cusp {} y {} no son antipodal: {} vs {}",
                i + 1,
                i + 7,
                cusps[i + 6].to_degrees(),
                opposite.to_degrees()
            );
        }
    }

    #[test]
    fn polich_page_fails_inside_polar_circle() {
        let last = 0.0_f64.to_radians();
        let lat = 80.0_f64.to_radians();
        let eps = 23.4_f64.to_radians();
        assert!(polich_page_houses(last, lat, eps).is_err());
    }

    #[test]
    fn polich_page_diverges_from_placidus() {
        // En latitudes medias los dos sistemas dan resultados parecidos
        // pero no idénticos. Las cusps intermedias deben diferir al
        // menos en una fracción de grado.
        let last = 200.0_f64.to_radians();
        let lat = 45.0_f64.to_radians();
        let eps = 23.4_f64.to_radians();
        let pp = polich_page_houses(last, lat, eps).unwrap();
        let pl = placidus_houses(last, lat, eps).unwrap();
        // Cusp 11 (intermedia) — diferencia esperada ~0.1° o más
        let diff_11 = (pp[10] - pl[10]).to_degrees().abs();
        assert!(
            diff_11 > 0.05 && diff_11 < 5.0,
            "diff cusp 11 esperada en (0.05°, 5°), fue {}",
            diff_11
        );
    }

    #[test]
    fn ascendant_and_midheaven_have_valid_ranges() {
        for last_deg in [10.0, 90.0, 180.0, 270.0, 359.0] {
            for lat_deg in [-50.0, -10.0, 0.0, 10.0, 50.0] {
                let last = (last_deg as f64).to_radians();
                let lat = (lat_deg as f64).to_radians();
                let eps = 23.4_f64.to_radians();
                let asc = ascendant(last, lat, eps);
                let mc = midheaven(last, eps);
                assert!(
                    asc.is_finite() && asc >= 0.0 && asc < TAU,
                    "asc out of range for last={} lat={}: {}",
                    last_deg,
                    lat_deg,
                    asc
                );
                assert!(
                    mc.is_finite() && mc >= 0.0 && mc < TAU,
                    "mc out of range for last={} lat={}: {}",
                    last_deg,
                    lat_deg,
                    mc
                );
            }
        }
    }
}
