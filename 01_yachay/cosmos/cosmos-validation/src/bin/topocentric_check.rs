//! topocentric-check CLI: compare local topocentric Moon + Sun ecliptic
//! longitude against Swiss Ephemeris reference for selected charts.
//!
//! The dominant signal is the diurnal-parallax shift on the Moon (up to
//! ~1°). Sub-arcsec agreement here means the observer-position chain
//! (WGS-84 → ITRS → R3(GAST) → TET) and the underlying apparent
//! pipeline are both correctly wired.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use cosmos_coords::Vector3;
use cosmos_time::julian::JulianDate;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::TDB;

use cosmos_validation::fixture::{Corrections, Frame};
use cosmos_validation::oracle::{Backend, Oracle};
use cosmos_validation::sidereal::{ecliptic_lon_lat, tet_equatorial_to_ecliptic_of_date};
use cosmos_validation::topocentric::{apparent_topocentric_state, Observer};

const MOON: i32 = 301;
const SUN: i32 = 10;

#[derive(Parser)]
#[command(version, about = "compare oracle topocentric Moon + Sun vs Swiss reference")]
struct Cli {
    #[arg(long)]
    spk: PathBuf,
    #[arg(long, default_value = "eternal-validation/fixtures/swiss-topocentric/swiss-topocentric.json")]
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
    moon_geo_lon_deg: f64,
    moon_topo_lon_deg: f64,
    moon_geo_dist_au: f64,
    moon_topo_dist_au: f64,
    sun_geo_lon_deg: f64,
    sun_topo_lon_deg: f64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let raw = std::fs::read_to_string(&cli.fixtures)
        .with_context(|| format!("reading {}", cli.fixtures.display()))?;
    let doc: Doc = serde_json::from_str(&raw)?;
    let oracle = Oracle::new(Backend::Spk { kernel_path: cli.spk })?;

    println!("{}", doc.description);
    println!();

    for chart in &doc.charts {
        compare(&oracle, chart)?;
        println!();
    }
    Ok(())
}

fn compare(oracle: &Oracle, chart: &Chart) -> Result<()> {
    println!("=== {} ===", chart.name);
    println!(
        "  observer: lat={:.4}°  lon={:.4}°  elev={}m   ΔT={}s",
        chart.lat_deg, chart.lon_deg, chart.elev_m, chart.delta_t_seconds
    );

    let observer = Observer::from_degrees(chart.lat_deg, chart.lon_deg, chart.elev_m);

    let tt = TDB::from_julian_date(JulianDate::new(chart.jd_tdb, 0.0))
        .to_tt_greenwich()
        .map_err(|e| anyhow::anyhow!("TDB→TT: {:?}", e))?;

    for (body, label, swiss_geo, swiss_topo) in [
        (
            MOON,
            "Moon",
            chart.swiss.moon_geo_lon_deg,
            chart.swiss.moon_topo_lon_deg,
        ),
        (
            SUN,
            "Sun ",
            chart.swiss.sun_geo_lon_deg,
            chart.swiss.sun_topo_lon_deg,
        ),
    ] {
        // Geocentric apparent ecliptic longitude.
        let geo = oracle.corrected_state(
            body,
            399,
            chart.jd_tdb,
            Frame::TrueEquatorEquinoxOfDate,
            Corrections::APPARENT,
        )?;
        let geo_lon_deg = ecl_lon_deg(geo.pos_km, &tt);

        // Topocentric.
        let topo = apparent_topocentric_state(
            oracle,
            body,
            chart.jd_tdb,
            &observer,
            chart.delta_t_seconds,
        )?;
        let topo_lon_deg = ecl_lon_deg(topo.pos_km, &tt);

        let parallax_swiss = wrap_signed_deg(swiss_topo - swiss_geo) * 3600.0;
        let parallax_ours = wrap_signed_deg(topo_lon_deg - geo_lon_deg) * 3600.0;
        let geo_diff = wrap_signed_deg(geo_lon_deg - swiss_geo) * 3600.0;
        let topo_diff = wrap_signed_deg(topo_lon_deg - swiss_topo) * 3600.0;

        println!(
            "  {} geo  lon={:>10.6}°  swiss={:>10.6}°  Δ={:>+8.3}\"   parallax: ours={:>+8.2}\" swiss={:>+8.2}\" Δ={:>+8.3}\"",
            label, geo_lon_deg, swiss_geo, geo_diff, parallax_ours, parallax_swiss,
            parallax_ours - parallax_swiss
        );
        println!(
            "  {} topo lon={:>10.6}°  swiss={:>10.6}°  Δ={:>+8.3}\"",
            label, topo_lon_deg, swiss_topo, topo_diff,
        );
    }

    let moon_dist_swiss_geo_km = chart.swiss.moon_geo_dist_au * 149_597_870.7;
    let moon_dist_swiss_topo_km = chart.swiss.moon_topo_dist_au * 149_597_870.7;
    println!(
        "  Moon distance swiss: geo={:>10.3} km   topo={:>10.3} km   Δ={:>+9.3} km",
        moon_dist_swiss_geo_km,
        moon_dist_swiss_topo_km,
        moon_dist_swiss_topo_km - moon_dist_swiss_geo_km,
    );

    Ok(())
}

fn ecl_lon_deg(pos_km: [f64; 3], tt: &cosmos_time::TT) -> f64 {
    let v_tet = Vector3::new(pos_km[0], pos_km[1], pos_km[2]);
    let v_ecl = tet_equatorial_to_ecliptic_of_date(v_tet, tt);
    let (lon, _) = ecliptic_lon_lat(v_ecl);
    lon.to_degrees()
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
