//! Topocentric correction for solar-system positions (Phase 3, step 5).
//!
//! Given an apparent geocentric Cartesian in TET and an observer
//! `(lat, lon, elev)` on the WGS-84 ellipsoid, this module computes the
//! observer's geocentric position in the same TET frame and returns the
//! topocentric vector (body − observer).
//!
//! Polar motion is intentionally ignored at this stage. The dominant
//! topocentric effect is the diurnal parallax, which for the Moon is
//! up to ~1° and dwarfs the sub-arcsec polar-motion contribution.
//! Phase 5 of the v1 roadmap can add `EOP::polar_motion(jd)` to tighten
//! observer position to sub-mas if needed.

use cosmos_core::Vector3;
use cosmos_time::julian::JulianDate;
use cosmos_time::scales::conversions::ToUT1WithDeltaT;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::sidereal::GAST;
use cosmos_time::{TDB, TT, UT1};

use crate::oracle::{Backend, Oracle, OracleError, StateKmS};

/// Observer geographic location, geodetic. Latitude/longitude in radians,
/// elevation in metres above the WGS-84 ellipsoid.
#[derive(Debug, Clone, Copy)]
pub struct Observer {
    pub lat_rad: f64,
    pub lon_rad: f64,
    pub elev_m: f64,
}

impl Observer {
    pub fn from_degrees(lat_deg: f64, lon_deg: f64, elev_m: f64) -> Self {
        Self {
            lat_rad: lat_deg.to_radians(),
            lon_rad: lon_deg.to_radians(),
            elev_m,
        }
    }
}

/// Compute the observer's position in the TET (true equator and equinox
/// of date) frame, in km. The chain is:
/// `(lat, lon, elev)` → ITRS (WGS-84) → TIRS (no polar motion) →
/// TET via R3(GAST) rotation around the equator-of-date Z-axis.
pub fn observer_position_tet_km(
    obs: &Observer,
    ut1: &UT1,
    tt: &TT,
) -> Result<Vector3, OracleError> {
    let itrs_m = wgs84_geodetic_to_itrs(obs);
    let itrs_km = Vector3::new(itrs_m.x * 1.0e-3, itrs_m.y * 1.0e-3, itrs_m.z * 1.0e-3);

    let gast = GAST::from_ut1_and_tt(ut1, tt)
        .map_err(|e| OracleError::Inner(format!("GAST: {:?}", e)))?;
    let gast_rad = gast.radians();

    Ok(rotate_z(itrs_km, gast_rad))
}

/// Inline WGS-84 geodetic-to-ITRS converter (returns metres). Mirrors
/// `cosmos_coords::frames::ITRSPosition::from_geodetic` so we do not
/// pull in the larger frames module just for one formula.
fn wgs84_geodetic_to_itrs(obs: &Observer) -> Vector3 {
    const A: f64 = 6_378_137.0;
    const F: f64 = 1.0 / 298.257_223_563;

    let (sin_lat, cos_lat) = libm::sincos(obs.lat_rad);
    let (sin_lon, cos_lon) = libm::sincos(obs.lon_rad);

    let w = 1.0 - F;
    let w2 = w * w;
    let d = cos_lat * cos_lat + w2 * sin_lat * sin_lat;
    let ac = A / libm::sqrt(d);
    let a_s = w2 * ac;

    let r = (ac + obs.elev_m) * cos_lat;
    Vector3::new(
        r * cos_lon,
        r * sin_lon,
        (a_s + obs.elev_m) * sin_lat,
    )
}

/// Rotate `v` around the Z axis by `angle_rad`. Positive angle takes
/// the +X axis into the +Y direction (right-handed rotation).
fn rotate_z(v: Vector3, angle_rad: f64) -> Vector3 {
    let (sin_a, cos_a) = libm::sincos(angle_rad);
    Vector3::new(
        cos_a * v.x - sin_a * v.y,
        sin_a * v.x + cos_a * v.y,
        v.z,
    )
}

/// Apparent topocentric state of `body` as seen from `observer` at TDB
/// epoch `jd_tdb`. Returns Cartesian km / km·s⁻¹ in the TET frame.
///
/// Velocity is the geocentric apparent velocity minus a small term
/// driven by the observer's diurnal motion; today we approximate that
/// term as zero (acceptable at the m/s level).
pub fn apparent_topocentric_state(
    oracle: &Oracle,
    body: i32,
    jd_tdb: f64,
    observer: &Observer,
    delta_t_seconds: f64,
) -> Result<StateKmS, OracleError> {
    use crate::fixture::{Corrections, Frame};

    // Geocentric apparent body in TET.
    let body_geo = oracle.corrected_state(
        body,
        /* center = Earth body */ 399,
        jd_tdb,
        Frame::TrueEquatorEquinoxOfDate,
        Corrections::APPARENT,
    )?;

    let tt = TDB::from_julian_date(JulianDate::new(jd_tdb, 0.0))
        .to_tt_greenwich()
        .map_err(|e| OracleError::Inner(format!("TDB→TT: {:?}", e)))?;
    let ut1 = tt
        .to_ut1_with_delta_t(delta_t_seconds)
        .map_err(|e| OracleError::Inner(format!("TT→UT1: {:?}", e)))?;

    let obs_tet = observer_position_tet_km(observer, &ut1, &tt)?;

    Ok(StateKmS {
        pos_km: [
            body_geo.pos_km[0] - obs_tet.x,
            body_geo.pos_km[1] - obs_tet.y,
            body_geo.pos_km[2] - obs_tet.z,
        ],
        // First-order: keep geocentric apparent velocity (observer's
        // diurnal speed is ~0.5 km/s at the equator and only matters
        // for sub-mm/s-class velocity reporting).
        vel_km_s: body_geo.vel_km_s,
    })
}

/// Convert a topocentric Cartesian state in the TET frame to local
/// **altitude / azimuth** in radians, given the observer's geodetic
/// latitude and the **Local Apparent Sidereal Time** at the observer
/// (in radians; obtainable via `cosmos_time::sidereal::GAST::to_last`).
///
/// Azimuth follows the modern N=0°, E=90°, S=180°, W=270° convention.
/// Altitude is geometric (no atmospheric refraction). To match Swiss
/// Ephemeris' S=0° / W=90° convention, add 180° (mod 360°) to the
/// returned value.
///
/// Formulas: Meeus 13.5 / 13.6.
pub fn alt_az_from_topocentric(
    topo_tet: [f64; 3],
    lat_rad: f64,
    last_rad: f64,
) -> (f64, f64) {
    // RA, Dec from the TET-frame Cartesian.
    let r = libm::sqrt(
        topo_tet[0] * topo_tet[0]
            + topo_tet[1] * topo_tet[1]
            + topo_tet[2] * topo_tet[2],
    );
    let dec = libm::asin(topo_tet[2] / r);
    let ra = libm::atan2(topo_tet[1], topo_tet[0]);

    let h = last_rad - ra;
    let (sin_h, cos_h) = libm::sincos(h);
    let (sin_phi, cos_phi) = libm::sincos(lat_rad);
    let (sin_d, cos_d) = libm::sincos(dec);

    let sin_alt = sin_phi * sin_d + cos_phi * cos_d * cos_h;
    let alt = libm::asin(sin_alt);
    let az = libm::atan2(-sin_h * cos_d, cos_phi * sin_d - sin_phi * cos_d * cos_h);
    let az = if az < 0.0 { az + std::f64::consts::TAU } else { az };
    (alt, az)
}

/// Convenience: full pipeline body → apparent topocentric Cartesian →
/// (alt, az). Returns `(alt_rad, az_rad)` with N=0° azimuth.
pub fn apparent_alt_az(
    oracle: &Oracle,
    body: i32,
    jd_tdb: f64,
    observer: &Observer,
    delta_t_seconds: f64,
) -> Result<(f64, f64), OracleError> {
    use cosmos_time::sidereal::GAST;

    let topo = apparent_topocentric_state(oracle, body, jd_tdb, observer, delta_t_seconds)?;

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
    let gast =
        GAST::from_ut1_and_tt(&ut1, &tt).map_err(|e| OracleError::Inner(format!("GAST: {:?}", e)))?;
    let last = gast.to_last(&location);
    let last_rad = last.angle().radians();

    Ok(alt_az_from_topocentric(topo.pos_km, observer.lat_rad, last_rad))
}

/// Convenience wrapper for callers that already know they want an SPK
/// backend pointed at the kernel of their choice.
pub fn apparent_topocentric_with_kernel(
    kernel_path: std::path::PathBuf,
    body: i32,
    jd_tdb: f64,
    observer: &Observer,
    delta_t_seconds: f64,
) -> Result<StateKmS, OracleError> {
    let oracle = Oracle::new(Backend::Spk { kernel_path })?;
    apparent_topocentric_state(&oracle, body, jd_tdb, observer, delta_t_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_at_greenwich_has_expected_geocentric_distance() {
        let greenwich = Observer::from_degrees(51.4769, 0.0, 0.0);
        let itrs = wgs84_geodetic_to_itrs(&greenwich);
        let r = libm::sqrt(itrs.x * itrs.x + itrs.y * itrs.y + itrs.z * itrs.z);
        // Geocentric distance at lat 51.5° on WGS-84: ~6_365 km — between
        // the equatorial 6378 km and polar 6357 km.
        assert!(
            (6_360_000.0..6_370_000.0).contains(&r),
            "geocentric distance {} m out of expected band",
            r
        );
    }

    #[test]
    fn equatorial_observer_at_zero_longitude_lies_on_x_axis() {
        let eq = Observer::from_degrees(0.0, 0.0, 0.0);
        let v = wgs84_geodetic_to_itrs(&eq);
        assert!((v.x - 6_378_137.0).abs() < 1.0);
        assert!(v.y.abs() < 1.0);
        assert!(v.z.abs() < 1.0);
    }

    #[test]
    fn rotate_z_recovers_original_after_full_turn() {
        let v = Vector3::new(1.0, 2.0, 3.0);
        let r = rotate_z(v, std::f64::consts::TAU);
        assert!((r.x - v.x).abs() < 1.0e-12);
        assert!((r.y - v.y).abs() < 1.0e-12);
        assert!((r.z - v.z).abs() < 1.0e-12);
    }
}
