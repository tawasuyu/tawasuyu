//! `EphemerisSession`: an opened handle to an ephemeris backend.
//!
//! Owns the heavy resources (memory-mapped SPK kernels, lazy ΔT table,
//! analytical series state) so they stay alive across many queries — the
//! astrologer's rectification loop pays the kernel-open cost once and
//! then iterates cheaply.

use std::path::{Path, PathBuf};

use cosmos_core::Vector3;
use cosmos_ephemeris::jpl::SpkFile;
use cosmos_validation::fixture::{Corrections, Frame};
use cosmos_validation::oracle::{Backend, Oracle, OracleError};
use cosmos_validation::sidereal::{tet_equatorial_to_ecliptic_of_date, true_obliquity_iau2006a};
use cosmos_validation::topocentric::{alt_az_from_topocentric, observer_position_tet_km};

use crate::apparent::{
    wrap_two_pi, ApparentPosition, EclipticCoord, EclipticVelocity, EquatorialCoord,
    HorizonCoord,
};
use crate::body::{Body, BodyKind};
use crate::error::{SkyError, SkyResult};
use crate::instant::Instant;
use crate::observer::Observer;

/// Configuration for opening an [`EphemerisSession`].
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub backend: SessionBackend,
    /// Optional separate SPK kernel for small bodies (e.g.
    /// `sb441-n16.bsp` for the 16 main-belt asteroids that ship with
    /// DE441). Required to compute [`Body::Ceres`], [`Body::Pallas`],
    /// [`Body::Juno`], [`Body::Vesta`], etc.
    pub asteroid_kernel: Option<PathBuf>,
}

impl SessionConfig {
    /// Use the analytical VSOP2013 + ELP/MPP02 backend. No kernel files
    /// required, but limited to **geometric** positions (no light-time,
    /// no aberration, no light deflection). Adequate for casual previews
    /// at the arcsec level; **not** what professional astrology should
    /// use as final output. For that, supply an SPK kernel.
    pub fn vsop2013() -> Self {
        Self {
            backend: SessionBackend::Vsop2013,
            asteroid_kernel: None,
        }
    }

    /// Use a JPL DE-series SPK planetary kernel. Required for full
    /// apparent positions (LT + aberration + light deflection + NPB).
    pub fn with_spk(planet_kernel: impl Into<PathBuf>) -> Self {
        Self {
            backend: SessionBackend::Spk {
                planet_kernel: planet_kernel.into(),
            },
            asteroid_kernel: None,
        }
    }

    /// Attach an asteroid SPK kernel. Builder-style; chain after
    /// [`SessionConfig::with_spk`].
    pub fn with_asteroid_kernel(mut self, path: impl Into<PathBuf>) -> Self {
        self.asteroid_kernel = Some(path.into());
        self
    }
}

/// Selectable ephemeris backend.
#[derive(Debug, Clone)]
pub enum SessionBackend {
    /// JPL DE-series planetary kernel (e.g. `de440.bsp`, `de441.bsp`).
    Spk { planet_kernel: PathBuf },
    /// VSOP2013 + ELP/MPP02 analytical theories. No external file.
    Vsop2013,
}

/// Geometric state in some frame, returned by
/// [`EphemerisSession::body_geometric`] for callers who need raw vectors.
#[derive(Debug, Clone, Copy)]
pub struct GeometricState {
    pub position_km: [f64; 3],
    pub velocity_km_per_s: [f64; 3],
}

pub struct EphemerisSession {
    oracle: Oracle,
    asteroid_spk: Option<SpkFile>,
    config: SessionConfig,
}

impl EphemerisSession {
    /// Open a session. For SPK backends this memory-maps the kernel file.
    pub fn open(config: SessionConfig) -> SkyResult<Self> {
        let backend = match &config.backend {
            SessionBackend::Spk { planet_kernel } => Backend::Spk {
                kernel_path: planet_kernel.clone(),
            },
            SessionBackend::Vsop2013 => Backend::Vsop2013,
        };
        let oracle = Oracle::new(backend)?;

        let asteroid_spk = match &config.asteroid_kernel {
            Some(path) => Some(open_spk(path)?),
            None => None,
        };

        Ok(Self {
            oracle,
            asteroid_spk,
            config,
        })
    }

    /// The configuration that produced this session.
    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    /// Low-level escape hatch. Use this when the façade does not yet
    /// expose the operation you need; behaviour is identical to invoking
    /// the underlying `Oracle` directly.
    pub fn oracle(&self) -> &Oracle {
        &self.oracle
    }

    /// Borrow the underlying SPK planetary kernel handle if the session
    /// was opened with one. Returns [`SkyError::SpkRequired`] otherwise.
    ///
    /// Used by higher-level crates (e.g. `eternal-astrology` for
    /// eclipses) that need direct access to the JPL kernel without
    /// reaching into the validation crate's `Oracle` API.
    pub fn require_spk(&self) -> SkyResult<&cosmos_ephemeris::jpl::SpkFile> {
        self.oracle.spk().ok_or(SkyError::SpkRequired)
    }

    /// Compute the apparent position of `body` at instant `t`, optionally
    /// reduced to a topocentric observer. The output is in the
    /// **true ecliptic and equator of date** (tropical frame).
    ///
    /// * `Body::Sun..Body::Pluto`: full apparent pipeline (LT + light
    ///   deflection + aberration + NPB) when the backend is SPK; just
    ///   geometric ICRF for the VSOP2013 backend.
    /// * `Body::MeanNode` / `Body::MeanLilith`: analytical (no SPK needed).
    /// * `Body::TrueNode` / `Body::TrueLilith`: require an SPK backend.
    /// * Asteroids: require both an SPK backend and an asteroid kernel
    ///   (see [`SessionConfig::with_asteroid_kernel`]).
    pub fn body_apparent(
        &self,
        body: Body,
        t: Instant,
        observer: Option<&Observer>,
    ) -> SkyResult<ApparentPosition> {
        match body.kind() {
            BodyKind::Major => self.major_apparent(body, t, observer),
            BodyKind::LunarPoint => self.lunar_point_apparent(body, t, observer),
            BodyKind::SmallBody => self.small_body_apparent(body, t, observer),
        }
    }

    /// Geometric (no corrections) state of `body` wrt `center` in the
    /// backend's native frame (ICRF for both current backends).
    pub fn body_geometric(
        &self,
        body: Body,
        center: Body,
        t: Instant,
    ) -> SkyResult<GeometricState> {
        let body_id = body.naif_id().ok_or(SkyError::UnsupportedBody {
            body,
            reason: "this body has no SPK representation; only analytical \
                     ecliptic longitude is defined",
        })?;
        let center_id = center.naif_id().ok_or(SkyError::UnsupportedBody {
            body: center,
            reason: "center body must have an SPK representation",
        })?;
        let jd_tdb = t.jd_tdb()?;
        let state = self.oracle.state(body_id, center_id, jd_tdb, Frame::Icrf)?;
        Ok(GeometricState {
            position_km: state.pos_km,
            velocity_km_per_s: state.vel_km_s,
        })
    }

    // ─── Major planets, Sun, Moon ──────────────────────────────────────

    fn major_apparent(
        &self,
        body: Body,
        t: Instant,
        observer: Option<&Observer>,
    ) -> SkyResult<ApparentPosition> {
        let naif = body.naif_id().expect("major bodies always have a NAIF ID");
        let jd_tdb = t.jd_tdb()?;
        let tt = t.tt()?;

        let geocentric_tet = match self.config.backend {
            SessionBackend::Spk { .. } => self.oracle.corrected_state(
                naif,
                /* center = Earth */ 399,
                jd_tdb,
                Frame::TrueEquatorEquinoxOfDate,
                Corrections::APPARENT,
            )?,
            SessionBackend::Vsop2013 => self.oracle.state(naif, 399, jd_tdb, Frame::Icrf)?,
        };

        let tet_pos = Vector3::new(
            geocentric_tet.pos_km[0],
            geocentric_tet.pos_km[1],
            geocentric_tet.pos_km[2],
        );
        let tet_vel = Vector3::new(
            geocentric_tet.vel_km_s[0],
            geocentric_tet.vel_km_s[1],
            geocentric_tet.vel_km_s[2],
        );

        let (equatorial, distance_km) = equatorial_from_tet(tet_pos);
        let (ecliptic, ecliptic_velocity) = ecliptic_from_tet(tet_pos, tet_vel, &tt);

        let ecliptic_coord = EclipticCoord {
            longitude_rad: ecliptic.0,
            latitude_rad: ecliptic.1,
            distance_km,
        };
        let equatorial_coord = EquatorialCoord {
            right_ascension_rad: equatorial.0,
            declination_rad: equatorial.1,
            distance_km,
        };

        let topocentric_horizon = match observer {
            Some(obs) => Some(self.topocentric_horizon(obs, &t, tet_pos)?),
            None => None,
        };

        Ok(ApparentPosition {
            ecliptic_of_date: ecliptic_coord,
            equatorial_of_date: equatorial_coord,
            topocentric_horizon,
            ecliptic_velocity,
        })
    }

    // ─── Lunar special points (nodes, Lilith) ──────────────────────────

    fn lunar_point_apparent(
        &self,
        body: Body,
        t: Instant,
        observer: Option<&Observer>,
    ) -> SkyResult<ApparentPosition> {
        let tt = t.tt()?;
        let jd_tdb = t.jd_tdb()?;

        let longitude_rad = match body {
            Body::MeanNode => cosmos_validation::lunar::mean_lunar_node(&tt),
            Body::MeanLilith => cosmos_validation::lunar::mean_lilith(&tt),
            Body::TrueNode | Body::TrueLilith => {
                let spk = self.oracle.spk().ok_or(SkyError::UnsupportedBody {
                    body,
                    reason: "true (osculating) lunar points require an SPK backend",
                })?;
                match body {
                    Body::TrueNode => {
                        cosmos_validation::lunar::true_lunar_node_geocentric(spk, &tt, jd_tdb)?
                    }
                    Body::TrueLilith => {
                        cosmos_validation::lunar::true_lilith_geocentric(spk, &tt, jd_tdb)?
                    }
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        };

        // By geometric definition both the lunar nodes (intersection of
        // the Moon's orbit with the ecliptic) and Lilith (a longitude on
        // the ecliptic ellipse's major axis) lie *on* the ecliptic, so
        // latitude is zero. They are conceptual points with no physical
        // distance; we leave that field at zero and skip topocentric
        // parallax — for a unit-sphere direction, geocentric ≡ topocentric.
        let lat_rad = 0.0;
        let ecliptic_coord = EclipticCoord {
            longitude_rad,
            latitude_rad: lat_rad,
            distance_km: 0.0,
        };
        let equatorial_coord = equatorial_from_ecliptic_angles(longitude_rad, lat_rad, &tt)?;
        let topocentric_horizon = match observer {
            // Treat the point as a unit-sphere direction: rebuild a
            // synthetic TET unit vector and reuse the horizon formula.
            Some(obs) => Some(self.lunar_point_horizon(longitude_rad, &t, obs)?),
            None => None,
        };

        Ok(ApparentPosition {
            ecliptic_of_date: ecliptic_coord,
            equatorial_of_date: equatorial_coord,
            topocentric_horizon,
            // Mean-point time-derivatives are well-defined (the polynomial
            // can be differentiated), but the value is dominated by the
            // mean-motion constant; we leave it at zero for now and
            // expose the rate via a follow-up if real consumers need it.
            ecliptic_velocity: EclipticVelocity::default(),
        })
    }

    fn lunar_point_horizon(
        &self,
        longitude_rad: f64,
        t: &Instant,
        observer: &Observer,
    ) -> SkyResult<HorizonCoord> {
        let tt = t.tt()?;
        // Unit vector in ecliptic-of-date with β = 0.
        let v_ecl = Vector3::new(libm::cos(longitude_rad), libm::sin(longitude_rad), 0.0);
        // Rotate ecliptic-of-date → TET equatorial (rotation about X by +ε).
        let eps_true = true_obliquity_iau2006a(&tt).map_err(|e| {
            SkyError::Ephemeris(OracleError::Inner(format!("obliquity: {}", e)))
        })?;
        let (sin_e, cos_e) = libm::sincos(eps_true);
        let v_tet = Vector3::new(
            v_ecl.x,
            v_ecl.y * cos_e - v_ecl.z * sin_e,
            v_ecl.y * sin_e + v_ecl.z * cos_e,
        );
        self.topocentric_horizon(observer, t, v_tet)
    }

    // ─── Asteroids ─────────────────────────────────────────────────────

    fn small_body_apparent(
        &self,
        body: Body,
        t: Instant,
        observer: Option<&Observer>,
    ) -> SkyResult<ApparentPosition> {
        let asteroid_spk = self.asteroid_spk.as_ref().ok_or(SkyError::UnsupportedBody {
            body,
            reason: "no asteroid SPK kernel was attached — \
                     use SessionConfig::with_asteroid_kernel(path)",
        })?;
        let planet_spk = self.oracle.spk().ok_or(SkyError::UnsupportedBody {
            body,
            reason: "asteroid computation requires the planet kernel \
                     (Earth + Sun positions); switch to an SPK backend",
        })?;
        let naif = body.naif_id().expect("asteroids always have a NAIF ID");
        let tt = t.tt()?;
        let jd_tdb = t.jd_tdb()?;

        let (lon, lat, dist_au) = cosmos_validation::asteroids::apparent_ecliptic_of_date(
            naif,
            asteroid_spk,
            planet_spk,
            &tt,
            jd_tdb,
        )?;
        let distance_km = dist_au * cosmos_core::constants::AU_KM;

        let ecliptic_coord = EclipticCoord {
            longitude_rad: lon,
            latitude_rad: lat,
            distance_km,
        };
        let equatorial_coord = equatorial_from_ecliptic_angles_with_distance(
            lon, lat, distance_km, &tt,
        )?;

        let topocentric_horizon = match observer {
            Some(obs) => {
                // Reconstruct a TET Cartesian for horizon reduction. The
                // re-rotation is bit-exact lossless: ecliptic→TET applies
                // the inverse of the same matrix we used inside
                // `asteroids::apparent_ecliptic_of_date`.
                let v_tet = tet_from_ecliptic(lon, lat, distance_km, &tt)?;
                Some(self.topocentric_horizon(obs, &t, v_tet)?)
            }
            None => None,
        };

        Ok(ApparentPosition {
            ecliptic_of_date: ecliptic_coord,
            equatorial_of_date: equatorial_coord,
            topocentric_horizon,
            // Apparent dλ/dt for asteroids requires a second SPK query
            // at t+δ to finite-difference; deferred until a real consumer
            // asks for it.
            ecliptic_velocity: EclipticVelocity::default(),
        })
    }

    // ─── Shared topocentric reduction ──────────────────────────────────

    fn topocentric_horizon(
        &self,
        observer: &Observer,
        t: &Instant,
        geocentric_tet_km: Vector3,
    ) -> SkyResult<HorizonCoord> {
        let tt = t.tt()?;
        let ut1 = t.ut1()?;

        let obs_tet =
            observer_position_tet_km(observer, &ut1, &tt).map_err(SkyError::Ephemeris)?;
        let topo = [
            geocentric_tet_km.x - obs_tet.x,
            geocentric_tet_km.y - obs_tet.y,
            geocentric_tet_km.z - obs_tet.z,
        ];

        use cosmos_core::Location;
        use cosmos_time::sidereal::GAST;
        let location = Location::from_degrees(
            observer.lat_rad.to_degrees(),
            observer.lon_rad.to_degrees(),
            observer.elev_m,
        )
        .map_err(|e| SkyError::Ephemeris(OracleError::Inner(format!("Location: {:?}", e))))?;
        let gast = GAST::from_ut1_and_tt(&ut1, &tt)
            .map_err(|e| SkyError::Ephemeris(OracleError::Inner(format!("GAST: {:?}", e))))?;
        let last_rad = gast.to_last(&location).angle().radians();

        let (alt, az) = alt_az_from_topocentric(topo, observer.lat_rad, last_rad);
        Ok(HorizonCoord {
            altitude_rad: alt,
            azimuth_rad: az,
        })
    }
}

// ─── Free helpers ──────────────────────────────────────────────────────

fn open_spk(path: &Path) -> SkyResult<SpkFile> {
    SpkFile::open(path).map_err(|e| {
        SkyError::Ephemeris(OracleError::KernelLoad(format!(
            "{}: {}",
            path.display(),
            e
        )))
    })
}

/// Decompose a TET-frame Cartesian into (RA, Dec, distance).
fn equatorial_from_tet(v: Vector3) -> ((f64, f64), f64) {
    let r = libm::sqrt(v.x * v.x + v.y * v.y + v.z * v.z);
    if r == 0.0 {
        return (((0.0, 0.0)), 0.0);
    }
    let dec = libm::asin(v.z / r);
    let ra = wrap_two_pi(libm::atan2(v.y, v.x));
    ((ra, dec), r)
}

/// Equatorial coordinates for a unit-direction ecliptic point (no
/// distance). Used by lunar mean/true points.
fn equatorial_from_ecliptic_angles(
    lon: f64,
    lat: f64,
    tt: &cosmos_time::TT,
) -> SkyResult<EquatorialCoord> {
    equatorial_from_ecliptic_angles_with_distance(lon, lat, 0.0, tt)
}

fn equatorial_from_ecliptic_angles_with_distance(
    lon: f64,
    lat: f64,
    distance_km: f64,
    tt: &cosmos_time::TT,
) -> SkyResult<EquatorialCoord> {
    let v_tet = tet_from_ecliptic(lon, lat, distance_km.max(1.0), tt)?;
    let dec = libm::asin(v_tet.z / libm::sqrt(v_tet.x * v_tet.x + v_tet.y * v_tet.y + v_tet.z * v_tet.z));
    let ra = wrap_two_pi(libm::atan2(v_tet.y, v_tet.x));
    Ok(EquatorialCoord {
        right_ascension_rad: ra,
        declination_rad: dec,
        distance_km,
    })
}

/// Rebuild a TET-equatorial Cartesian (km) from ecliptic-of-date (λ, β, r).
fn tet_from_ecliptic(
    lon: f64,
    lat: f64,
    distance_km: f64,
    tt: &cosmos_time::TT,
) -> SkyResult<Vector3> {
    let (sin_lon, cos_lon) = libm::sincos(lon);
    let (sin_lat, cos_lat) = libm::sincos(lat);
    let v_ecl = Vector3::new(
        distance_km * cos_lat * cos_lon,
        distance_km * cos_lat * sin_lon,
        distance_km * sin_lat,
    );
    // Rotate ecliptic-of-date → TET equatorial: rotation about X by +ε.
    let eps_true = true_obliquity_iau2006a(tt)
        .map_err(|e| SkyError::Ephemeris(OracleError::Inner(format!("obliquity: {}", e))))?;
    let (sin_e, cos_e) = libm::sincos(eps_true);
    Ok(Vector3::new(
        v_ecl.x,
        v_ecl.y * cos_e - v_ecl.z * sin_e,
        v_ecl.y * sin_e + v_ecl.z * cos_e,
    ))
}

/// Rotate position + velocity from TET equatorial to ecliptic of date,
/// then decompose the position into (lon, lat) and compute the
/// corresponding rate of motion in (lon, lat, distance).
fn ecliptic_from_tet(
    pos_tet: Vector3,
    vel_tet: Vector3,
    tt: &cosmos_time::TT,
) -> ((f64, f64), EclipticVelocity) {
    let pos_ecl = tet_equatorial_to_ecliptic_of_date(pos_tet, tt);
    let vel_ecl = tet_equatorial_to_ecliptic_of_date(vel_tet, tt);

    let r_xy_sq = pos_ecl.x * pos_ecl.x + pos_ecl.y * pos_ecl.y;
    let r_xy = libm::sqrt(r_xy_sq);
    let r = libm::sqrt(r_xy_sq + pos_ecl.z * pos_ecl.z);

    let lon = wrap_two_pi(libm::atan2(pos_ecl.y, pos_ecl.x));
    let lat = libm::atan2(pos_ecl.z, r_xy);

    const SECONDS_PER_DAY: f64 = 86_400.0;
    let velocity = if r_xy_sq == 0.0 || r == 0.0 {
        EclipticVelocity::default()
    } else {
        let xy_dot = pos_ecl.x * vel_ecl.x + pos_ecl.y * vel_ecl.y;
        let lon_rate_per_s = (pos_ecl.x * vel_ecl.y - pos_ecl.y * vel_ecl.x) / r_xy_sq;
        let lat_rate_per_s = (vel_ecl.z * r_xy - pos_ecl.z * (xy_dot / r_xy)) / (r * r);
        let radial_rate_per_s = (xy_dot + pos_ecl.z * vel_ecl.z) / r;
        EclipticVelocity {
            longitude_rate_rad_per_day: lon_rate_per_s * SECONDS_PER_DAY,
            latitude_rate_rad_per_day: lat_rate_per_s * SECONDS_PER_DAY,
            radial_rate_km_per_day: radial_rate_per_s * SECONDS_PER_DAY,
        }
    };

    ((lon, lat), velocity)
}
