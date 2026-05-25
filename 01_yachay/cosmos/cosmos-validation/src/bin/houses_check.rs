//! houses-check CLI: compare local Asc/MC + house cusps against Swiss
//! Ephemeris for a small set of canonical charts.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use cosmos_core::Location;
use cosmos_time::julian::JulianDate;
use cosmos_time::scales::conversions::ToUT1WithDeltaT;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::sidereal::GAST;
use cosmos_time::TDB;

use cosmos_validation::houses::{
    ascendant, campanus_houses, equal_houses, koch_houses, midheaven, placidus_houses,
    porphyry_houses, regiomontanus_houses, whole_sign_houses,
};
use cosmos_validation::sidereal::true_obliquity_iau2006a;

#[derive(Parser)]
#[command(version, about = "compare oracle Asc/MC and house cusps vs Swiss reference")]
struct Cli {
    #[arg(long, default_value = "eternal-validation/fixtures/swiss-houses/swiss-houses.json")]
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
    jd_tdb: f64,
    delta_t_seconds: f64,
    swiss: SwissCusps,
}

#[derive(Debug, Deserialize)]
struct SwissCusps {
    ascendant_deg: f64,
    mc_deg: f64,
    armc_deg: f64,
    whole_sign_cusps_deg: Vec<f64>,
    equal_cusps_deg: Vec<f64>,
    placidus_cusps_deg: Vec<f64>,
    koch_cusps_deg: Vec<f64>,
    regiomontanus_cusps_deg: Vec<f64>,
    campanus_cusps_deg: Vec<f64>,
    porphyry_cusps_deg: Vec<f64>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let raw = std::fs::read_to_string(&cli.fixtures)
        .with_context(|| format!("reading {}", cli.fixtures.display()))?;
    let doc: Doc = serde_json::from_str(&raw)?;
    println!("{}", doc.description);
    println!();

    for chart in &doc.charts {
        compare(chart)?;
        println!();
    }
    Ok(())
}

fn compare(chart: &Chart) -> Result<()> {
    let tt = TDB::from_julian_date(JulianDate::new(chart.jd_tdb, 0.0))
        .to_tt_greenwich()
        .map_err(|e| anyhow::anyhow!("TDB→TT: {:?}", e))?;
    let ut1 = tt
        .to_ut1_with_delta_t(chart.delta_t_seconds)
        .map_err(|e| anyhow::anyhow!("TT→UT1: {:?}", e))?;

    let location = Location::from_degrees(chart.lat_deg, chart.lon_deg, 0.0)
        .map_err(|e| anyhow::anyhow!("Location: {:?}", e))?;

    let gast =
        GAST::from_ut1_and_tt(&ut1, &tt).map_err(|e| anyhow::anyhow!("GAST: {:?}", e))?;
    let last = gast.to_last(&location);
    let last_rad = last.angle().radians();

    let eps = true_obliquity_iau2006a(&tt)
        .map_err(|e| anyhow::anyhow!("true obliquity: {}", e))?;
    let lat = chart.lat_deg.to_radians();

    let asc = ascendant(last_rad, lat, eps);
    let mc = midheaven(last_rad, eps);

    let asc_deg = asc.to_degrees();
    let mc_deg = mc.to_degrees();

    println!("=== {} ===", chart.name);
    println!(
        "  lat={:.4}°  lon={:.4}°  jd_tdb={}  ΔT={}s",
        chart.lat_deg, chart.lon_deg, chart.jd_tdb, chart.delta_t_seconds
    );
    println!(
        "  ARMC swiss={:>10.6}°   LAST ours={:>10.6}°   Δ={:>+8.3}\"",
        chart.swiss.armc_deg,
        last_rad.to_degrees(),
        diff_arcsec(chart.swiss.armc_deg, last_rad.to_degrees()),
    );
    println!(
        "  Asc  swiss={:>10.6}°   ours={:>10.6}°   Δ={:>+8.3}\"",
        chart.swiss.ascendant_deg,
        asc_deg,
        diff_arcsec(chart.swiss.ascendant_deg, asc_deg),
    );
    println!(
        "  MC   swiss={:>10.6}°   ours={:>10.6}°   Δ={:>+8.3}\"",
        chart.swiss.mc_deg,
        mc_deg,
        diff_arcsec(chart.swiss.mc_deg, mc_deg),
    );

    let ws_ours = whole_sign_houses(asc);
    let eq_ours = equal_houses(asc);

    println!("  Whole-Sign cusps (Δ arcsec):");
    print_cusp_row(&chart.swiss.whole_sign_cusps_deg, &ws_ours);
    println!("  Equal cusps      (Δ arcsec):");
    print_cusp_row(&chart.swiss.equal_cusps_deg, &eq_ours);

    match placidus_houses(last_rad, lat, eps) {
        Ok(pl) => {
            println!("  Placidus cusps   (Δ arcsec):");
            print_cusp_row(&chart.swiss.placidus_cusps_deg, &pl);
        }
        Err(e) => println!("  Placidus: {}", e),
    }
    match koch_houses(last_rad, lat, eps) {
        Ok(k) => {
            println!("  Koch cusps       (Δ arcsec):");
            print_cusp_row(&chart.swiss.koch_cusps_deg, &k);
        }
        Err(e) => println!("  Koch: {}", e),
    }
    let r = regiomontanus_houses(last_rad, lat, eps);
    println!("  Regiomontanus cusps (Δ arcsec):");
    print_cusp_row(&chart.swiss.regiomontanus_cusps_deg, &r);
    match campanus_houses(last_rad, lat, eps) {
        Ok(c) => {
            println!("  Campanus cusps   (Δ arcsec):");
            print_cusp_row(&chart.swiss.campanus_cusps_deg, &c);
        }
        Err(e) => println!("  Campanus: {}", e),
    }
    let p = porphyry_houses(last_rad, lat, eps);
    println!("  Porphyry cusps   (Δ arcsec):");
    print_cusp_row(&chart.swiss.porphyry_cusps_deg, &p);

    Ok(())
}

fn diff_arcsec(swiss_deg: f64, ours_deg: f64) -> f64 {
    let mut d = (ours_deg - swiss_deg) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    d * 3600.0
}

fn print_cusp_row(swiss: &[f64], ours: &[f64; 12]) {
    print!("   ");
    for i in 0..12 {
        let d = diff_arcsec(swiss[i], ours[i].to_degrees());
        print!(" h{:<2}={:>+7.3}", i + 1, d);
        if i == 5 {
            print!("\n   ");
        }
    }
    println!();
}
