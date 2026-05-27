//! Showcase CLI: sombra hora por hora a lo largo del día desde Lima
//! al 2026-05-27, para un gnomon de 1.00 m. Sirve para verificar que
//! la sombra crece hacia el ocaso y "salta" al lado opuesto al cruzar
//! el meridiano (típico de un cuadrante solar).
//!
//! Corré con: `cargo run -p cosmos-sundial --example sundial_lima_demo
//! --release`.

use cosmos_core::Location;
use cosmos_sundial::sundial_reading;
use cosmos_time::TDB;

fn main() {
    let lima = Location::from_degrees(-12.05, -77.05, 150.0).unwrap();
    println!("=== Cuadrante solar · Lima · 2026-05-27 · gnomon h = 1.00 m ===");
    println!(
        "lat={:.3}°  lon={:.3}°\n",
        lima.latitude_degrees(),
        lima.longitude_degrees()
    );
    println!(
        "{:<7}  {:>8}  {:>8}  {:>10}  {:>10}  {:>10}",
        "TDB", "alt°", "az°", "HA°", "sombra_az", "sombra_m"
    );
    println!("{}", "─".repeat(60));
    for hour in 10..=23u32 {
        let iso = format!("2026-05-27T{:02}:00:00", hour);
        let tdb: TDB = iso.parse().unwrap();
        let r = sundial_reading(&tdb, &lima);
        let sun = r.sun;
        let s_az = r
            .shadow_azimuth_deg
            .map(|a| format!("{a:>10.2}"))
            .unwrap_or_else(|| "        —".to_string());
        let s_l = r
            .shadow_length_for(1.0)
            .map(|l| format!("{l:>10.2}"))
            .unwrap_or_else(|| "        —".to_string());
        println!(
            "{:<7}  {:>8.2}  {:>8.2}  {:>10.2}  {}  {}",
            format!("{hour:02}h"),
            sun.altitude_deg,
            sun.azimuth_deg,
            r.hour_angle_deg,
            s_az,
            s_l
        );
    }
}
