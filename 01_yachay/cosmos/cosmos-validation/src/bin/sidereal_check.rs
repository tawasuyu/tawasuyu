//! Sidereal-check CLI.
//!
//! Reads a Swiss-generated fixture file with `swiss_extras` (tropical
//! ecliptic longitude, Lahiri sidereal longitude, and ayanamsha) embedded
//! per-fixture, computes the same three quantities through the local
//! oracle + sidereal pipeline, and prints a side-by-side table with
//! residuals in arcseconds.
//!
//! This is informational (no CI gating) — the goal is to expose the
//! Phase 3 precision baseline against Swiss before tightening the
//! ayanamsha series.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use cosmos_time::julian::JulianDate;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::TDB;

use cosmos_validation::fixture::{Corrections, Fixture, Frame};
use cosmos_validation::oracle::{Backend, Oracle};
use cosmos_validation::sidereal::{
    ecliptic_lon_lat, lahiri_ayanamsha, lahiri_sidereal_longitude,
    tet_equatorial_to_ecliptic_of_date,
};
use cosmos_coords::Vector3;

#[derive(Parser)]
#[command(version, about = "compare oracle Lahiri sidereal vs Swiss reference values")]
struct Cli {
    /// Path to JPL SPK kernel.
    #[arg(long)]
    spk: PathBuf,
    /// Path to Swiss-generated fixture file (must carry swiss_extras).
    #[arg(long, default_value = "eternal-validation/fixtures/regression-de440-swiss-apparent/swiss.json")]
    fixtures: PathBuf,
}

#[derive(Debug, Deserialize)]
struct FixtureWithSwissExtras {
    #[serde(flatten)]
    fixture: Fixture,
    swiss_extras: SwissExtras,
}

#[derive(Debug, Deserialize)]
struct SwissExtras {
    tropical_lon_deg: f64,
    lahiri_sidereal_lon_deg: f64,
    lahiri_ayanamsha_deg: f64,
}

#[derive(Debug, Deserialize)]
struct SwissFixtureSet {
    description: String,
    fixtures: Vec<FixtureWithSwissExtras>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let raw = std::fs::read_to_string(&cli.fixtures)
        .with_context(|| format!("reading {}", cli.fixtures.display()))?;
    let set: SwissFixtureSet = serde_json::from_str(&raw)
        .with_context(|| "fixture file does not carry swiss_extras")?;

    let oracle = Oracle::new(Backend::Spk { kernel_path: cli.spk })?;

    println!("{}", set.description);
    println!();
    println!(
        "{:<42}  {:>14}  {:>14}  {:>10}  {:>14}  {:>14}  {:>10}  {:>10}",
        "fixture",
        "swiss trop",
        "ours trop",
        "Δ \"",
        "swiss sid",
        "ours sid",
        "Δ \"",
        "Δ ay \"",
    );
    println!("{}", "-".repeat(140));

    let mut max_trop = 0.0f64;
    let mut max_sid = 0.0f64;
    let mut max_ay = 0.0f64;

    for entry in &set.fixtures {
        let fx = &entry.fixture;
        let observed = oracle.corrected_state(
            fx.body,
            fx.center,
            fx.jd_tdb,
            Frame::TrueEquatorEquinoxOfDate,
            Corrections::APPARENT,
        )?;

        let tt = TDB::from_julian_date(JulianDate::new(fx.jd_tdb, 0.0))
            .to_tt_greenwich()
            .map_err(|e| anyhow::anyhow!("TDB→TT: {:?}", e))?;
        let v_tet = Vector3::new(observed.pos_km[0], observed.pos_km[1], observed.pos_km[2]);
        let v_ecl = tet_equatorial_to_ecliptic_of_date(v_tet, &tt);
        let (lon_tropical, _) = ecliptic_lon_lat(v_ecl);
        let lon_sidereal = lahiri_sidereal_longitude(v_tet, &tt);
        let ay = lahiri_ayanamsha(&tt);

        let ours_trop_deg = lon_tropical.to_degrees();
        let ours_sid_deg = lon_sidereal.to_degrees();
        let ours_ay_deg = ay.to_degrees();

        let dt = wrap_signed_deg(ours_trop_deg - entry.swiss_extras.tropical_lon_deg);
        let ds = wrap_signed_deg(ours_sid_deg - entry.swiss_extras.lahiri_sidereal_lon_deg);
        let day = wrap_signed_deg(ours_ay_deg - entry.swiss_extras.lahiri_ayanamsha_deg);

        let dt_arcsec = dt * 3600.0;
        let ds_arcsec = ds * 3600.0;
        let day_arcsec = day * 3600.0;

        max_trop = max_trop.max(dt_arcsec.abs());
        max_sid = max_sid.max(ds_arcsec.abs());
        max_ay = max_ay.max(day_arcsec.abs());

        println!(
            "{:<42}  {:>14.6}  {:>14.6}  {:>10.3}  {:>14.6}  {:>14.6}  {:>10.3}  {:>10.3}",
            truncate(&fx.name, 42),
            entry.swiss_extras.tropical_lon_deg,
            ours_trop_deg,
            dt_arcsec,
            entry.swiss_extras.lahiri_sidereal_lon_deg,
            ours_sid_deg,
            ds_arcsec,
            day_arcsec,
        );
    }

    println!();
    println!(
        "Max |Δ| (arcsec): tropical={:.3}, sidereal={:.3}, ayanamsha={:.3}",
        max_trop, max_sid, max_ay
    );
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
