//! eclipses-check CLI: enumerate next N lunar eclipses with our local
//! finder and diff against Swiss reference.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use cosmos_ephemeris::jpl::SpkFile;
use cosmos_time::julian::JulianDate;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::TDB;

use cosmos_validation::delta_t::delta_t_seconds;
use cosmos_validation::eclipses::{
    next_lunar_eclipse, next_solar_eclipse, LunarEclipseKind, SolarEclipseKind,
};

const SECONDS_PER_DAY: f64 = 86_400.0;

#[derive(Parser)]
#[command(version, about = "compare lunar eclipse times + types vs Swiss")]
struct Cli {
    #[arg(long)]
    spk: PathBuf,
    #[arg(long, default_value = "eternal-validation/fixtures/swiss-eclipses/swiss-eclipses.json")]
    fixtures: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Doc {
    description: String,
    start_jd_ut: f64,
    lunar_eclipses: Vec<EclipseRef>,
    solar_eclipses_global: Vec<EclipseRef>,
}

#[derive(Debug, Deserialize)]
struct EclipseRef {
    max_jd_ut: f64,
    kind: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let raw = std::fs::read_to_string(&cli.fixtures)
        .with_context(|| format!("reading {}", cli.fixtures.display()))?;
    let doc: Doc = serde_json::from_str(&raw)?;
    let spk = SpkFile::open(&cli.spk).map_err(|e| anyhow::anyhow!("SPK open: {}", e))?;

    println!("{}", doc.description);

    println!("\n=== Lunar eclipses ===");
    print_header();
    let mut search_jd_tdb = ut_to_tdb(doc.start_jd_ut, 70.0)?;
    for (i, eref) in doc.lunar_eclipses.iter().enumerate() {
        let res = next_lunar_eclipse(&spk, search_jd_tdb, 24)
            .map_err(|e| anyhow::anyhow!("lunar eclipse search: {:?}", e))?;
        let (our_tdb, kind_str) = match res {
            Some((tdb, snap)) => {
                let k = match snap.kind {
                    LunarEclipseKind::Total => "total",
                    LunarEclipseKind::Partial => "partial",
                    LunarEclipseKind::Penumbral => "penumbral",
                    LunarEclipseKind::None => "none",
                };
                (Some(tdb), k.to_string())
            }
            None => (None, "(none)".to_string()),
        };
        print_row(i + 1, eref, our_tdb, &kind_str)?;
        if let Some(t) = our_tdb {
            search_jd_tdb = t + 1.0;
        }
    }

    println!("\n=== Solar eclipses (global) ===");
    print_header();
    let mut search_jd_tdb = ut_to_tdb(doc.start_jd_ut, 70.0)?;
    for (i, eref) in doc.solar_eclipses_global.iter().enumerate() {
        let res = next_solar_eclipse(&spk, search_jd_tdb, 24)
            .map_err(|e| anyhow::anyhow!("solar eclipse search: {:?}", e))?;
        let (our_tdb, kind_str) = match res {
            Some((tdb, snap)) => {
                let k = match snap.kind {
                    SolarEclipseKind::Total => "total",
                    SolarEclipseKind::Annular => "annular",
                    SolarEclipseKind::Partial => "partial",
                    SolarEclipseKind::Hybrid => "hybrid",
                    SolarEclipseKind::None => "none",
                };
                (Some(tdb), k.to_string())
            }
            None => (None, "(none)".to_string()),
        };
        print_row(i + 1, eref, our_tdb, &kind_str)?;
        if let Some(t) = our_tdb {
            search_jd_tdb = t + 1.0;
        }
    }

    Ok(())
}

fn print_header() {
    println!(
        "{:<4}  {:>20}  {:>20}  {:>10}  {:<10}  {:<10}",
        "#", "swiss max (UT)", "ours max (UT)", "Δ (s)", "swiss kind", "ours kind",
    );
    println!("{}", "-".repeat(95));
}

fn print_row(idx: usize, eref: &EclipseRef, our_tdb: Option<f64>, kind: &str) -> Result<()> {
    match our_tdb {
        Some(tdb) => {
            let dt_s = approx_delta_t(tdb);
            let tt = TDB::from_julian_date(JulianDate::new(tdb, 0.0))
                .to_tt_greenwich()
                .map_err(|e| anyhow::anyhow!("TDB→TT: {:?}", e))?;
            let tt_jd = tt.to_julian_date();
            let our_ut = tt_jd.jd1() + tt_jd.jd2() - dt_s / SECONDS_PER_DAY;
            let diff_seconds = (our_ut - eref.max_jd_ut) * SECONDS_PER_DAY;
            println!(
                "{:<4}  {:>20.6}  {:>20.6}  {:>+10.1}  {:<10}  {:<10}",
                idx, eref.max_jd_ut, our_ut, diff_seconds, eref.kind, kind,
            );
        }
        None => {
            println!(
                "{:<4}  {:>20.6}  {:>20}  {:>10}  {:<10}  {:<10}",
                idx, eref.max_jd_ut, "(none)", "n/a", eref.kind, kind,
            );
        }
    }
    Ok(())
}

fn ut_to_tdb(jd_ut: f64, dt_seconds: f64) -> Result<f64> {
    // UT → TT via ΔT, then TT → TDB ≈ TT for our precision needs at this stage.
    Ok(jd_ut + dt_seconds / SECONDS_PER_DAY)
}

/// Re-export the centralised ΔT helper for the older callsite name.
fn approx_delta_t(jd_tdb: f64) -> f64 {
    delta_t_seconds(jd_tdb)
}
