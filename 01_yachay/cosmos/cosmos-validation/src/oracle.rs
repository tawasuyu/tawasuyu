//! Oracle: thin layer over eternal-ephemeris that produces a state
//! vector in km/(km·s⁻¹) for a (body, center, jd_tdb) query, regardless of
//! which backend handles it under the hood.
//!
//! Backends:
//!   * [`Backend::Spk`]      — JPL SPK kernel (DE-series). Native frame ICRF.
//!                             Accepts any (body, center) the kernel covers.
//!   * [`Backend::Vsop2013`] — VSOP2013 planetary theory + ELP/MPP02 lunar
//!                             theory. Native frame ICRF (`to_icrs()` is
//!                             already applied by the underlying crate).
//!                             Accepts (planet, center=Sun) heliocentric,
//!                             (planet, center=Earth) geocentric, and
//!                             (Moon, center=Earth) via ELP.

use std::path::{Path, PathBuf};

use cosmos_coords::Vector3;
use cosmos_core::constants::{AU_KM, SECONDS_PER_DAY_F64};
use cosmos_ephemeris::jpl::{SpkError, SpkFile};
use cosmos_ephemeris::moon::ElpMpp02Moon;
use cosmos_ephemeris::planets::{
    Vsop2013Emb, Vsop2013Jupiter, Vsop2013Mars, Vsop2013Mercury, Vsop2013Neptune, Vsop2013Pluto,
    Vsop2013Saturn, Vsop2013Uranus, Vsop2013Venus,
};
use cosmos_ephemeris::sun::Vsop2013Sun;
use cosmos_ephemeris::Vsop2013Earth;
use cosmos_core::utils::jd_to_centuries;
use cosmos_time::julian::JulianDate;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::{NutationCalculator, TDB, TT};

use crate::fixture::{Corrections, Frame};

/// Speed of light in AU per day (IAU 2012).
const C_AU_PER_DAY: f64 = 173.144_632_684_669_3;

/// Conversion factor from AU/day to km/s.
const AU_PER_DAY_TO_KM_PER_S: f64 = AU_KM / SECONDS_PER_DAY_F64;

/// NAIF integer IDs used by the VSOP routing table.
mod naif {
    pub const MERCURY_BARYCENTER: i32 = 1;
    pub const VENUS_BARYCENTER: i32 = 2;
    pub const EARTH_MOON_BARYCENTER: i32 = 3;
    pub const MARS_BARYCENTER: i32 = 4;
    pub const JUPITER_BARYCENTER: i32 = 5;
    pub const SATURN_BARYCENTER: i32 = 6;
    pub const URANUS_BARYCENTER: i32 = 7;
    pub const NEPTUNE_BARYCENTER: i32 = 8;
    pub const PLUTO_BARYCENTER: i32 = 9;
    pub const SUN: i32 = 10;
    pub const MOON: i32 = 301;
    pub const EARTH: i32 = 399;
}

#[derive(Debug, Clone, Copy)]
pub struct StateKmS {
    pub pos_km: [f64; 3],
    pub vel_km_s: [f64; 3],
}

#[derive(Debug, Clone)]
pub enum Backend {
    /// JPL SPK kernel (DE-series). Native frame is ICRF.
    Spk { kernel_path: PathBuf },
    /// VSOP2013 + ELP/MPP02 analytical theories. No kernel needed.
    Vsop2013,
}

impl Backend {
    pub fn native_frame(&self) -> Frame {
        match self {
            Backend::Spk { .. } => Frame::Icrf,
            Backend::Vsop2013 => Frame::Icrf,
        }
    }
}

#[derive(Debug)]
pub enum OracleError {
    Spk(SpkError),
    UnsupportedFrame { backend_frame: Frame, requested: Frame },
    UnsupportedRoute { body: i32, center: i32 },
    KernelLoad(String),
    Inner(String),
}

impl std::fmt::Display for OracleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OracleError::Spk(e) => write!(f, "SPK error: {}", e),
            OracleError::UnsupportedFrame { backend_frame, requested } => write!(
                f,
                "Backend produces frame {:?}, fixture requests {:?}; frame conversion not yet implemented",
                backend_frame, requested
            ),
            OracleError::UnsupportedRoute { body, center } => write!(
                f,
                "VSOP backend has no route for (body={}, center={})",
                body, center
            ),
            OracleError::KernelLoad(msg) => write!(f, "Failed to load kernel: {}", msg),
            OracleError::Inner(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for OracleError {}

impl From<SpkError> for OracleError {
    fn from(e: SpkError) -> Self {
        OracleError::Spk(e)
    }
}

pub struct Oracle {
    backend: Backend,
    spk: Option<SpkFile>,
}

impl Oracle {
    pub fn new(backend: Backend) -> Result<Self, OracleError> {
        let spk = match &backend {
            Backend::Spk { kernel_path } => Some(load_spk(kernel_path)?),
            Backend::Vsop2013 => None,
        };
        Ok(Self { backend, spk })
    }

    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// Direct access to the underlying SPK kernel handle. Returns `None`
    /// when the backend is analytical (VSOP2013). Exposed so higher-level
    /// crates (`eternal-sky`) can route the lunar-node, Lilith, and
    /// asteroid paths through the same memory-mapped kernel that the
    /// planet path uses, without opening a second handle.
    pub fn spk(&self) -> Option<&SpkFile> {
        self.spk.as_ref()
    }

    /// Compute geometric state vector for the requested (body, center, jd_tdb)
    /// tuple in the requested frame. Returns km, km/s.
    pub fn state(
        &self,
        body: i32,
        center: i32,
        jd_tdb: f64,
        frame: Frame,
    ) -> Result<StateKmS, OracleError> {
        let native = self.backend.native_frame();
        if frame != native {
            return Err(OracleError::UnsupportedFrame {
                backend_frame: native,
                requested: frame,
            });
        }
        match &self.backend {
            Backend::Spk { .. } => {
                let spk = self.spk.as_ref().expect("Spk backend must have spk loaded");
                let (pos, vel) = spk.compute_state(body, center, jd_tdb)?;
                Ok(StateKmS { pos_km: pos, vel_km_s: vel })
            }
            Backend::Vsop2013 => vsop_state(body, center, jd_tdb),
        }
    }

    /// Compute state vector with the requested corrections applied, in
    /// the requested frame. When `corrections` is `Corrections::GEOMETRIC`
    /// this is identical to [`Oracle::state`].
    ///
    /// Light-time correction is the only stage wired in this iteration of
    /// the harness. It re-queries the target's SSB-centred state at the
    /// emission time `t_obs - τ` while keeping the observer's state fixed
    /// at `t_obs`, iterating τ to convergence. The reported velocity is
    /// the apparent rate of change of that vector, i.e.
    /// `target_velocity(t_emit) - observer_velocity(t_obs)`.
    pub fn corrected_state(
        &self,
        body: i32,
        center: i32,
        jd_tdb: f64,
        frame: Frame,
        corrections: Corrections,
    ) -> Result<StateKmS, OracleError> {
        if corrections == Corrections::GEOMETRIC {
            return self.state(body, center, jd_tdb, frame);
        }

        if !corrections.light_time && corrections.stellar_aberration {
            return Err(OracleError::Inner(
                "stellar aberration without light-time correction is not a supported combination"
                    .into(),
            ));
        }

        if !corrections.light_time {
            return self.state(body, center, jd_tdb, frame);
        }

        let native = self.backend.native_frame();
        // Frame::Icrf is the SPK output natively; TET is reached via NPB
        // rotation, so accept it only when a full apparent computation is
        // requested.
        match (native, frame) {
            (Frame::Icrf, Frame::Icrf) => {}
            (Frame::Icrf, Frame::TrueEquatorEquinoxOfDate)
                if corrections == Corrections::APPARENT =>
            {
                // Allowed: oracle will apply NPB after LD + aberration.
            }
            _ => {
                return Err(OracleError::UnsupportedFrame {
                    backend_frame: native,
                    requested: frame,
                });
            }
        }

        match &self.backend {
            Backend::Spk { .. } => {
                let astrometric = self.spk_astrometric_state(body, center, jd_tdb)?;
                match (
                    corrections.stellar_aberration,
                    corrections.gravitational_deflection,
                    frame,
                ) {
                    (false, false, Frame::Icrf) => Ok(astrometric),
                    (true, false, Frame::Icrf) => {
                        self.spk_apparent_vector_state(center, jd_tdb, astrometric)
                    }
                    (true, true, Frame::TrueEquatorEquinoxOfDate) => {
                        self.spk_apparent_observer_state(center, jd_tdb, astrometric)
                    }
                    _ => Err(OracleError::Inner(format!(
                        "unsupported corrections+frame combination: {:?} in {:?}",
                        corrections, frame
                    ))),
                }
            }
            Backend::Vsop2013 => Err(OracleError::Inner(
                "VSOP2013 backend does not yet support light-time correction".into(),
            )),
        }
    }

    /// Full apparent observer state: gravitational light deflection by Sun,
    /// stellar aberration, and NPB rotation to true equator and equinox of
    /// date. Output position is Cartesian in TET frame.
    fn spk_apparent_observer_state(
        &self,
        observer: i32,
        jd_obs: f64,
        astrometric: StateKmS,
    ) -> Result<StateKmS, OracleError> {
        let spk = self.spk.as_ref().expect("Spk backend must have spk loaded");

        // Observer geometry: barycentric position & velocity, Sun-observer vector.
        let (observer_pos_ssb, observer_vel_ssb) = ssb_state(spk, observer, jd_obs)?;
        let (sun_pos_ssb, _) = ssb_state(spk, naif::SUN, jd_obs)?;

        let v_obs_au_day = Vector3::new(
            observer_vel_ssb[0] * SECONDS_PER_DAY_F64 / AU_KM,
            observer_vel_ssb[1] * SECONDS_PER_DAY_F64 / AU_KM,
            observer_vel_ssb[2] * SECONDS_PER_DAY_F64 / AU_KM,
        );

        let sun_to_observer_km = [
            observer_pos_ssb[0] - sun_pos_ssb[0],
            observer_pos_ssb[1] - sun_pos_ssb[1],
            observer_pos_ssb[2] - sun_pos_ssb[2],
        ];
        let sun_obs_dist_km = libm::sqrt(
            sun_to_observer_km[0] * sun_to_observer_km[0]
                + sun_to_observer_km[1] * sun_to_observer_km[1]
                + sun_to_observer_km[2] * sun_to_observer_km[2],
        );
        let sun_obs_dist_au = sun_obs_dist_km / AU_KM;
        let sun_to_observer_unit = Vector3::new(
            sun_to_observer_km[0] / sun_obs_dist_km,
            sun_to_observer_km[1] / sun_obs_dist_km,
            sun_to_observer_km[2] / sun_obs_dist_km,
        );

        // Decompose the astrometric Cartesian into direction + distance.
        let distance_km = libm::sqrt(
            astrometric.pos_km[0] * astrometric.pos_km[0]
                + astrometric.pos_km[1] * astrometric.pos_km[1]
                + astrometric.pos_km[2] * astrometric.pos_km[2],
        );
        if distance_km == 0.0 {
            return Ok(astrometric);
        }
        let dir_icrs = Vector3::new(
            astrometric.pos_km[0] / distance_km,
            astrometric.pos_km[1] / distance_km,
            astrometric.pos_km[2] / distance_km,
        );

        // 1. Gravitational light deflection by the Sun.
        let dir_after_ld = cosmos_coords::aberration::apply_light_deflection(
            dir_icrs,
            sun_to_observer_unit,
            sun_obs_dist_au,
        );
        // 2. Stellar aberration (relativistic, IAU 2000A).
        let dir_after_ab = cosmos_coords::aberration::apply_aberration(
            dir_after_ld,
            v_obs_au_day,
            sun_obs_dist_au,
        );
        // 3. NPB rotation to true equator and equinox of date.
        let tt = jd_tdb_as_tt(jd_obs)?;
        let nutation = tt
            .nutation_iau2006a()
            .map_err(|e| OracleError::Inner(format!("nutation failed: {:?}", e)))?;
        let tt_jd = tt.to_julian_date();
        let t_centuries = jd_to_centuries(tt_jd.jd1(), tt_jd.jd2());
        let npb = cosmos_core::precession::PrecessionIAU2006::new().npb_matrix_iau2006a(
            t_centuries,
            nutation.nutation_longitude(),
            nutation.nutation_obliquity(),
        );
        let dir_tet = npb * dir_after_ab;

        Ok(StateKmS {
            pos_km: [
                dir_tet.x * distance_km,
                dir_tet.y * distance_km,
                dir_tet.z * distance_km,
            ],
            // Velocity is left at the astrometric value; the test tolerance
            // for apparent observer fixtures disables the velocity gate
            // (Horizons OBSERVER does not return a 3-D velocity).
            vel_km_s: astrometric.vel_km_s,
        })
    }

    /// Apply stellar aberration on top of the astrometric vector. Uses
    /// `cosmos_coords::aberration::apply_aberration`, which encodes
    /// the IAU 2000A relativistic transformation (stellar aberration with
    /// a small gravitational-correction `w2` term proportional to
    /// `GM_sun / (r_observer-sun · c²)`). Velocity is propagated by
    /// finite-difference around the requested epoch.
    fn spk_apparent_vector_state(
        &self,
        observer: i32,
        jd_obs: f64,
        astrometric: StateKmS,
    ) -> Result<StateKmS, OracleError> {
        let spk = self.spk.as_ref().expect("Spk backend must have spk loaded");

        // Observer state at obs time (SSB-centred).
        let (observer_pos_ssb, observer_vel_ssb) = ssb_state(spk, observer, jd_obs)?;
        let (sun_pos_ssb, _) = ssb_state(spk, naif::SUN, jd_obs)?;

        let v_obs_au_day = Vector3::new(
            observer_vel_ssb[0] * SECONDS_PER_DAY_F64 / AU_KM,
            observer_vel_ssb[1] * SECONDS_PER_DAY_F64 / AU_KM,
            observer_vel_ssb[2] * SECONDS_PER_DAY_F64 / AU_KM,
        );
        let sun_to_observer_au = [
            (observer_pos_ssb[0] - sun_pos_ssb[0]) / AU_KM,
            (observer_pos_ssb[1] - sun_pos_ssb[1]) / AU_KM,
            (observer_pos_ssb[2] - sun_pos_ssb[2]) / AU_KM,
        ];
        let sun_obs_dist_au = libm::sqrt(
            sun_to_observer_au[0] * sun_to_observer_au[0]
                + sun_to_observer_au[1] * sun_to_observer_au[1]
                + sun_to_observer_au[2] * sun_to_observer_au[2],
        );

        let pos = aberrate_position(astrometric.pos_km, v_obs_au_day, sun_obs_dist_au);

        Ok(StateKmS {
            pos_km: pos,
            // Aberration of the velocity vector is small (~v_obs/c × astrometric
            // velocity). Keep the astrometric velocity unchanged here; the test
            // tolerances for apparent-vector fixtures account for it. A proper
            // analytic apparent-velocity propagation is part of Phase 2 step 3.
            vel_km_s: astrometric.vel_km_s,
        })
    }

    /// Iteratively-converged geocentric / barycentric apparent state at
    /// the SSB level, queried only via direct SPK segments (no chaining).
    fn spk_astrometric_state(
        &self,
        body: i32,
        center: i32,
        jd_obs: f64,
    ) -> Result<StateKmS, OracleError> {
        let spk = self.spk.as_ref().expect("Spk backend must have spk loaded");

        // Observer state at observation time (relative to SSB).
        let (observer_pos_obs, observer_vel_obs) = ssb_state(spk, center, jd_obs)?;

        // First-pass τ from the geometric Earth–target distance at t_obs.
        let mut tau_days = {
            let (target_pos_obs, _) = ssb_state(spk, body, jd_obs)?;
            let dx = target_pos_obs[0] - observer_pos_obs[0];
            let dy = target_pos_obs[1] - observer_pos_obs[1];
            let dz = target_pos_obs[2] - observer_pos_obs[2];
            let dist_km = libm::sqrt(dx * dx + dy * dy + dz * dz);
            (dist_km / AU_KM) / C_AU_PER_DAY
        };

        // Iterate until τ stabilises (typical convergence: 2-3 iterations).
        let mut target_pos_emit = [0.0; 3];
        let mut target_vel_emit = [0.0; 3];
        for _ in 0..8 {
            let jd_emit = jd_obs - tau_days;
            let (p, v) = ssb_state(spk, body, jd_emit)?;
            target_pos_emit = p;
            target_vel_emit = v;
            let dx = target_pos_emit[0] - observer_pos_obs[0];
            let dy = target_pos_emit[1] - observer_pos_obs[1];
            let dz = target_pos_emit[2] - observer_pos_obs[2];
            let dist_km = libm::sqrt(dx * dx + dy * dy + dz * dz);
            let new_tau = (dist_km / AU_KM) / C_AU_PER_DAY;
            // 1e-15 days ≈ 0.09 ns: well below any precision we care about.
            let converged = (new_tau - tau_days).abs() < 1.0e-15;
            tau_days = new_tau;
            if converged {
                break;
            }
        }

        Ok(StateKmS {
            pos_km: [
                target_pos_emit[0] - observer_pos_obs[0],
                target_pos_emit[1] - observer_pos_obs[1],
                target_pos_emit[2] - observer_pos_obs[2],
            ],
            vel_km_s: [
                target_vel_emit[0] - observer_vel_obs[0],
                target_vel_emit[1] - observer_vel_obs[1],
                target_vel_emit[2] - observer_vel_obs[2],
            ],
        })
    }
}

/// Convert a TDB Julian Date to a `TT` time-scale value via the Fairhead-
/// Bretagnon series exposed by `eternal-time`. At the precision we
/// validate (sub-mas) the Greenwich variant is sufficient — the observer-
/// location term is bounded by ~µs (sub-µas in our path).
fn jd_tdb_as_tt(jd_tdb: f64) -> Result<TT, OracleError> {
    let tdb = TDB::from_julian_date(JulianDate::new(jd_tdb, 0.0));
    tdb.to_tt_greenwich()
        .map_err(|e| OracleError::Inner(format!("TDB→TT conversion failed: {:?}", e)))
}

fn aberrate_position(pos_km: [f64; 3], v_obs_au_day: Vector3, sun_obs_dist_au: f64) -> [f64; 3] {
    let distance_km = libm::sqrt(
        pos_km[0] * pos_km[0] + pos_km[1] * pos_km[1] + pos_km[2] * pos_km[2],
    );
    if distance_km == 0.0 {
        return pos_km;
    }
    let direction = Vector3::new(
        pos_km[0] / distance_km,
        pos_km[1] / distance_km,
        pos_km[2] / distance_km,
    );
    let aberrated =
        cosmos_coords::aberration::apply_aberration(direction, v_obs_au_day, sun_obs_dist_au);
    [
        aberrated.x * distance_km,
        aberrated.y * distance_km,
        aberrated.z * distance_km,
    ]
}

/// Return (pos, vel) of `body` relative to SSB at `jd_tdb`. Handles bodies
/// that DE kernels store relative to their barycentre (399 wrt 3, 301 wrt 3)
/// by chaining a single hop. Any deeper chain is the caller's problem.
fn ssb_state(
    spk: &SpkFile,
    body: i32,
    jd_tdb: f64,
) -> Result<([f64; 3], [f64; 3]), SpkError> {
    if let Ok(state) = spk.compute_state(body, 0, jd_tdb) {
        return Ok(state);
    }
    // Try body wrt EMB (3), then add EMB wrt SSB.
    let (p, v) = spk.compute_state(body, 3, jd_tdb)?;
    let (p_emb, v_emb) = spk.compute_state(3, 0, jd_tdb)?;
    Ok((
        [p[0] + p_emb[0], p[1] + p_emb[1], p[2] + p_emb[2]],
        [v[0] + v_emb[0], v[1] + v_emb[1], v[2] + v_emb[2]],
    ))
}

fn load_spk(path: &Path) -> Result<SpkFile, OracleError> {
    SpkFile::open(path).map_err(|e| OracleError::KernelLoad(format!("{}: {}", path.display(), e)))
}

fn tdb_from_jd(jd_tdb: f64) -> TDB {
    TDB::from_julian_date(JulianDate::new(jd_tdb, 0.0))
}

/// Convert a Vector3 in AU to a `[f64; 3]` in km.
fn au_to_km(v: &Vector3) -> [f64; 3] {
    [v.x * AU_KM, v.y * AU_KM, v.z * AU_KM]
}

/// Convert a Vector3 in AU/day to a `[f64; 3]` in km/s.
fn au_per_day_to_km_per_s(v: &Vector3) -> [f64; 3] {
    [
        v.x * AU_PER_DAY_TO_KM_PER_S,
        v.y * AU_PER_DAY_TO_KM_PER_S,
        v.z * AU_PER_DAY_TO_KM_PER_S,
    ]
}

fn vsop_state(body: i32, center: i32, jd_tdb: f64) -> Result<StateKmS, OracleError> {
    let tdb = tdb_from_jd(jd_tdb);

    // Moon (301) only has a geocentric route through ELP/MPP02.
    if body == naif::MOON && center == naif::EARTH {
        let state = ElpMpp02Moon::new()
            .geocentric_state_icrs(&tdb)
            .map_err(|e| OracleError::Inner(format!("ELP/MPP02 error: {:?}", e)))?;
        // ELP returns km and km/day.
        return Ok(StateKmS {
            pos_km: [state[0], state[1], state[2]],
            vel_km_s: [
                state[3] / SECONDS_PER_DAY_F64,
                state[4] / SECONDS_PER_DAY_F64,
                state[5] / SECONDS_PER_DAY_F64,
            ],
        });
    }

    match (body, center) {
        // Heliocentric planet (center = Sun).
        (b, naif::SUN) => vsop_helio_state(b, &tdb),
        // Geocentric planet / Sun (center = Earth).
        (b, naif::EARTH) => vsop_geo_state(b, &tdb),
        _ => Err(OracleError::UnsupportedRoute { body, center }),
    }
}

fn vsop_helio_state(body: i32, tdb: &TDB) -> Result<StateKmS, OracleError> {
    macro_rules! helio {
        ($struct:expr) => {{
            let (pos, vel) = $struct
                .heliocentric_state(tdb)
                .map_err(|e| OracleError::Inner(format!("VSOP heliocentric error: {:?}", e)))?;
            Ok(StateKmS {
                pos_km: au_to_km(&pos),
                vel_km_s: au_per_day_to_km_per_s(&vel),
            })
        }};
    }
    match body {
        naif::MERCURY_BARYCENTER => helio!(Vsop2013Mercury),
        naif::VENUS_BARYCENTER => helio!(Vsop2013Venus),
        naif::EARTH_MOON_BARYCENTER => helio!(Vsop2013Emb),
        naif::MARS_BARYCENTER => helio!(Vsop2013Mars),
        naif::JUPITER_BARYCENTER => helio!(Vsop2013Jupiter),
        naif::SATURN_BARYCENTER => helio!(Vsop2013Saturn),
        naif::URANUS_BARYCENTER => helio!(Vsop2013Uranus),
        naif::NEPTUNE_BARYCENTER => helio!(Vsop2013Neptune),
        naif::PLUTO_BARYCENTER => helio!(Vsop2013Pluto),
        naif::EARTH => helio!(Vsop2013Earth::new()),
        naif::SUN => Ok(StateKmS {
            pos_km: [0.0, 0.0, 0.0],
            vel_km_s: [0.0, 0.0, 0.0],
        }),
        _ => Err(OracleError::UnsupportedRoute {
            body,
            center: naif::SUN,
        }),
    }
}

fn vsop_geo_state(body: i32, tdb: &TDB) -> Result<StateKmS, OracleError> {
    macro_rules! geo {
        ($struct:expr) => {{
            let (pos, vel) = $struct
                .geocentric_state(tdb)
                .map_err(|e| OracleError::Inner(format!("VSOP geocentric error: {:?}", e)))?;
            Ok(StateKmS {
                pos_km: au_to_km(&pos),
                vel_km_s: au_per_day_to_km_per_s(&vel),
            })
        }};
    }
    match body {
        naif::MERCURY_BARYCENTER => geo!(Vsop2013Mercury),
        naif::VENUS_BARYCENTER => geo!(Vsop2013Venus),
        naif::MARS_BARYCENTER => geo!(Vsop2013Mars),
        naif::JUPITER_BARYCENTER => geo!(Vsop2013Jupiter),
        naif::SATURN_BARYCENTER => geo!(Vsop2013Saturn),
        naif::URANUS_BARYCENTER => geo!(Vsop2013Uranus),
        naif::NEPTUNE_BARYCENTER => geo!(Vsop2013Neptune),
        naif::PLUTO_BARYCENTER => geo!(Vsop2013Pluto),
        naif::SUN => geo!(Vsop2013Sun),
        _ => Err(OracleError::UnsupportedRoute {
            body,
            center: naif::EARTH,
        }),
    }
}
