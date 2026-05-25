//! Fixed-star catalog and apparent-position pipeline (Phase 3, step 7).
//!
//! Contains a curated list of ~25 named bright stars taken verbatim
//! from Swiss Ephemeris' `sefstars.txt` (Hipparcos / Aloistr-Astrodienst,
//! AGPL-3). For each star we project the J2000 ICRS position by space
//! motion to the requested epoch, build a unit direction, then run the
//! same gravitational-deflection + stellar-aberration + NPB pipeline
//! we use for solar-system bodies and convert the result to ecliptic
//! longitude/latitude of date.
//!
//! Stellar parallax (BCRS→GCRS shift) IS applied: for each star at
//! distance `d = 1 / parallax_arcsec` parsec we subtract Earth's
//! heliocentric position from the star's BCRS Cartesian. This brings
//! nearby high-parallax stars (Rigil Kentaurus, Sirius, Procyon) into
//! sub-arcsec agreement with Swiss; without it Rigil Kentaurus would
//! drift by ~0.95″.

use cosmos_core::utils::jd_to_centuries;
use cosmos_core::Vector3;
use cosmos_ephemeris::jpl::SpkFile;
use cosmos_time::{NutationCalculator, TT};

use crate::oracle::OracleError;
use crate::sidereal::{ecliptic_lon_lat, tet_equatorial_to_ecliptic_of_date};

const PI: f64 = std::f64::consts::PI;
const TAU: f64 = std::f64::consts::TAU;
const MAS_TO_RAD: f64 = PI / (180.0 * 3600.0 * 1000.0);

/// One catalogued fixed star. All angular quantities at J2000.0 / ICRS.
#[derive(Debug, Clone, Copy)]
pub struct Star {
    pub name: &'static str,
    pub bayer: &'static str,
    pub ra_deg: f64,
    pub dec_deg: f64,
    /// Proper motion in RA *with* cos(dec) factor, mas/year.
    /// (Hipparcos / Swiss convention.)
    pub pm_ra_mas_yr: f64,
    pub pm_dec_mas_yr: f64,
    pub parallax_mas: f64,
    pub rv_km_s: f64,
    pub mag: f64,
}

/// Curated subset of bright stars (V mag ≤ 1.65), names ordered by
/// ascending magnitude. Values from `sefstars.txt`.
pub const CATALOG: &[Star] = &[
    Star { name: "Sirius", bayer: "alCMa", ra_deg: 101.2871553333, dec_deg: -16.7161158611, pm_ra_mas_yr: -546.01, pm_dec_mas_yr: -1223.07, parallax_mas: 379.21, rv_km_s: -5.5, mag: -1.46 },
    Star { name: "Canopus", bayer: "alCar", ra_deg: 95.9879578333, dec_deg: -52.6956613889, pm_ra_mas_yr: 19.93, pm_dec_mas_yr: 23.24, parallax_mas: 10.55, rv_km_s: 20.3, mag: -0.74 },
    Star { name: "Rigil Kentaurus", bayer: "alCen", ra_deg: 219.9008500000, dec_deg: -60.8356194444, pm_ra_mas_yr: -3608.0, pm_dec_mas_yr: 686.0, parallax_mas: 742.0, rv_km_s: -22.3, mag: -0.1 },
    Star { name: "Arcturus", bayer: "alBoo", ra_deg: 213.9153002917, dec_deg: 19.1824091667, pm_ra_mas_yr: -1093.39, pm_dec_mas_yr: -2000.06, parallax_mas: 88.83, rv_km_s: -5.19, mag: -0.05 },
    Star { name: "Vega", bayer: "alLyr", ra_deg: 279.2347347917, dec_deg: 38.7836889444, pm_ra_mas_yr: 200.94, pm_dec_mas_yr: 286.23, parallax_mas: 130.23, rv_km_s: -20.6, mag: 0.03 },
    Star { name: "Capella", bayer: "alAur", ra_deg: 79.1723279583, dec_deg: 45.9979914722, pm_ra_mas_yr: 75.25, pm_dec_mas_yr: -426.89, parallax_mas: 76.2, rv_km_s: 29.19, mag: 0.08 },
    Star { name: "Rigel", bayer: "beOri", ra_deg: 78.6344670833, dec_deg: -8.2016383611, pm_ra_mas_yr: 1.31, pm_dec_mas_yr: 0.5, parallax_mas: 3.78, rv_km_s: 17.8, mag: 0.13 },
    Star { name: "Procyon", bayer: "alCMi", ra_deg: 114.8254979167, dec_deg: 5.2249875556, pm_ra_mas_yr: -714.59, pm_dec_mas_yr: -1036.8, parallax_mas: 284.56, rv_km_s: -3.2, mag: 0.37 },
    Star { name: "Betelgeuse", bayer: "alOri", ra_deg: 88.7929390000, dec_deg: 7.4070640000, pm_ra_mas_yr: 27.54, pm_dec_mas_yr: 11.3, parallax_mas: 6.55, rv_km_s: 21.91, mag: 0.42 },
    Star { name: "Achernar", bayer: "alEri", ra_deg: 24.4285228333, dec_deg: -57.2367528056, pm_ra_mas_yr: 87.0, pm_dec_mas_yr: -38.24, parallax_mas: 23.39, rv_km_s: 18.6, mag: 0.46 },
    Star { name: "Hadar", bayer: "beCen", ra_deg: 210.9558556250, dec_deg: -60.3730351667, pm_ra_mas_yr: -33.27, pm_dec_mas_yr: -23.16, parallax_mas: 8.32, rv_km_s: 5.9, mag: 0.6 },
    Star { name: "Altair", bayer: "alAql", ra_deg: 297.6958272917, dec_deg: 8.8683211944, pm_ra_mas_yr: 536.23, pm_dec_mas_yr: 385.29, parallax_mas: 194.95, rv_km_s: -26.6, mag: 0.76 },
    Star { name: "Acrux", bayer: "alCru", ra_deg: 186.6495634167, dec_deg: -63.0990928611, pm_ra_mas_yr: -35.83, pm_dec_mas_yr: -14.86, parallax_mas: 10.13, rv_km_s: 11.9, mag: 0.81 },
    Star { name: "Aldebaran", bayer: "alTau", ra_deg: 68.9801627917, dec_deg: 16.5093023611, pm_ra_mas_yr: 63.45, pm_dec_mas_yr: -188.94, parallax_mas: 48.94, rv_km_s: 54.26, mag: 0.86 },
    Star { name: "Antares", bayer: "alSco", ra_deg: 247.3519154167, dec_deg: -26.4320026111, pm_ra_mas_yr: -12.11, pm_dec_mas_yr: -23.3, parallax_mas: 5.89, rv_km_s: -3.5, mag: 0.91 },
    Star { name: "Spica", bayer: "alVir", ra_deg: 201.2982473750, dec_deg: -11.1613194722, pm_ra_mas_yr: -42.35, pm_dec_mas_yr: -30.67, parallax_mas: 13.06, rv_km_s: 1.0, mag: 0.97 },
    Star { name: "Pollux", bayer: "beGem", ra_deg: 116.3289577917, dec_deg: 28.0261988889, pm_ra_mas_yr: -626.55, pm_dec_mas_yr: -45.8, parallax_mas: 96.54, rv_km_s: 3.23, mag: 1.14 },
    Star { name: "Fomalhaut", bayer: "alPsA", ra_deg: 344.4126927083, dec_deg: -29.6222370278, pm_ra_mas_yr: 328.95, pm_dec_mas_yr: -164.67, parallax_mas: 129.81, rv_km_s: 6.5, mag: 1.16 },
    Star { name: "Deneb", bayer: "alCyg", ra_deg: 310.3579797500, dec_deg: 45.2803388056, pm_ra_mas_yr: 2.01, pm_dec_mas_yr: 1.85, parallax_mas: 2.31, rv_km_s: -4.9, mag: 1.25 },
    Star { name: "Mimosa", bayer: "beCru", ra_deg: 191.9302865417, dec_deg: -59.6887720000, pm_ra_mas_yr: -42.97, pm_dec_mas_yr: -16.18, parallax_mas: 11.71, rv_km_s: 10.3, mag: 1.25 },
    Star { name: "Regulus", bayer: "alLeo", ra_deg: 152.0929624583, dec_deg: 11.9672087778, pm_ra_mas_yr: -248.73, pm_dec_mas_yr: 5.59, parallax_mas: 41.13, rv_km_s: 5.9, mag: 1.4 },
    Star { name: "Adara", bayer: "epCMa", ra_deg: 104.6564531667, dec_deg: -28.9720861667, pm_ra_mas_yr: 3.24, pm_dec_mas_yr: 1.33, parallax_mas: 8.05, rv_km_s: 27.3, mag: 1.5 },
    Star { name: "Castor", bayer: "alGem", ra_deg: 113.6494716250, dec_deg: 31.8882822222, pm_ra_mas_yr: -191.45, pm_dec_mas_yr: -145.19, parallax_mas: 64.12, rv_km_s: 5.4, mag: 1.58 },
    Star { name: "Shaula", bayer: "laSco", ra_deg: 263.4021671667, dec_deg: -37.1038235556, pm_ra_mas_yr: -8.53, pm_dec_mas_yr: -30.8, parallax_mas: 5.71, rv_km_s: -3.0, mag: 1.62 },
    Star { name: "Bellatrix", bayer: "gaOri", ra_deg: 81.2827635417, dec_deg: 6.3497032778, pm_ra_mas_yr: -8.11, pm_dec_mas_yr: -12.88, parallax_mas: 12.92, rv_km_s: 18.2, mag: 1.64 },
    Star { name: "Elnath", bayer: "beTau", ra_deg: 81.5729713333, dec_deg: 28.6074517222, pm_ra_mas_yr: 22.76, pm_dec_mas_yr: -173.58, parallax_mas: 24.36, rv_km_s: 9.2, mag: 1.65 },
];

/// Look up a star by case-insensitive name match (e.g. "Spica", "antares").
pub fn by_name(name: &str) -> Option<&'static Star> {
    let needle = name.to_ascii_lowercase();
    CATALOG.iter().find(|s| s.name.eq_ignore_ascii_case(&needle))
}

/// 1 parsec in km: AU_KM × (180·3600/π).
const PARSEC_KM: f64 = cosmos_core::constants::AU_KM * 206_264.806_247_096_36;

/// ICRS unit direction at the given epoch, after applying space motion
/// linearly (proper motion). The radial-velocity term has a sub-µas
/// effect over a century for our stars and is omitted.
fn icrs_direction_at(star: &Star, jd_tdb: f64) -> Vector3 {
    let dt_yr = (jd_tdb - 2_451_545.0) / 365.25;
    let cos_dec = libm::cos(star.dec_deg.to_radians());
    // Hipparcos pmRA already includes cos(dec); divide it out to get pure RA rate.
    let d_ra_rad = star.pm_ra_mas_yr * MAS_TO_RAD * dt_yr / cos_dec;
    let d_dec_rad = star.pm_dec_mas_yr * MAS_TO_RAD * dt_yr;
    let ra = star.ra_deg.to_radians() + d_ra_rad;
    let dec = star.dec_deg.to_radians() + d_dec_rad;
    let cd = libm::cos(dec);
    Vector3::new(cd * libm::cos(ra), cd * libm::sin(ra), libm::sin(dec))
}

/// Star distance in km derived from parallax. Returns `None` for the
/// nominal "infinite distance" case (zero parallax in the catalog).
fn star_distance_km(star: &Star) -> Option<f64> {
    if star.parallax_mas <= 0.0 {
        return None;
    }
    // 1 parsec = 1 AU / tan(1″); for parallax in milliarcsec, distance
    // in parsec = 1000 / parallax_mas.
    let distance_pc = 1000.0 / star.parallax_mas;
    Some(distance_pc * PARSEC_KM)
}

/// Apparent ecliptic (longitude, latitude) of a fixed star at the given
/// TT, in radians. Pipeline: ICRS J2000 + space motion → unit direction
/// → light deflection by Sun → stellar aberration → NPB rotation to
/// true equator and equinox of date → ecliptic-of-date longitude/lat.
pub fn apparent_ecliptic_of_date(
    star: &Star,
    spk: &SpkFile,
    tt: &TT,
    jd_tdb: f64,
) -> Result<(f64, f64), OracleError> {
    use cosmos_core::constants::{AU_KM, SECONDS_PER_DAY_F64};

    // Earth state in ICRF for aberration + LD geometry.
    // We use 399 wrt 3 + 3 wrt 0 chain since we don't have a direct
    // 399 wrt 0 segment in DE440.
    let (e_emb_pos, e_emb_vel) = spk.compute_state(399, 3, jd_tdb)?;
    let (emb_ssb_pos, emb_ssb_vel) = spk.compute_state(3, 0, jd_tdb)?;
    let earth_pos_ssb_km = [
        e_emb_pos[0] + emb_ssb_pos[0],
        e_emb_pos[1] + emb_ssb_pos[1],
        e_emb_pos[2] + emb_ssb_pos[2],
    ];
    let earth_vel_ssb_kms = [
        e_emb_vel[0] + emb_ssb_vel[0],
        e_emb_vel[1] + emb_ssb_vel[1],
        e_emb_vel[2] + emb_ssb_vel[2],
    ];

    // Sun-to-Earth vector for LD.
    let (sun_pos_ssb, _) = spk.compute_state(10, 0, jd_tdb)?;
    let sun_to_earth_km = [
        earth_pos_ssb_km[0] - sun_pos_ssb[0],
        earth_pos_ssb_km[1] - sun_pos_ssb[1],
        earth_pos_ssb_km[2] - sun_pos_ssb[2],
    ];
    let sun_earth_dist_km = libm::sqrt(
        sun_to_earth_km[0] * sun_to_earth_km[0]
            + sun_to_earth_km[1] * sun_to_earth_km[1]
            + sun_to_earth_km[2] * sun_to_earth_km[2],
    );
    let sun_earth_dist_au = sun_earth_dist_km / AU_KM;
    let sun_to_earth_unit = Vector3::new(
        sun_to_earth_km[0] / sun_earth_dist_km,
        sun_to_earth_km[1] / sun_earth_dist_km,
        sun_to_earth_km[2] / sun_earth_dist_km,
    );

    let earth_vel_au_day = Vector3::new(
        earth_vel_ssb_kms[0] * SECONDS_PER_DAY_F64 / AU_KM,
        earth_vel_ssb_kms[1] * SECONDS_PER_DAY_F64 / AU_KM,
        earth_vel_ssb_kms[2] * SECONDS_PER_DAY_F64 / AU_KM,
    );

    // Star direction after BCRS→GCRS parallax shift. The star's BCRS
    // Cartesian is `dir_icrs * distance`; subtracting Earth's
    // heliocentric position gives the geocentric direction vector
    // (whose angular shift from the BCRS direction is the annual
    // parallax). For low-parallax stars the shift is sub-µas; for
    // Rigil Kentaurus it's ~0.74″.
    let dir_icrs = icrs_direction_at(star, jd_tdb);
    let dir_geocentric = match star_distance_km(star) {
        Some(d_km) => {
            let earth_helio = [
                earth_pos_ssb_km[0] - sun_pos_ssb[0],
                earth_pos_ssb_km[1] - sun_pos_ssb[1],
                earth_pos_ssb_km[2] - sun_pos_ssb[2],
            ];
            let bcrs_pos = Vector3::new(
                dir_icrs.x * d_km - earth_helio[0],
                dir_icrs.y * d_km - earth_helio[1],
                dir_icrs.z * d_km - earth_helio[2],
            );
            let r = libm::sqrt(bcrs_pos.x * bcrs_pos.x + bcrs_pos.y * bcrs_pos.y + bcrs_pos.z * bcrs_pos.z);
            Vector3::new(bcrs_pos.x / r, bcrs_pos.y / r, bcrs_pos.z / r)
        }
        None => dir_icrs,
    };
    let dir_after_ld = cosmos_coords::aberration::apply_light_deflection(
        dir_geocentric,
        sun_to_earth_unit,
        sun_earth_dist_au,
    );
    let dir_after_ab = cosmos_coords::aberration::apply_aberration(
        dir_after_ld,
        earth_vel_au_day,
        sun_earth_dist_au,
    );

    // NPB rotation to TET.
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
    let dir_tet = npb * dir_after_ab;

    // Convert TET equatorial to ecliptic-of-date.
    let dir_ecl = tet_equatorial_to_ecliptic_of_date(dir_tet, tt);
    let (lon, lat) = ecliptic_lon_lat(dir_ecl);
    Ok((wrap_two_pi(lon), lat))
}

fn wrap_two_pi(x: f64) -> f64 {
    let mut y = x.rem_euclid(TAU);
    if y >= TAU {
        y -= TAU;
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_known_stars() {
        assert!(by_name("Spica").is_some());
        assert!(by_name("vega").is_some());  // case-insensitive
        assert!(by_name("Sirius").unwrap().mag < 0.0);
    }

    #[test]
    fn icrs_direction_at_j2000_matches_ra_dec_unit_vector() {
        let spica = by_name("Spica").unwrap();
        let dir = icrs_direction_at(spica, 2_451_545.0);
        let ra = spica.ra_deg.to_radians();
        let dec = spica.dec_deg.to_radians();
        let cd = libm::cos(dec);
        assert!((dir.x - cd * libm::cos(ra)).abs() < 1.0e-15);
        assert!((dir.y - cd * libm::sin(ra)).abs() < 1.0e-15);
        assert!((dir.z - libm::sin(dec)).abs() < 1.0e-15);
    }
}
