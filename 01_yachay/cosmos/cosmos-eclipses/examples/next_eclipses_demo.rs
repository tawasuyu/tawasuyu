//! Barre 2026-01-01..2030-01-01 buscando eclipses solares y lunares
//! geocéntricos. Imprime una tabla con fecha, tipo y magnitud.
//!
//! Corré con: `cargo run -p cosmos-eclipses --example next_eclipses_demo
//! --release`.

use cosmos_eclipses::{find_lunar_eclipses, find_solar_eclipses, EclipseEvent};
use cosmos_time::JulianDate;

fn main() {
    let jd_from = JulianDate::from_calendar(2026, 1, 1, 0, 0, 0.0).to_f64();
    let jd_to = JulianDate::from_calendar(2030, 1, 1, 0, 0, 0.0).to_f64();
    let step = 1.0 / 24.0;

    println!("=== Eclipses geocéntricos — 2026-01-01 → 2030-01-01 ===");
    println!("paso de muestreo: 1 hora · ventana 4 años\n");

    let solar = find_solar_eclipses(jd_from, jd_to, step);
    let lunar = find_lunar_eclipses(jd_from, jd_to, step);

    println!("SOLARES ({})", solar.len());
    print_events(&solar, /* is_solar */ true);

    println!("\nLUNARES ({})", lunar.len());
    print_events(&lunar, false);
}

fn print_events(events: &[EclipseEvent], is_solar: bool) {
    if events.is_empty() {
        println!("  (vacío)");
        return;
    }
    println!(
        "  {:<20}  {:<12}  {:>10}  {:>10}",
        "máximo (UTC aprox)", "tipo", "magnitud", "duración_h"
    );
    println!("  {}", "─".repeat(58));
    for ev in events {
        let (y, mo, d, h, mi) = jd_to_calendar(ev.jd_mid);
        let kind = if is_solar {
            format!("{:?}", ev.kind_max_solar.unwrap())
        } else {
            format!("{:?}", ev.kind_max_lunar.unwrap())
        };
        println!(
            "  {:04}-{:02}-{:02} {:02}:{:02}    {:<12}  {:>10.3}  {:>10.2}",
            y, mo, d, h, mi, kind, ev.magnitude_max, ev.duration_hours
        );
    }
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
