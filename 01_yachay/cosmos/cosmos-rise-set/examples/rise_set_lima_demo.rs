//! Imprime una tabla con rise/transit/set para el Sol, la Luna y los
//! planetas brillantes desde Lima el día actual del prompt
//! (2026-05-27). Útil como "agenda celeste" del observador.
//!
//! Corré con: `cargo run -p cosmos-rise-set --example rise_set_lima_demo
//! --release`.

use cosmos_core::Location;
use cosmos_rise_set::{rise_transit_set_window, Horizon, RiseTransitSet};
use cosmos_skywatch::Body;
use cosmos_time::{JulianDate, TDB};

fn main() {
    let lima = Location::from_degrees(-12.05, -77.05, 150.0).expect("lima");
    let t0 = TDB::from_julian_date(JulianDate::from_calendar(2026, 5, 27, 0, 0, 0.0));

    println!("=== Agenda celeste — Lima · 2026-05-27 (TDB) ===");
    println!(
        "{:<10} {:>12} {:>12} {:>12} {:>10} {}",
        "cuerpo", "salida", "tránsito", "puesta", "alt(°)", "nota"
    );
    println!("{}", "─".repeat(72));

    let bodies = [
        (Body::Sun, Horizon::SunStandard),
        (Body::Moon, Horizon::MoonStandard),
        (Body::Mercury, Horizon::Geometric),
        (Body::Venus, Horizon::Geometric),
        (Body::Mars, Horizon::Geometric),
        (Body::Jupiter, Horizon::Geometric),
        (Body::Saturn, Horizon::Geometric),
    ];

    for (body, horizon) in bodies {
        let r = rise_transit_set_window(&body, &t0, 1.0, &lima, horizon);
        print_row(body, &r);
    }

    println!("\n--- Crepúsculos del Sol ---");
    for (name, horizon) in [
        ("civil", Horizon::CivilTwilight),
        ("náutico", Horizon::NauticalTwilight),
        ("astronómico", Horizon::AstronomicalTwilight),
    ] {
        let r = rise_transit_set_window(&Body::Sun, &t0, 1.0, &lima, horizon);
        let rise_s = r.rise.map(|t| fmt_hm(t.to_julian_date().to_f64())).unwrap_or("—".into());
        let set_s = r.set.map(|t| fmt_hm(t.to_julian_date().to_f64())).unwrap_or("—".into());
        println!("  {:<12} amanece {:>8}   anochece {:>8}", name, rise_s, set_s);
    }
}

fn print_row(body: Body, r: &RiseTransitSet) {
    let rise_s = r.rise.map(|t| fmt_hm(t.to_julian_date().to_f64())).unwrap_or("—".into());
    let set_s = r.set.map(|t| fmt_hm(t.to_julian_date().to_f64())).unwrap_or("—".into());
    let transit_s = fmt_hm(r.transit.to_julian_date().to_f64());
    let nota = if r.never_rises {
        "(no sale)"
    } else if r.never_sets {
        "(circumpolar)"
    } else {
        ""
    };
    println!(
        "{:<10} {:>12} {:>12} {:>12} {:>10.1} {}",
        body.canonical(),
        rise_s,
        transit_s,
        set_s,
        r.transit_altitude_deg,
        nota
    );
}

fn fmt_hm(jd: f64) -> String {
    let (_, _, _, h, m) = jd_to_calendar(jd);
    format!("{h:02}:{m:02}")
}

fn jd_to_calendar(jd: f64) -> (i32, u32, u32, u32, u32) {
    let j = (jd + 0.5).floor() as i64;
    let f = jd + 0.5 - (j as f64);
    let a = j + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = (e - (153 * m + 2) / 5 + 1) as u32;
    let month = (m + 3 - 12 * (m / 10)) as u32;
    let year = (100 * b + d - 4800 + m / 10) as i32;
    let secs_of_day = f * 86400.0;
    let hour = (secs_of_day / 3600.0).floor() as u32;
    let minute = ((secs_of_day - (hour as f64) * 3600.0) / 60.0).floor() as u32;
    (year, month, day, hour, minute)
}
