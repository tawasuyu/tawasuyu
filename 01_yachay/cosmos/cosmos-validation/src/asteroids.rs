//! Asteroid apparent-position pipeline (Phase 3, step 11).
//!
//! Asteroids are stored in their own SPK kernel (e.g. `sb441-n16.bsp`,
//! 16 main-belt asteroids referenced to Sun) separate from the planet
//! kernel (`de440.bsp`). This module composes the two: it pulls the
//! asteroid's heliocentric state from the asteroid kernel, the Earth's
//! heliocentric state from the planet kernel, and runs the same
//! apparent-position pipeline (LT iteration, light deflection, stellar
//! aberration, NPB rotation, ecliptic conversion) that we use for
//! planets.
//!
//! Naming: NAIF small-body IDs are `2_NNN_NNN` for numbered asteroids
//! (e.g. Ceres = 2000001) and `2_NNN_NNN` for centaurs (Chiron =
//! 2002060, but Chiron lives in a different kernel than the main-belt
//! 16-asteroid file shipped with DE441).

use cosmos_core::constants::{AU_KM, SECONDS_PER_DAY_F64};
use cosmos_core::utils::jd_to_centuries;
use cosmos_core::Vector3;
use cosmos_ephemeris::jpl::SpkFile;
use cosmos_time::{NutationCalculator, TT};

use crate::oracle::OracleError;
use crate::sidereal::{ecliptic_lon_lat, tet_equatorial_to_ecliptic_of_date};

const C_AU_PER_DAY: f64 = 173.144_632_684_669_3;

/// Curated NAIF small-body IDs and human-readable names. The four
/// main-belt astrologically-important asteroids (Ceres, Pallas, Juno,
/// Vesta) are present in `sb441-n16.bsp`. Chiron, Pholus, Eris, Sedna
/// and other centaurs/TNOs are distributed by JPL Horizons as **Type 21**
/// SPK kernels (Modified Divided Differences), which the eternal-
/// ephemeris reader does not yet support — only Type 2 Chebyshev. Their
/// IDs are listed here for documentation and so a future Type-21-capable
/// reader can plug straight into the existing `apparent_ecliptic_of_date`
/// pipeline.
pub const KNOWN_ASTEROIDS: &[(i32, &str)] = &[
    (2_000_001, "Ceres"),
    (2_000_002, "Pallas"),
    (2_000_003, "Juno"),
    (2_000_004, "Vesta"),
    // The bodies below need a Type 21 SPK reader:
    (2_002_060, "Chiron"),
    (2_005_145, "Pholus"),
    (2_136_199, "Eris"),
    (2_090_377, "Sedna"),
];

/// Look up the NAIF ID by case-insensitive name.
pub fn naif_id_by_name(name: &str) -> Option<i32> {
    KNOWN_ASTEROIDS
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(name))
        .map(|(id, _)| *id)
}

/// Apparent geocentric ecliptic-of-date position of an asteroid, in
/// radians. Returns `(longitude, latitude, distance_au)`.
///
/// `asteroid_spk` is the kernel that contains the asteroid (e.g.
/// `sb441-n16.bsp`); `planet_spk` is the kernel that contains Earth
/// and Sun (e.g. `de440.bsp`). Pass the same path twice if your
/// kernel has both.
pub fn apparent_ecliptic_of_date(
    asteroid_id: i32,
    asteroid_spk: &SpkFile,
    planet_spk: &SpkFile,
    tt: &TT,
    jd_tdb: f64,
) -> Result<(f64, f64, f64), OracleError> {
    // 1. Compute asteroid SSB position with light-time iteration.
    //    Asteroid SPK stores body wrt Sun (center=10). We add Sun-wrt-SSB
    //    from the planet kernel to get asteroid-wrt-SSB.
    //
    //    Observer (Earth) position is at observation time t_obs in SSB.
    //    Asteroid is queried at emission time t_emit = t_obs − τ,
    //    where τ = |asteroid(t_emit) − earth(t_obs)| / c.
    let (earth_pos_obs, earth_vel_obs) = earth_ssb_state(planet_spk, jd_tdb)?;

    // First-pass τ from geometric distance at t_obs.
    let mut tau_days = {
        let asteroid_pos_obs = asteroid_ssb_position(asteroid_spk, planet_spk, asteroid_id, jd_tdb)?;
        let dx = asteroid_pos_obs.x - earth_pos_obs.x;
        let dy = asteroid_pos_obs.y - earth_pos_obs.y;
        let dz = asteroid_pos_obs.z - earth_pos_obs.z;
        let dist_km = libm::sqrt(dx * dx + dy * dy + dz * dz);
        (dist_km / AU_KM) / C_AU_PER_DAY
    };

    let mut asteroid_pos_emit = Vector3::zeros();
    for _ in 0..6 {
        let jd_emit = jd_tdb - tau_days;
        asteroid_pos_emit = asteroid_ssb_position(asteroid_spk, planet_spk, asteroid_id, jd_emit)?;
        let dx = asteroid_pos_emit.x - earth_pos_obs.x;
        let dy = asteroid_pos_emit.y - earth_pos_obs.y;
        let dz = asteroid_pos_emit.z - earth_pos_obs.z;
        let dist_km = libm::sqrt(dx * dx + dy * dy + dz * dz);
        let new_tau = (dist_km / AU_KM) / C_AU_PER_DAY;
        let converged = (new_tau - tau_days).abs() < 1.0e-15;
        tau_days = new_tau;
        if converged {
            break;
        }
    }

    let astrometric = Vector3::new(
        asteroid_pos_emit.x - earth_pos_obs.x,
        asteroid_pos_emit.y - earth_pos_obs.y,
        asteroid_pos_emit.z - earth_pos_obs.z,
    );
    let astrometric_dist_km = libm::sqrt(
        astrometric.x * astrometric.x
            + astrometric.y * astrometric.y
            + astrometric.z * astrometric.z,
    );

    // 2. Light deflection by Sun.
    let (sun_pos_ssb, _) = planet_spk
        .compute_state(10, 0, jd_tdb)
        .map_err(OracleError::from)?;
    let sun_to_earth_km = [
        earth_pos_obs.x - sun_pos_ssb[0],
        earth_pos_obs.y - sun_pos_ssb[1],
        earth_pos_obs.z - sun_pos_ssb[2],
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

    let dir = Vector3::new(
        astrometric.x / astrometric_dist_km,
        astrometric.y / astrometric_dist_km,
        astrometric.z / astrometric_dist_km,
    );
    let dir_after_ld = cosmos_coords::aberration::apply_light_deflection(
        dir,
        sun_to_earth_unit,
        sun_earth_dist_au,
    );

    // 3. Stellar aberration.
    let earth_vel_au_day = Vector3::new(
        earth_vel_obs.x * SECONDS_PER_DAY_F64 / AU_KM,
        earth_vel_obs.y * SECONDS_PER_DAY_F64 / AU_KM,
        earth_vel_obs.z * SECONDS_PER_DAY_F64 / AU_KM,
    );
    let dir_after_ab = cosmos_coords::aberration::apply_aberration(
        dir_after_ld,
        earth_vel_au_day,
        sun_earth_dist_au,
    );

    // 4. NPB rotation to TET.
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

    // 5. Ecliptic-of-date longitude/latitude.
    let dir_ecl = tet_equatorial_to_ecliptic_of_date(dir_tet, tt);
    let (lon, lat) = ecliptic_lon_lat(dir_ecl);
    let lon = if lon < 0.0 {
        lon + std::f64::consts::TAU
    } else {
        lon
    };

    Ok((lon, lat, astrometric_dist_km / AU_KM))
}

/// Earth state (position + velocity) wrt SSB in km / km·s⁻¹, using the
/// `(399 wrt 3) + (3 wrt 0)` chain that DE440 supports directly.
fn earth_ssb_state(planet_spk: &SpkFile, jd_tdb: f64) -> Result<(Vector3, Vector3), OracleError> {
    let (e_emb_pos, e_emb_vel) = planet_spk
        .compute_state(399, 3, jd_tdb)
        .map_err(OracleError::from)?;
    let (emb_ssb_pos, emb_ssb_vel) = planet_spk
        .compute_state(3, 0, jd_tdb)
        .map_err(OracleError::from)?;
    Ok((
        Vector3::new(
            e_emb_pos[0] + emb_ssb_pos[0],
            e_emb_pos[1] + emb_ssb_pos[1],
            e_emb_pos[2] + emb_ssb_pos[2],
        ),
        Vector3::new(
            e_emb_vel[0] + emb_ssb_vel[0],
            e_emb_vel[1] + emb_ssb_vel[1],
            e_emb_vel[2] + emb_ssb_vel[2],
        ),
    ))
}

/// Asteroid heliocentric position via asteroid SPK + Sun SSB position
/// from planet SPK.
fn asteroid_ssb_position(
    asteroid_spk: &SpkFile,
    planet_spk: &SpkFile,
    asteroid_id: i32,
    jd_tdb: f64,
) -> Result<Vector3, OracleError> {
    let (a_helio, _) = asteroid_spk
        .compute_state(asteroid_id, 10, jd_tdb)
        .map_err(OracleError::from)?;
    let (sun_ssb, _) = planet_spk
        .compute_state(10, 0, jd_tdb)
        .map_err(OracleError::from)?;
    Ok(Vector3::new(
        a_helio[0] + sun_ssb[0],
        a_helio[1] + sun_ssb[1],
        a_helio[2] + sun_ssb[2],
    ))
}
