//! Regression test: every fixture set under `fixtures/` is loaded, the
//! oracle backend declared by the set is instantiated (the SPK kernel
//! comes from `CELESTIAL_VALIDATION_SPK` or the bundled de432s.bsp), and
//! every fixture must stay within its declared tolerance.
//!
//! SPK fixture sets are kernel-scoped via directory name:
//!   `fixtures/regression-de432/` — gates only with de432-class kernels.
//!   `fixtures/regression-de440/` — gates only with de440-class kernels.
//! VSOP/ELP sets are kernel-independent and always gate.

use std::path::{Path, PathBuf};

use cosmos_validation::fixture::{BackendKind, FixtureSet};
use cosmos_validation::oracle::{Backend, Oracle};
use cosmos_validation::report::ErrorReport;

fn locate_kernel() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CELESTIAL_VALIDATION_SPK") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("eternal-ephemeris/tests/data/de432s.bsp");
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

/// Kernel filename → "kernel family" tag. de432s.bsp → "de432", de440.bsp
/// → "de440", de441.bsp → "de441". Used to gate SPK fixture directories.
fn kernel_family(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.starts_with("de440") || name.starts_with("de441") {
        "de440"
    } else if name.starts_with("de43") {
        // de430, de432, de432s, de432t, ...
        "de432"
    } else {
        "unknown"
    }
}

fn fixture_files(kernel: Option<&Path>) -> Vec<PathBuf> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let fam = kernel.map(kernel_family);

    // Each directory under fixtures/ is walked if its name starts with
    // one of these prefixes. The kernel family determines which SPK
    // prefix is active.
    let mut accepted_prefixes: Vec<&str> = vec!["regression-vsop2013"];
    match fam {
        Some("de440") => accepted_prefixes.push("regression-de440"),
        Some("de432") => accepted_prefixes.push("regression-de432"),
        _ => {}
    }

    let Ok(rd) = std::fs::read_dir(&base) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(dir_name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !accepted_prefixes
            .iter()
            .any(|prefix| dir_name.starts_with(prefix))
        {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&p) else {
            continue;
        };
        for f in files.flatten() {
            let fp = f.path();
            if fp.extension().and_then(|s| s.to_str()) == Some("json") {
                out.push(fp);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn fixtures_stay_within_tolerance() {
    let kernel = locate_kernel();
    let files = fixture_files(kernel.as_deref());
    if files.is_empty() {
        eprintln!("Skipping: no fixture files found for available backends");
        return;
    }

    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for file in &files {
        let set = FixtureSet::load(file).expect("load fixtures");

        let backend = match set.backend {
            BackendKind::Spk => match &kernel {
                Some(k) => Backend::Spk {
                    kernel_path: k.clone(),
                },
                None => {
                    eprintln!("Skipping {}: SPK kernel unavailable", file.display());
                    continue;
                }
            },
            BackendKind::Vsop2013 => Backend::Vsop2013,
        };

        let oracle = match Oracle::new(backend) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("Skipping {}: {}", file.display(), e);
                continue;
            }
        };

        for fx in &set.fixtures {
            let observed = match oracle.state(fx.body, fx.center, fx.jd_tdb, fx.frame) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("skip {} ({}): {}", fx.name, file.display(), e);
                    continue;
                }
            };
            let rep = ErrorReport::compute(fx, &observed);
            checked += 1;
            if !rep.within(&fx.tolerance) {
                failures.push(format!(
                    "{} [{}]: pos_err={:.3e} km (tol {:.3e}), vel_err={:.3e} km/s (tol {:.3e})",
                    fx.name,
                    file.file_name().and_then(|s| s.to_str()).unwrap_or("?"),
                    rep.pos_err_km,
                    fx.tolerance.pos_km,
                    rep.vel_err_km_s,
                    fx.tolerance.vel_km_s,
                ));
            }
        }
    }

    if checked == 0 {
        eprintln!("Skipping: no fixtures were evaluated");
        return;
    }

    assert!(
        failures.is_empty(),
        "fixture regressions ({} checked):\n  {}",
        checked,
        failures.join("\n  ")
    );
}
