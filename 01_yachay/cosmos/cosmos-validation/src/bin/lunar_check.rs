//! lunar-check CLI: compare local Mean / True lunar node and Lilith
//! against Swiss Ephemeris reference values across the modern era.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use cosmos_ephemeris::jpl::SpkFile;
use cosmos_time::julian::JulianDate;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::TDB;

use cosmos_validation::lunar::{
    mean_lilith, mean_lunar_node, true_lilith_geocentric, true_lunar_node_geocentric,
};

#[derive(Parser)]
#[command(version, about = "compare oracle Mean/True lunar node + Lilith vs Swiss reference")]
struct Cli {
    #[arg(long)]
    spk: PathBuf,
    #[arg(long, default_value = "eternal-validation/fixtures/swiss-lunar/swiss-lunar.json")]
    fixtures: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Doc {
    description: String,
    samples: Vec<Sample>,
}

#[derive(Debug, Deserialize)]
struct Sample {
    label: String,
    jd_tdb: f64,
    mean_node_deg: f64,
    true_node_deg: f64,
    mean_apog_deg: f64,
    oscu_apog_deg: f64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let raw = std::fs::read_to_string(&cli.fixtures)
        .with_context(|| format!("reading {}", cli.fixtures.display()))?;
    let doc: Doc = serde_json::from_str(&raw)?;
    let spk = SpkFile::open(&cli.spk).map_err(|e| anyhow::anyhow!("SPK open: {}", e))?;

    println!("{}", doc.description);
    println!();
    println!(
        "{:<14}  {:>15}  {:>10}  {:>10}  {:>15}  {:>10}  {:>10}  {:>15}  {:>10}  {:>15}  {:>10}",
        "epoch",
        "swiss mean node",
        "ours",
        "Δ \"",
        "swiss true node",
        "ours",
        "Δ \"",
        "swiss mean apog",
        "Δ \"",
        "swiss oscu apog",
        "Δ \"",
    );
    println!("{}", "-".repeat(170));

    for sample in &doc.samples {
        let tt = TDB::from_julian_date(JulianDate::new(sample.jd_tdb, 0.0))
            .to_tt_greenwich()
            .map_err(|e| anyhow::anyhow!("TDB→TT: {:?}", e))?;

        let our_mean_node = mean_lunar_node(&tt).to_degrees();
        let our_mean_lilith = mean_lilith(&tt).to_degrees();
        let our_true_node = true_lunar_node_geocentric(&spk, &tt, sample.jd_tdb)?.to_degrees();
        let our_true_lilith = true_lilith_geocentric(&spk, &tt, sample.jd_tdb)?.to_degrees();

        let d_mn = diff_arcsec(our_mean_node, sample.mean_node_deg);
        let d_tn = diff_arcsec(our_true_node, sample.true_node_deg);
        let d_ma = diff_arcsec(our_mean_lilith, sample.mean_apog_deg);
        let d_oa = diff_arcsec(our_true_lilith, sample.oscu_apog_deg);

        println!(
            "{:<14}  {:>15.6}  {:>10.6}  {:>+10.3}  {:>15.6}  {:>10.6}  {:>+10.3}  {:>15.6}  {:>+10.3}  {:>15.6}  {:>+10.3}",
            sample.label,
            sample.mean_node_deg,
            our_mean_node,
            d_mn,
            sample.true_node_deg,
            our_true_node,
            d_tn,
            sample.mean_apog_deg,
            d_ma,
            sample.oscu_apog_deg,
            d_oa,
        );
    }

    Ok(())
}

fn diff_arcsec(ours_deg: f64, swiss_deg: f64) -> f64 {
    let mut d = (ours_deg - swiss_deg) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    d * 3600.0
}
