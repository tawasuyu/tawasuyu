//! altaz-check CLI: compare local Moon + Sun alt/az against Swiss
//! reference values for the four canonical charts.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use cosmos_validation::oracle::{Backend, Oracle};
use cosmos_validation::topocentric::{apparent_alt_az, Observer};

const MOON: i32 = 301;
const SUN: i32 = 10;

#[derive(Parser)]
#[command(version, about = "compare oracle Moon + Sun alt/az vs Swiss reference")]
struct Cli {
    #[arg(long)]
    spk: PathBuf,
    #[arg(long, default_value = "eternal-validation/fixtures/swiss-altaz/swiss-altaz.json")]
    fixtures: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Doc {
    description: String,
    charts: Vec<Chart>,
}

#[derive(Debug, Deserialize)]
struct Chart {
    name: String,
    lat_deg: f64,
    lon_deg: f64,
    elev_m: f64,
    jd_tdb: f64,
    delta_t_seconds: f64,
    swiss: SwissRef,
}

#[derive(Debug, Deserialize)]
struct SwissRef {
    moon_az_deg: f64,
    moon_true_alt_deg: f64,
    sun_az_deg: f64,
    sun_true_alt_deg: f64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let raw = std::fs::read_to_string(&cli.fixtures)
        .with_context(|| format!("reading {}", cli.fixtures.display()))?;
    let doc: Doc = serde_json::from_str(&raw)?;
    let oracle = Oracle::new(Backend::Spk { kernel_path: cli.spk })?;

    println!("{}", doc.description);
    println!();
    println!(
        "{:<26}  {:<6}  {:>10}  {:>10}  {:>9}  {:>10}  {:>10}  {:>9}",
        "chart", "body", "swiss az", "ours", "Δ az \"", "swiss alt", "ours", "Δ alt \"",
    );
    println!("{}", "-".repeat(110));

    for chart in &doc.charts {
        let observer = Observer::from_degrees(chart.lat_deg, chart.lon_deg, chart.elev_m);
        for (body, label, swiss_az, swiss_alt) in [
            (MOON, "Moon", chart.swiss.moon_az_deg, chart.swiss.moon_true_alt_deg),
            (SUN, "Sun", chart.swiss.sun_az_deg, chart.swiss.sun_true_alt_deg),
        ] {
            let (alt, az) =
                apparent_alt_az(&oracle, body, chart.jd_tdb, &observer, chart.delta_t_seconds)?;
            let ours_alt_deg = alt.to_degrees();
            // Swiss `swe.azalt` uses the classical astronomical convention
            // (azimuth from South going West), modulo 360°. Our public
            // `apparent_alt_az` returns N=0/E=90 (modern). Convert here so
            // the diff column reflects implementation accuracy, not a
            // 180° convention shift.
            let ours_az_deg = (az.to_degrees() + 180.0) % 360.0;
            let d_az = wrap_signed_deg(ours_az_deg - swiss_az) * 3600.0;
            let d_alt = (ours_alt_deg - swiss_alt) * 3600.0;
            println!(
                "{:<26}  {:<6}  {:>10.5}  {:>10.5}  {:>+9.3}  {:>10.5}  {:>10.5}  {:>+9.3}",
                truncate(&chart.name, 26),
                label,
                swiss_az,
                ours_az_deg,
                d_az,
                swiss_alt,
                ours_alt_deg,
                d_alt,
            );
        }
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

fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        &s[..n]
    }
}
