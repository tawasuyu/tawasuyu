//! stars-check CLI: compare local fixed-star apparent ecliptic
//! positions against Swiss reference values.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use cosmos_ephemeris::jpl::SpkFile;
use cosmos_time::julian::JulianDate;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::TDB;

use cosmos_validation::fixed_stars::{apparent_ecliptic_of_date, by_name};

#[derive(Parser)]
#[command(version, about = "compare oracle fixed-star apparent positions vs Swiss reference")]
struct Cli {
    #[arg(long)]
    spk: PathBuf,
    #[arg(long, default_value = "eternal-validation/fixtures/swiss-stars/swiss-stars.json")]
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
    stars: Vec<StarRef>,
}

#[derive(Debug, Deserialize)]
struct StarRef {
    name: String,
    ecl_lon_deg: f64,
    ecl_lat_deg: f64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let raw = std::fs::read_to_string(&cli.fixtures)
        .with_context(|| format!("reading {}", cli.fixtures.display()))?;
    let doc: Doc = serde_json::from_str(&raw)?;
    let spk = SpkFile::open(&cli.spk).map_err(|e| anyhow::anyhow!("SPK open: {}", e))?;

    println!("{}", doc.description);
    println!();

    for epoch in &doc.epochs {
        println!("=== {} (jd_tdb={}) ===", epoch.label, epoch.jd_tdb);
        let tt = TDB::from_julian_date(JulianDate::new(epoch.jd_tdb, 0.0))
            .to_tt_greenwich()
            .map_err(|e| anyhow::anyhow!("TDB→TT: {:?}", e))?;
        println!(
            "  {:<18}  {:>12}  {:>12}  {:>10}  {:>10}",
            "star", "swiss lon", "ours", "Δ lon \"", "Δ lat \"",
        );

        let mut max_lon = 0.0f64;
        let mut max_lat = 0.0f64;
        for sref in &epoch.stars {
            let star = match by_name(&sref.name) {
                Some(s) => s,
                None => {
                    println!("  {:<18}  (not in local catalog)", sref.name);
                    continue;
                }
            };
            let (lon, lat) = apparent_ecliptic_of_date(star, &spk, &tt, epoch.jd_tdb)?;
            let our_lon_deg = lon.to_degrees();
            let our_lat_deg = lat.to_degrees();
            let d_lon = wrap_signed_deg(our_lon_deg - sref.ecl_lon_deg) * 3600.0;
            let d_lat = (our_lat_deg - sref.ecl_lat_deg) * 3600.0;
            max_lon = max_lon.max(d_lon.abs());
            max_lat = max_lat.max(d_lat.abs());
            println!(
                "  {:<18}  {:>12.6}  {:>12.6}  {:>+10.3}  {:>+10.3}",
                sref.name, sref.ecl_lon_deg, our_lon_deg, d_lon, d_lat,
            );
        }
        println!("  Max |Δ lon|={:.3}\"   Max |Δ lat|={:.3}\"", max_lon, max_lat);
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
