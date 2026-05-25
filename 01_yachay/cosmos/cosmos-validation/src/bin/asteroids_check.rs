//! asteroids-check CLI: compare local apparent ecliptic positions for
//! the four main-belt astrologically-important asteroids against Swiss
//! Ephemeris reference values across three epochs.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use cosmos_ephemeris::jpl::SpkFile;
use cosmos_time::julian::JulianDate;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::TDB;

use cosmos_validation::asteroids::{apparent_ecliptic_of_date, naif_id_by_name};

#[derive(Parser)]
#[command(version, about = "compare oracle apparent asteroid positions vs Swiss")]
struct Cli {
    /// Planet kernel (Earth + Sun + planet bodies). Default = de440.bsp.
    #[arg(long, default_value = "/home/sergio/.local/share/ephemeris/de440.bsp")]
    planets_spk: PathBuf,
    /// Asteroid kernel (sb441-n16.bsp).
    #[arg(long, default_value = "/home/sergio/.local/share/ephemeris/sb441-n16.bsp")]
    asteroids_spk: PathBuf,
    #[arg(long, default_value = "eternal-validation/fixtures/swiss-asteroids/swiss-asteroids.json")]
    fixtures: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Doc {
    description: String,
    epochs: Vec<Epoch>,
}

#[derive(Debug, Deserialize)]
struct Epoch {
    label: String,
    jd_tdb: f64,
    asteroids: Vec<AstRef>,
}

#[derive(Debug, Deserialize)]
struct AstRef {
    name: String,
    ecl_lon_deg: Option<f64>,
    ecl_lat_deg: Option<f64>,
    dist_au: Option<f64>,
    error: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let raw = std::fs::read_to_string(&cli.fixtures)
        .with_context(|| format!("reading {}", cli.fixtures.display()))?;
    let doc: Doc = serde_json::from_str(&raw)?;
    let planets = SpkFile::open(&cli.planets_spk)
        .map_err(|e| anyhow::anyhow!("planet SPK open: {}", e))?;
    let asteroids = SpkFile::open(&cli.asteroids_spk)
        .map_err(|e| anyhow::anyhow!("asteroid SPK open: {}", e))?;

    println!("{}", doc.description);
    println!();

    for epoch in &doc.epochs {
        println!("=== {} (jd_tdb = {}) ===", epoch.label, epoch.jd_tdb);
        let tt = TDB::from_julian_date(JulianDate::new(epoch.jd_tdb, 0.0))
            .to_tt_greenwich()
            .map_err(|e| anyhow::anyhow!("TDB→TT: {:?}", e))?;
        println!(
            "  {:<10}  {:>12}  {:>12}  {:>9}  {:>10}  {:>9}  {:>11}",
            "asteroid", "swiss lon", "ours lon", "Δ lon \"", "swiss lat", "Δ lat \"", "Δ dist au",
        );
        for aref in &epoch.asteroids {
            if let Some(err) = &aref.error {
                println!("  {:<10}  swiss error: {}", aref.name, err);
                continue;
            }
            let naif = match naif_id_by_name(&aref.name) {
                Some(id) => id,
                None => {
                    println!("  {:<10}  (no NAIF mapping)", aref.name);
                    continue;
                }
            };
            let result = apparent_ecliptic_of_date(naif, &asteroids, &planets, &tt, epoch.jd_tdb);
            let (our_lon, our_lat, our_dist) = match result {
                Ok(t) => t,
                Err(e) => {
                    println!(
                        "  {:<10}  ours error: {} (likely body not in asteroid kernel)",
                        aref.name, e
                    );
                    continue;
                }
            };
            let our_lon_deg = our_lon.to_degrees();
            let our_lat_deg = our_lat.to_degrees();
            let swiss_lon = aref.ecl_lon_deg.unwrap_or(f64::NAN);
            let swiss_lat = aref.ecl_lat_deg.unwrap_or(f64::NAN);
            let swiss_dist = aref.dist_au.unwrap_or(f64::NAN);
            let d_lon = wrap_signed_deg(our_lon_deg - swiss_lon) * 3600.0;
            let d_lat = (our_lat_deg - swiss_lat) * 3600.0;
            let d_dist = our_dist - swiss_dist;
            println!(
                "  {:<10}  {:>12.6}  {:>12.6}  {:>+9.3}  {:>10.6}  {:>+9.3}  {:>+11.3e}",
                aref.name, swiss_lon, our_lon_deg, d_lon, swiss_lat, d_lat, d_dist,
            );
        }
        println!();
    }
    Ok(())
}

fn wrap_signed_deg(d: f64) -> f64 {
    let d = d % 360.0;
    if d > 180.0 {
        d - 360.0
    } else if d < -180.0 {
        d + 360.0
    } else {
        d
    }
}
