//! Showcase CLI: "qué se ve esta noche desde Lima al 2026-05-27 a
//! las 23:00 TDB".
//!
//! Imprime una tabla ordenada por altitud de los 10 cuerpos del
//! sistema solar — los que están sobre el horizonte arriba, los que
//! están debajo abajo con su altitud negativa.
//!
//! Corré con: `cargo run -p cosmos-skywatch --example
//! skywatch_lima_demo --release`.

use cosmos_core::Location;
use cosmos_skywatch::{sky_positions_all, Body, SkyPosition};
use cosmos_time::TDB;

fn main() {
    let lima = Location::from_degrees(-12.05, -77.05, 150.0).expect("lima válida");
    let tdb: TDB = "2026-05-27T23:00:00".parse().expect("ISO 8601");

    let mut all = sky_positions_all(&tdb, &lima);
    // Ordena por altitud descendente — los más altos arriba.
    all.sort_by(|a, b| {
        b.1
            .visibility_score()
            .partial_cmp(&a.1.visibility_score())
            .unwrap()
    });

    println!("=== Skywatch · Lima · 2026-05-27 23:00 TDB ===");
    println!("lat={:.3}°  lon={:.3}°  alt={} m\n", -12.05, -77.05, 150);
    println!(
        "{:<10}  {:>8}  {:>8}  {:>8}  {:>8}  {:>11}  visible",
        "body", "alt°", "az°", "RA°", "Dec°", "d (au)"
    );
    println!("{}", "─".repeat(72));
    for (body, pos) in &all {
        print_row(body, pos);
    }

    let visibles = all
        .iter()
        .filter(|(_, p)| p.above_horizon)
        .count();
    println!(
        "\nresumen: {visibles}/{} cuerpos sobre el horizonte.",
        all.len()
    );
}

fn print_row(body: &Body, pos: &SkyPosition) {
    let mark = if pos.above_horizon { "●" } else { "·" };
    println!(
        "{:<10}  {:>8.2}  {:>8.2}  {:>8.2}  {:>8.2}  {:>11.6}  {mark}",
        body.canonical(),
        pos.altitude_deg,
        pos.azimuth_deg,
        pos.right_ascension_deg,
        pos.declination_deg,
        pos.distance_au
    );
}
