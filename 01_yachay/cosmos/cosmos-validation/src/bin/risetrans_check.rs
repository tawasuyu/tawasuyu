//! risetrans-check CLI: compare local next-rise / set / transit times
//! for the Sun and Moon at the four reference charts against Swiss
//! reference values.
//!
//! Times are reported in UT (the format Swiss `rise_trans` uses). We
//! convert TDB → TT → UT1 with the chart's ΔT before comparison.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use cosmos_time::julian::JulianDate;
use cosmos_time::scales::ToTTFromTDB;
use cosmos_time::TDB;

use cosmos_validation::oracle::{Backend, Oracle};
use cosmos_validation::rise_set::{find_next_event, Event, HorizonTarget};
use cosmos_validation::topocentric::Observer;

const MOON: i32 = 301;
const SUN: i32 = 10;
const SECONDS_PER_DAY: f64 = 86_400.0;

#[derive(Parser)]
#[command(version, about = "compare oracle rise/set/transit vs Swiss reference")]
struct Cli {
    #[arg(long)]
    spk: PathBuf,
    #[arg(long, default_value = "eternal-validation/fixtures/swiss-risetrans/swiss-risetrans.json")]
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
    sun_rise_jd_ut: Option<f64>,
    sun_set_jd_ut: Option<f64>,
    sun_transit_jd_ut: Option<f64>,
    moon_rise_jd_ut: Option<f64>,
    moon_set_jd_ut: Option<f64>,
    moon_transit_jd_ut: Option<f64>,
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
        "{:<26}  {:<6}  {:<7}  {:>20}  {:>20}  {:>10}",
        "chart", "body", "event", "swiss UT", "ours UT", "Δ sec",
    );
    println!("{}", "-".repeat(100));

    for chart in &doc.charts {
        let observer = Observer::from_degrees(chart.lat_deg, chart.lon_deg, chart.elev_m);
        let dt_days = chart.delta_t_seconds / SECONDS_PER_DAY;

        // Convert chart's TDB start time to UT for comparison reporting.
        for (body, label, swiss_rise, swiss_set, swiss_transit, target) in [
            (
                SUN,
                "Sun",
                chart.swiss.sun_rise_jd_ut,
                chart.swiss.sun_set_jd_ut,
                chart.swiss.sun_transit_jd_ut,
                HorizonTarget::Refracted,
            ),
            (
                MOON,
                "Moon",
                chart.swiss.moon_rise_jd_ut,
                chart.swiss.moon_set_jd_ut,
                chart.swiss.moon_transit_jd_ut,
                HorizonTarget::Refracted,
            ),
        ] {
            for (event, label_e, swiss_ut) in [
                (Event::Rise, "rise", swiss_rise),
                (Event::Set, "set", swiss_set),
                (Event::Transit, "transit", swiss_transit),
            ] {
                let our_jd_tdb = find_next_event(
                    &oracle,
                    body,
                    &observer,
                    chart.jd_tdb,
                    chart.delta_t_seconds,
                    event,
                    if event == Event::Transit {
                        HorizonTarget::Geometric
                    } else {
                        target
                    },
                    1.6,
                );

                let row = match (our_jd_tdb, swiss_ut) {
                    (Ok(our_tdb), Some(swiss_ut)) => {
                        // Convert ours TDB → TT → UT1 for comparison.
                        let our_tt = TDB::from_julian_date(JulianDate::new(our_tdb, 0.0))
                            .to_tt_greenwich()
                            .map_err(|e| anyhow::anyhow!("TDB→TT: {:?}", e))?;
                        let our_tt_jd = our_tt.to_julian_date();
                        let our_tt_value = our_tt_jd.jd1() + our_tt_jd.jd2();
                        let our_ut = our_tt_value - dt_days;
                        let diff_seconds = (our_ut - swiss_ut) * SECONDS_PER_DAY;
                        format!(
                            "{:<26}  {:<6}  {:<7}  {:>20.10}  {:>20.10}  {:>+10.3}",
                            truncate(&chart.name, 26),
                            label,
                            label_e,
                            swiss_ut,
                            our_ut,
                            diff_seconds,
                        )
                    }
                    (Ok(our_tdb), None) => {
                        format!(
                            "{:<26}  {:<6}  {:<7}  {:>20}  {:>20.10}  {:>10}",
                            truncate(&chart.name, 26),
                            label,
                            label_e,
                            "(none)",
                            our_tdb,
                            "n/a",
                        )
                    }
                    (Err(e), Some(swiss_ut)) => {
                        format!(
                            "{:<26}  {:<6}  {:<7}  {:>20.10}  {:>20}  {:>10}    error: {}",
                            truncate(&chart.name, 26),
                            label,
                            label_e,
                            swiss_ut,
                            "(error)",
                            "n/a",
                            e,
                        )
                    }
                    (Err(e), None) => {
                        format!(
                            "{:<26}  {:<6}  {:<7}  {:>20}  {:>20}  {:>10}    error: {}",
                            truncate(&chart.name, 26),
                            label,
                            label_e,
                            "(none)",
                            "(error)",
                            "n/a",
                            e,
                        )
                    }
                };
                println!("{}", row);
            }
        }
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        &s[..n]
    }
}
