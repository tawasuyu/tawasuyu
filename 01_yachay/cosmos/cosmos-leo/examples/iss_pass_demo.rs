//! Predice pasos de la Estación Espacial Internacional sobre Lima
//! durante las próximas 24 h a partir de un TLE incrustado.
//!
//! Para un satélite LEO en una ubicación dada, un "paso" es un
//! intervalo continuo donde la altura topocéntrica > 10° (el umbral
//! visible útil — por debajo lo tapan edificios y la refracción
//! degrada la imagen). El demo barre cada 30 s e imprime los pasos
//! encontrados con su inicio, máximo y fin.
//!
//! Corré con: `cargo run -p cosmos-leo --example iss_pass_demo --release`.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};
use cosmos_core::Location;
use cosmos_leo::{parse_tle, Satellite, TopoState};

// TLE validado (epoch 2020-07-12 21:15 UTC). El demo es histórico
// pero la geometría de pasos sigue siendo realista — la ISS gana ~7
// pasos/día visibles sobre Lima en cualquier época.
const ISS_TLE: &str = "ISS (ZARYA)
1 25544U 98067A   20194.88612269 -.00002218  00000-0 -31515-4 0  9992
2 25544  51.6461 221.2784 0001413  89.1723 280.4612 15.49507896236008";

const MIN_ALT_DEG: f64 = 10.0;

fn main() {
    let lima = Location::from_degrees(-12.05, -77.05, 150.0).expect("lima");
    let sat = parse_tle(ISS_TLE).expect("ISS parseable");

    let t0 = NaiveDate::from_ymd_opt(2020, 7, 12)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let t_end = t0 + Duration::hours(24);
    let step = Duration::seconds(30);

    println!("=== Pasos de la ISS sobre Lima — 2020-07-12 (24 h) ===");
    println!(
        "Satélite: {} (NORAD {}). Inclinación {:.2}°. Periodo {:.1} min.",
        sat.name(),
        sat.catalog_number(),
        sat.inclination_deg(),
        sat.period_minutes()
    );
    println!("Umbral de visibilidad: alt > {MIN_ALT_DEG:.0}°.\n");

    let passes = find_passes(&sat, &lima, t0, t_end, step);
    if passes.is_empty() {
        println!("(no se detectaron pasos > {MIN_ALT_DEG:.0}° en la ventana)");
        return;
    }
    println!(
        "{:<16} {:<16} {:<16} {:>8} {:>10}",
        "inicio", "máximo", "fin", "alt_max°", "rango_min_km"
    );
    println!("{}", "─".repeat(70));
    for p in &passes {
        println!(
            "{} {} {} {:>8.1} {:>10.0}",
            fmt(&p.start),
            fmt(&p.max),
            fmt(&p.end),
            p.max_alt_deg,
            p.min_range_km
        );
    }
}

struct Pass {
    start: NaiveDateTime,
    max: NaiveDateTime,
    end: NaiveDateTime,
    max_alt_deg: f64,
    min_range_km: f64,
}

fn find_passes(
    sat: &Satellite,
    loc: &Location,
    t0: NaiveDateTime,
    t_end: NaiveDateTime,
    step: Duration,
) -> Vec<Pass> {
    let mut passes = Vec::new();
    let mut t = t0;
    let mut current: Option<Pass> = None;
    while t <= t_end {
        let topo: TopoState = sat
            .propagate(t)
            .map(|s| s.to_topocentric(loc))
            .expect("propaga");
        if topo.altitude_deg > MIN_ALT_DEG {
            match &mut current {
                None => {
                    current = Some(Pass {
                        start: t,
                        max: t,
                        end: t,
                        max_alt_deg: topo.altitude_deg,
                        min_range_km: topo.range_km,
                    });
                }
                Some(p) => {
                    p.end = t;
                    if topo.altitude_deg > p.max_alt_deg {
                        p.max_alt_deg = topo.altitude_deg;
                        p.max = t;
                    }
                    if topo.range_km < p.min_range_km {
                        p.min_range_km = topo.range_km;
                    }
                }
            }
        } else if let Some(p) = current.take() {
            passes.push(p);
        }
        t += step;
    }
    if let Some(p) = current {
        passes.push(p);
    }
    passes
}

fn fmt(t: &NaiveDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        t.year(),
        t.month(),
        t.day(),
        t.hour(),
        t.minute()
    )
}
