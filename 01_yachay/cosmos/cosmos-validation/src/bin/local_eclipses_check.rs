//! local-eclipses-check CLI: enumerate next N local solar eclipses
//! per observer site and diff against Swiss reference.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use cosmos_ephemeris::jpl::SpkFile;
use cosmos_time::julian::JulianDate;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::TDB;

use cosmos_validation::delta_t::delta_t_seconds;
use cosmos_validation::eclipses::{next_local_solar_eclipse, SolarEclipseKind};
use cosmos_validation::topocentric::Observer;

const SECONDS_PER_DAY: f64 = 86_400.0;

#[derive(Parser)]
#[command(version, about = "compare local solar eclipse times + types vs Swiss")]
struct Cli {
    #[arg(long)]
    spk: PathBuf,
    #[arg(long, default_value = "eternal-validation/fixtures/swiss-local-eclipses/swiss-local-eclipses.json")]
    fixtures: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Doc {
    description: String,
    start_jd_ut: f64,
    sites: Vec<Site>,
}

#[derive(Debug, Deserialize)]
struct Site {
    name: String,
    lat_deg: f64,
    lon_deg: f64,
    elev_m: f64,
    eclipses: Vec<EclipseRef>,
}

#[derive(Debug, Deserialize)]
struct EclipseRef {
    max_jd_ut: f64,
    kind: String,
    magnitude: f64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let raw = std::fs::read_to_string(&cli.fixtures)
        .with_context(|| format!("reading {}", cli.fixtures.display()))?;
    let doc: Doc = serde_json::from_str(&raw)?;
    let spk = SpkFile::open(&cli.spk).map_err(|e| anyhow::anyhow!("SPK open: {}", e))?;

    println!("{}", doc.description);
    println!();

    for site in &doc.sites {
        println!("=== {} (lat={:.4}°, lon={:.4}°) ===", site.name, site.lat_deg, site.lon_deg);
        println!(
            "{:<4}  {:>20}  {:>20}  {:>10}  {:>10}  {:>10}  {:<10}  {:<10}",
            "#", "swiss max (UT)", "ours max (UT)", "Δ (s)", "swiss mag", "ours mag", "swiss kind", "ours kind",
        );
        println!("{}", "-".repeat(110));

        let observer = Observer::from_degrees(site.lat_deg, site.lon_deg, site.elev_m);

        // Convert UT start → TDB (just add a representative ΔT).
        let mut search_jd_tdb = doc.start_jd_ut + delta_t_seconds(doc.start_jd_ut) / SECONDS_PER_DAY;
        for (i, eref) in site.eclipses.iter().enumerate() {
            // Use Swiss ΔT for the chart's epoch.
            let dt_seconds = delta_t_seconds(eref.max_jd_ut);
            // Search up to 10 years (≈ 124 synodic months) — solar eclipses
            // visible from a given site can be 5+ years apart (Sydney, Tokyo).
            let res = next_local_solar_eclipse(&spk, search_jd_tdb, &observer, dt_seconds, 124)?;
            let row = match res {
                Some((our_tdb, snap)) => {
                    let dt = delta_t_seconds(our_tdb);
                    let tt = TDB::from_julian_date(JulianDate::new(our_tdb, 0.0))
                        .to_tt_greenwich()
                        .map_err(|e| anyhow::anyhow!("TDB→TT: {:?}", e))?;
                    let tt_jd = tt.to_julian_date();
                    let our_ut = tt_jd.jd1() + tt_jd.jd2() - dt / SECONDS_PER_DAY;
                    let diff_seconds = (our_ut - eref.max_jd_ut) * SECONDS_PER_DAY;
                    let our_kind = match snap.kind {
                        SolarEclipseKind::Total => "total",
                        SolarEclipseKind::Annular => "annular",
                        SolarEclipseKind::Partial => "partial",
                        SolarEclipseKind::Hybrid => "hybrid",
                        SolarEclipseKind::None => "none",
                    };
                    search_jd_tdb = our_tdb + 1.0;
                    format!(
                        "{:<4}  {:>20.6}  {:>20.6}  {:>+10.1}  {:>10.4}  {:>10.4}  {:<10}  {:<10}",
                        i + 1,
                        eref.max_jd_ut,
                        our_ut,
                        diff_seconds,
                        eref.magnitude,
                        snap.magnitude.max(0.0),
                        eref.kind,
                        our_kind,
                    )
                }
                None => format!("{:<4}  {:>20.6}  (none found)", i + 1, eref.max_jd_ut),
            };
            println!("{}", row);
        }
        println!();
    }
    Ok(())
}
