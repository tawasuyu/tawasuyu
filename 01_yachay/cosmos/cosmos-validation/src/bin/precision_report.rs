//! Precision-report CLI.
//!
//! Run modes:
//!   * `report`    – evaluate every fixture in a file and print a table.
//!                   The fixture set declares its backend; for SPK sets
//!                   `--spk` is required.
//!   * `bootstrap` – write a self-baseline fixture set by querying the
//!                   current SPK backend. Useful to verify wiring before
//!                   real Horizons fetches; **not** a correctness check.
//!   * `fetch`     – (requires `--features fetch`) query JPL Horizons for
//!                   a curated body × epoch grid and write a real fixture
//!                   set ready for regression use. `--backend` selects
//!                   which grid (SSB-centred for SPK, Sun- and Earth-
//!                   centred for VSOP).

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use cosmos_validation::fixture::{
    BackendKind, Corrections, Fixture, FixtureSet, Frame, Source, Tolerance,
};
use cosmos_validation::oracle::{Backend, Oracle};
use cosmos_validation::report::{ErrorReport, ReportTable};

#[derive(Parser)]
#[command(version, about = "eternal-ephemeris precision thermometer")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum BackendArg {
    Spk,
    Vsop,
    /// SPK with light-time correction (astrometric vectors).
    SpkAstrometric,
    /// SPK with light-time + stellar aberration (apparent vectors).
    SpkApparentVector,
    /// SPK with light-time correction, fixtures sourced from Horizons OBSERVER
    /// mode (astrometric J2000 RA/Dec → Cartesian; velocity unchecked).
    SpkObserverAstrometric,
    /// SPK with full apparent corrections (light-time + stellar aberration
    /// + gravitational deflection + precession + nutation). Fixtures sourced
    /// from Horizons OBSERVER QUANTITIES='2,20' (apparent RA/Dec, true
    /// equator and equinox of date).
    SpkObserverApparent,
}

impl From<BackendArg> for BackendKind {
    fn from(a: BackendArg) -> Self {
        match a {
            BackendArg::Spk
            | BackendArg::SpkAstrometric
            | BackendArg::SpkApparentVector
            | BackendArg::SpkObserverAstrometric
            | BackendArg::SpkObserverApparent => BackendKind::Spk,
            BackendArg::Vsop => BackendKind::Vsop2013,
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Compare every fixture against the current backend output.
    Report {
        /// Path to JPL SPK kernel — required when the fixture set's backend is `spk`.
        #[arg(long)]
        spk: Option<PathBuf>,
        /// Path to fixtures JSON file.
        #[arg(long, default_value = "eternal-validation/fixtures/regression-de432/self_baseline.json")]
        fixtures: PathBuf,
    },
    /// Write a self-baseline fixture set from the current SPK output.
    /// These fixtures detect regression of the local code against itself —
    /// they are NOT external validation.
    Bootstrap {
        /// Path to JPL SPK kernel.
        #[arg(long)]
        spk: PathBuf,
        /// Output fixture file.
        #[arg(long, default_value = "eternal-validation/fixtures/regression-de432/self_baseline.json")]
        out: PathBuf,
    },
    /// Fetch fixtures from JPL Horizons for a curated body × epoch grid.
    #[cfg(feature = "fetch")]
    Fetch {
        /// Which backend the resulting fixtures will gate.
        #[arg(long, value_enum, default_value_t = BackendArg::Spk)]
        backend: BackendArg,
        /// Output fixture file. Default depends on backend.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

struct GridPoint {
    name: String,
    body: i32,
    center: i32,
    jd_tdb: f64,
}

/// SPK backend grid: planet barycenters wrt SSB plus the Earth/Moon
/// split wrt the Earth-Moon barycenter. The Earth/Moon entries need a
/// DE440-class kernel (de432s.bsp does not ship the 399 and 301 wrt 3
/// segments), so the produced fixture set is gated under
/// `regression-de440/`.
fn spk_grid() -> Vec<GridPoint> {
    let ssb_bodies: &[(i32, &str)] = &[
        (1, "Mercury barycenter"),
        (2, "Venus barycenter"),
        (3, "Earth-Moon barycenter"),
        (4, "Mars barycenter"),
        (5, "Jupiter barycenter"),
        (6, "Saturn barycenter"),
        (7, "Uranus barycenter"),
        (8, "Neptune barycenter"),
        (10, "Sun"),
    ];
    let mut points = cross(ssb_bodies, /* center = SSB */ 0, "wrt SSB");

    // Earth/Moon split: both 399 and 301 are stored relative to the
    // Earth-Moon barycenter (NAIF 3) in modern DE kernels.
    let emb_bodies: &[(i32, &str)] = &[(399, "Earth"), (301, "Moon")];
    points.extend(cross(emb_bodies, /* center = EMB */ 3, "wrt EMB"));

    points
}

/// Geocentric astrometric grid: planets + Sun + Moon as seen from Earth.
/// Centre is body 399 (Earth). With VEC_CORR='LT', Horizons returns the
/// light-time-corrected vector — what an observer sees today minus the
/// effect of stellar aberration. The local SPK backend computes the same
/// via [`Oracle::corrected_state`] with `Corrections::ASTROMETRIC`.
fn spk_astrometric_grid() -> Vec<GridPoint> {
    let bodies: &[(i32, &str)] = &[
        (1, "Mercury barycenter"),
        (2, "Venus barycenter"),
        (4, "Mars barycenter"),
        (5, "Jupiter barycenter"),
        (6, "Saturn barycenter"),
        (7, "Uranus barycenter"),
        (8, "Neptune barycenter"),
        (10, "Sun"),
        (301, "Moon"),
    ];
    cross(bodies, /* observer = Earth body */ 399, "astrometric wrt Earth")
}

/// Heliocentric + selected geocentric grid for VSOP2013.
fn vsop_grid() -> Vec<GridPoint> {
    // Heliocentric: planets the VSOP backend exposes via `heliocentric_state`.
    let helio_bodies: &[(i32, &str)] = &[
        (1, "Mercury barycenter"),
        (2, "Venus barycenter"),
        (3, "Earth-Moon barycenter"),
        (4, "Mars barycenter"),
        (5, "Jupiter barycenter"),
        (6, "Saturn barycenter"),
        (7, "Uranus barycenter"),
        (8, "Neptune barycenter"),
    ];
    let mut points = cross(helio_bodies, 10 /* Sun */, "wrt Sun");

    // Geocentric: the Sun and the Moon are the two interesting checks here.
    let geo_bodies: &[(i32, &str)] = &[(10, "Sun"), (301, "Moon")];
    points.extend(cross(geo_bodies, 399 /* Earth */, "wrt Earth"));

    points
}

fn cross(bodies: &[(i32, &str)], center: i32, suffix: &str) -> Vec<GridPoint> {
    let epochs: &[(f64, &str)] = &[
        (2451545.0, "J2000"),
        (2460000.5, "2023-02-25"),
        (2440000.5, "1968-05-24"),
    ];
    let mut points = Vec::with_capacity(bodies.len() * epochs.len());
    for &(body, body_name) in bodies {
        for &(jd, epoch_name) in epochs {
            points.push(GridPoint {
                name: format!("{} {} @ {}", body_name, suffix, epoch_name),
                body,
                center,
                jd_tdb: jd,
            });
        }
    }
    points
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Report { spk, fixtures } => cmd_report(spk, fixtures),
        Cmd::Bootstrap { spk, out } => cmd_bootstrap(spk, out),
        #[cfg(feature = "fetch")]
        Cmd::Fetch { backend, out } => cmd_fetch(backend, out),
    }
}

fn cmd_report(spk: Option<PathBuf>, fixtures: PathBuf) -> Result<()> {
    let set = FixtureSet::load(&fixtures)
        .with_context(|| format!("failed to load fixtures from {}", fixtures.display()))?;
    let backend = match set.backend {
        BackendKind::Spk => {
            let path = spk
                .ok_or_else(|| anyhow::anyhow!("fixture set requires --spk <kernel-path>"))?;
            Backend::Spk { kernel_path: path }
        }
        BackendKind::Vsop2013 => Backend::Vsop2013,
    };
    let oracle = Oracle::new(backend)?;

    let mut table = ReportTable::new();
    for fx in &set.fixtures {
        let computed =
            oracle.corrected_state(fx.body, fx.center, fx.jd_tdb, fx.frame, set.corrections);
        match computed {
            Ok(observed) => {
                let rep = ErrorReport::compute(fx, &observed);
                table.push(fx, rep);
            }
            Err(e) => eprintln!("skip {}: {}", fx.name, e),
        }
    }

    println!("{}", set.description);
    println!();
    print!("{}", table.render());
    println!();
    if table.all_pass() {
        println!("All fixtures within tolerance.");
    } else {
        println!("Some fixtures exceeded tolerance.");
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_bootstrap(spk: PathBuf, out: PathBuf) -> Result<()> {
    let kernel_label = spk
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let oracle = Oracle::new(Backend::Spk { kernel_path: spk })?;

    let mut fixtures = Vec::new();
    for point in spk_grid() {
        let state = match oracle.state(point.body, point.center, point.jd_tdb, Frame::Icrf) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skip {}: {}", point.name, e);
                continue;
            }
        };
        fixtures.push(Fixture {
            name: point.name,
            body: point.body,
            center: point.center,
            jd_tdb: point.jd_tdb,
            frame: Frame::Icrf,
            pos_km: state.pos_km,
            vel_km_s: state.vel_km_s,
            source: Source::SelfBaseline {
                kernel: kernel_label.clone(),
            },
            // Self-baseline: tolerance is the regression budget, not an
            // accuracy claim. Tight values catch unintended drift.
            tolerance: Tolerance {
                pos_km: 1.0e-6,
                vel_km_s: 1.0e-12,
            },
        });
    }

    let set = FixtureSet {
        description: format!(
            "Self-baseline fixtures generated from {}. NOT external validation.",
            kernel_label
        ),
        backend: BackendKind::Spk,
        corrections: Corrections::GEOMETRIC,
        fixtures,
    };
    set.save(&out)?;
    println!(
        "Wrote {} self-baseline fixtures to {}",
        set.fixtures.len(),
        out.display()
    );
    Ok(())
}

#[cfg(feature = "fetch")]
fn cmd_fetch(backend: BackendArg, out: Option<PathBuf>) -> Result<()> {
    use cosmos_validation::horizons::HorizonsFetcher;

    let (kind, grid, default_out, corrections) = match backend {
        BackendArg::Spk => (
            BackendKind::Spk,
            spk_grid(),
            PathBuf::from("eternal-validation/fixtures/regression-de440/horizons.json"),
            Corrections::GEOMETRIC,
        ),
        BackendArg::Vsop => (
            BackendKind::Vsop2013,
            vsop_grid(),
            PathBuf::from("eternal-validation/fixtures/regression-vsop2013/horizons.json"),
            Corrections::GEOMETRIC,
        ),
        BackendArg::SpkAstrometric => (
            BackendKind::Spk,
            spk_astrometric_grid(),
            PathBuf::from("eternal-validation/fixtures/regression-de440-astrometric/horizons.json"),
            Corrections::ASTROMETRIC,
        ),
        BackendArg::SpkApparentVector => (
            BackendKind::Spk,
            spk_astrometric_grid(), // same grid as astrometric (geocentric, planets+Sun+Moon)
            PathBuf::from(
                "eternal-validation/fixtures/regression-de440-apparent-vector/horizons.json",
            ),
            Corrections::APPARENT_VECTOR,
        ),
        BackendArg::SpkObserverAstrometric => (
            BackendKind::Spk,
            spk_astrometric_grid(),
            PathBuf::from(
                "eternal-validation/fixtures/regression-de440-observer-astrometric/horizons.json",
            ),
            Corrections::ASTROMETRIC,
        ),
        BackendArg::SpkObserverApparent => (
            BackendKind::Spk,
            spk_astrometric_grid(),
            PathBuf::from(
                "eternal-validation/fixtures/regression-de440-observer-apparent/horizons.json",
            ),
            Corrections::APPARENT,
        ),
    };
    let out = out.unwrap_or(default_out);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let fetcher = HorizonsFetcher::new()?;
    let mut fixtures = Vec::new();
    for point in grid {
        eprintln!("fetching {} ...", point.name);
        // Tolerance is selected per-backend rather than just per-Corrections,
        // because OBSERVER fixtures (spherical RA/Dec round-trip via CSV)
        // and VECTOR fixtures (direct Cartesian) have different precision
        // floors even for the same correction stack.
        let tolerance = match backend {
            BackendArg::Spk => match (point.body, point.center) {
                // Moon/Earth split is amplified by the M/E mass ratio; the
                // DE440/DE441 lunar-fit difference exceeds the strict gate.
                (301, 3) | (399, 3) => Tolerance::SPK_LUNAR_CROSS_VERSION,
                _ => Tolerance::SPK_STRICT,
            },
            // Astrometric SPK (LT only) via VECTOR endpoint: the LT
            // iteration adds floating-point round-off beyond the strict
            // gate; sub-cm is realistic.
            BackendArg::SpkAstrometric => Tolerance {
                pos_km: 1.0e-2,
                vel_km_s: 1.0e-8,
            },
            // LT + stellar aberration via VECTOR endpoint; velocity loose.
            BackendArg::SpkApparentVector => Tolerance {
                pos_km: 1.0e-2,
                vel_km_s: 1.0e-3,
            },
            // OBSERVER astrometric: spherical RA/Dec/range round-trip
            // leaks ~mm-cm of radial noise that has zero angular signal.
            BackendArg::SpkObserverAstrometric => Tolerance {
                pos_km: 1.0e-1,
                vel_km_s: 1.0e10,
            },
            // OBSERVER apparent: local pipeline uses IAU 2006/2000A while
            // Horizons OBSERVER uses IAU 76/80/94 — known ~30-50 mas
            // systematic gap. Tolerance absorbs ~30 mas × Neptune distance.
            BackendArg::SpkObserverApparent => Tolerance {
                pos_km: 1.5e3,
                vel_km_s: 1.0e10,
            },
            BackendArg::Vsop => Tolerance::vsop_baseline_for(point.body),
        };
        let _ = (kind, corrections); // retained for the fixture-set header below
        let fx = match backend {
            BackendArg::SpkObserverAstrometric => fetcher.fetch_observer_astrometric(
                &point.name,
                point.body,
                point.center,
                point.jd_tdb,
                tolerance,
            )?,
            BackendArg::SpkObserverApparent => fetcher.fetch_observer_apparent(
                &point.name,
                point.body,
                point.center,
                point.jd_tdb,
                tolerance,
            )?,
            _ => fetcher.fetch(
                &point.name,
                point.body,
                point.center,
                point.jd_tdb,
                tolerance,
                corrections,
            )?,
        };
        fixtures.push(fx);
    }
    let set = FixtureSet {
        description: format!(
            "JPL Horizons reference fixtures for {:?} backend (ICRF, TDB, km/(km·s⁻¹), corrections={:?}).",
            kind, corrections
        ),
        backend: kind,
        corrections,
        fixtures,
    };
    set.save(&out)?;
    println!(
        "Wrote {} Horizons fixtures to {}",
        set.fixtures.len(),
        out.display()
    );
    Ok(())
}
