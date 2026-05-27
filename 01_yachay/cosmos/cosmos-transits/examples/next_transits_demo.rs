//! Showcase CLI: barrer 2026-01-01..2040-01-01 buscando tránsitos
//! de Mercurio y Venus sobre el Sol. Imprime una tabla con la fecha
//! del centro, la separación mínima y la duración.
//!
//! El próximo tránsito de Mercurio es el **2032-11-13** (verificado
//! contra NASA/JPL). El siguiente tránsito de Venus es el **2117**
//! — así que el barrido de Venus debería salir vacío.
//!
//! Corré con: `cargo run -p cosmos-transits --example
//! next_transits_demo --release`.

use cosmos_time::JulianDate;
use cosmos_transits::{find_transits, InnerPlanet, TransitEvent};

fn main() {
    let jd_from = JulianDate::from_calendar(2026, 1, 1, 0, 0, 0.0).to_f64();
    let jd_to = JulianDate::from_calendar(2040, 1, 1, 0, 0, 0.0).to_f64();
    let step = 1.0 / 24.0; // 1 hora

    println!("=== Tránsitos planetarios sobre el Sol — 2026-01-01 → 2040-01-01 ===");
    println!("paso de muestreo: 1 hora · ventana ~14 años\n");

    let mercury = find_transits(&InnerPlanet::Mercury, jd_from, jd_to, step);
    let venus = find_transits(&InnerPlanet::Venus, jd_from, jd_to, step);

    println!("MERCURIO ({} eventos)", mercury.len());
    print_events(&mercury);

    println!("\nVENUS ({} eventos)", venus.len());
    if venus.is_empty() {
        println!(
            "  (vacío — el próximo tránsito de Venus es el 2117-12-11. \
             El par 2117/2125 cerrará la serie que comenzó en 1631.)"
        );
    } else {
        print_events(&venus);
    }
}

fn print_events(events: &[TransitEvent]) {
    if events.is_empty() {
        return;
    }
    println!(
        "  {:<20}  {:>12}  {:>10}",
        "centro (UTC aprox)", "sep_min (°)", "duración_h"
    );
    println!("  {}", "─".repeat(50));
    for ev in events {
        let (y, mo, d, h, mi) = jd_to_calendar(ev.jd_mid);
        println!(
            "  {:04}-{:02}-{:02} {:02}:{:02}    {:>10.6}    {:>10.2}",
            y, mo, d, h, mi, ev.min_separation_deg, ev.duration_hours
        );
    }
}

/// Conversión rápida JD → fecha calendario gregoriana. Suficiente para
/// imprimir; no busca precisión sub-segundo.
fn jd_to_calendar(jd: f64) -> (i32, u32, u32, u32, u32) {
    // Fliegel & Van Flandern 1968.
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
