//! Ground-truth fixture representation.
//!
//! A fixture is one state vector (position + velocity) for a NAIF body at a
//! given TDB Julian date, expressed in km and km/s in some reference frame,
//! plus enough provenance to know how skeptical to be of it.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Frame {
    /// J2000 / ICRF equatorial. SPK natively returns this.
    Icrf,
    /// J2000 ecliptic. VSOP2013 native frame.
    EclipticJ2000,
    /// True equator and equinox of date — the classical equinox-based
    /// frame that Horizons reports as "apparent RA/Dec".
    #[serde(rename = "TET")]
    TrueEquatorEquinoxOfDate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// External: JPL Horizons API.
    Horizons {
        ephemeris: String,
        fetched_at: String,
    },
    /// External: Swiss Ephemeris.
    SwissEphemeris { version: String },
    /// Self-baseline: produced by the current SPK reader of this very
    /// codebase. Useful as a smoke-test wiring before Horizons fixtures
    /// are fetched, but **never** counts as validation of correctness.
    SelfBaseline { kernel: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Tolerance {
    /// Maximum allowed position error in km.
    pub pos_km: f64,
    /// Maximum allowed velocity error in km/s.
    pub vel_km_s: f64,
}

impl Tolerance {
    /// Strict tolerance suitable for full JPL kernels vs Horizons.
    pub const SPK_STRICT: Self = Self {
        pos_km: 1.0e-3,    // 1 metre
        vel_km_s: 1.0e-9,  // 1 mm/s
    };

    /// Loose tolerance suitable for VSOP2013 / ELP analytical theories.
    pub const VSOP_LOOSE: Self = Self {
        pos_km: 5.0e1,     // 50 km
        vel_km_s: 1.0e-3,  // 1 m/s
    };

    /// SPK-vs-Horizons tolerance for the Moon-EMB and Earth-EMB segments.
    /// The DE440 kernel and the Horizons DE441 server differ in the lunar
    /// fit by an amount that propagates as up to ~2 mm of position error
    /// on Moon-wrt-EMB and ~25 µm on Earth-wrt-EMB (the Earth value
    /// scales by the Moon/Earth mass ratio). Velocity error sits at
    /// ~2 nm/s for the Moon. These are *cross-version* gates: the local
    /// reader is correct; the kernels themselves diverge slightly.
    pub const SPK_LUNAR_CROSS_VERSION: Self = Self {
        pos_km: 1.0e-2,    // 10 m — covers the 2 mm peak with margin
        vel_km_s: 1.0e-8,  // 10 nm/s
    };

    /// Realistic per-body regression budget for the *currently embedded*
    /// VSOP2013 truncation in eternal-ephemeris. These are not accuracy
    /// claims — they are observed-baseline-plus-buffer values that catch
    /// drift in the analytical backend without forcing a full series
    /// re-embedding to make the suite pass.
    ///
    /// Phase 1 of the v1 roadmap is expected to tighten these (or remove
    /// the analytic backend in favour of SPK for high-precision work).
    pub fn vsop_baseline_for(naif_body: i32) -> Self {
        match naif_body {
            // Mercury, Venus, EMB, Mars and the geocentric Sun.
            1 | 2 | 3 | 4 | 10 => Self {
                pos_km: 2.5e2,
                vel_km_s: 5.0e-4,
            },
            // Jupiter.
            5 => Self {
                pos_km: 7.0e2,
                vel_km_s: 5.0e-4,
            },
            // Saturn.
            6 => Self {
                pos_km: 3.0e3,
                vel_km_s: 1.0e-3,
            },
            // Uranus, Neptune, Pluto.
            7 | 8 | 9 => Self {
                pos_km: 1.0e4,
                vel_km_s: 2.0e-3,
            },
            // ELP/MPP02 Moon is the cleanest theory in the analytic stack.
            301 => Self {
                pos_km: 2.0,
                vel_km_s: 5.0e-6,
            },
            // Catch-all.
            _ => Self::VSOP_LOOSE,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub name: String,
    /// NAIF integer ID of the target body.
    pub body: i32,
    /// NAIF integer ID of the centre body (0 = SSB, 10 = Sun, 399 = Earth, ...).
    pub center: i32,
    /// Reference epoch as a Julian Date in the TDB time scale.
    pub jd_tdb: f64,
    pub frame: Frame,
    /// Position vector, km.
    pub pos_km: [f64; 3],
    /// Velocity vector, km/s.
    pub vel_km_s: [f64; 3],
    pub source: Source,
    pub tolerance: Tolerance,
}

/// Which backend a fixture set is intended to gate. Each set tests one
/// backend, so the file declares it explicitly.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// JPL SPK kernel reader. Requires a kernel path at test time.
    #[default]
    Spk,
    /// VSOP2013 planetary theory + ELP/MPP02 lunar theory. No kernel.
    Vsop2013,
}

/// Which IAU-style corrections the oracle should apply before comparing
/// to a fixture's reference state. Each flag corresponds to a stage of
/// the apparent-position pipeline; missing flags mean "skip that stage".
///
/// Maps to Horizons `VEC_CORR` (or `APPARENT`) options when fetching:
///   * `{}`                                       → `VEC_CORR='NONE'` (geometric)
///   * `{light_time}`                             → `VEC_CORR='LT'` (astrometric)
///   * `{light_time, stellar_aberration}`         → `VEC_CORR='LT+S'` (apparent vectors)
///   * `{light_time, stellar_aberration,
///       gravitational_deflection}`               → spherical OBSERVER apparent
///
/// Default is no corrections (geometric).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Corrections {
    #[serde(default)]
    pub light_time: bool,
    #[serde(default)]
    pub stellar_aberration: bool,
    #[serde(default)]
    pub gravitational_deflection: bool,
}

impl Corrections {
    pub const GEOMETRIC: Self = Self {
        light_time: false,
        stellar_aberration: false,
        gravitational_deflection: false,
    };
    pub const ASTROMETRIC: Self = Self {
        light_time: true,
        stellar_aberration: false,
        gravitational_deflection: false,
    };
    pub const APPARENT_VECTOR: Self = Self {
        light_time: true,
        stellar_aberration: true,
        gravitational_deflection: false,
    };
    /// Full apparent-direction corrections: LT + stellar aberration +
    /// gravitational light deflection. Combined with `Frame::TrueEquator
    /// EquinoxOfDate` this matches Horizons OBSERVER apparent RA/Dec.
    pub const APPARENT: Self = Self {
        light_time: true,
        stellar_aberration: true,
        gravitational_deflection: true,
    };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureSet {
    pub description: String,
    #[serde(default)]
    pub backend: BackendKind,
    /// Corrections applied (by the ephemeris source AND expected from the
    /// local oracle) when producing/comparing this set. Empty = geometric.
    #[serde(default)]
    pub corrections: Corrections,
    pub fixtures: Vec<Fixture>,
}

impl FixtureSet {
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path.as_ref())?;
        let set: Self = serde_json::from_str(&raw)?;
        Ok(set)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> anyhow::Result<()> {
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(path.as_ref(), raw)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_roundtrip() {
        let f = Fixture {
            name: "test".into(),
            body: 399,
            center: 0,
            jd_tdb: 2451545.0,
            frame: Frame::Icrf,
            pos_km: [1.0, 2.0, 3.0],
            vel_km_s: [0.1, 0.2, 0.3],
            source: Source::SelfBaseline {
                kernel: "de432s.bsp".into(),
            },
            tolerance: Tolerance::SPK_STRICT,
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: Fixture = serde_json::from_str(&json).unwrap();
        assert_eq!(back.body, 399);
        assert_eq!(back.frame, Frame::Icrf);
    }
}
