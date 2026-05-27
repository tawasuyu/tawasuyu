//! Showcase CLI: curva de marea de equilibrio Sol+Luna en 24h desde el
//! Callao, al 2026-06-15. Imprime una tabla con altura por hora + un
//! gráfico ASCII rudimentario.
//!
//! Corré con: `cargo run -p cosmos-tides --example tides_callao_demo
//! --release`.

use cosmos_core::Location;
use cosmos_tides::tide_reading;
use cosmos_time::TDB;

fn main() {
    let callao = Location::from_degrees(-12.07, -77.13, 0.0).unwrap();
    println!("=== Marea de equilibrio (Sol+Luna) · Callao · 2026-06-15 ===");
    println!(
        "lat={:.3}°  lon={:.3}°  modelo: 2do polinomio de Legendre · no incluye respuesta hidrodinámica local\n",
        callao.latitude_degrees(),
        callao.longitude_degrees()
    );
    println!(
        "{:<5}  {:>10}  {:>10}  {:>10}  {:>10}  altura total",
        "TDB", "lunar (m)", "solar (m)", "z_lun°", "z_sol°"
    );
    println!("{}", "─".repeat(80));

    let mut total_range = (f64::MAX, f64::MIN);
    let mut samples: Vec<(u32, f64, f64, f64, f64, f64)> = Vec::with_capacity(24);
    for h in 0..24u32 {
        let iso = format!("2026-06-15T{:02}:00:00", h);
        let r = tide_reading(&iso.parse().unwrap(), &callao);
        samples.push((
            h,
            r.lunar.height_m,
            r.solar.height_m,
            r.lunar.zenith_deg,
            r.solar.zenith_deg,
            r.total_height_m,
        ));
        if r.total_height_m < total_range.0 {
            total_range.0 = r.total_height_m;
        }
        if r.total_height_m > total_range.1 {
            total_range.1 = r.total_height_m;
        }
    }

    let lo = total_range.0;
    let hi = total_range.1;
    let span = (hi - lo).max(1e-9);
    let bar_width: usize = 30;
    for (h, lunar, solar, zl, zs, tot) in &samples {
        let pos = ((tot - lo) / span * bar_width as f64).round() as usize;
        let pos = pos.min(bar_width);
        let mut bar = String::with_capacity(bar_width);
        for i in 0..bar_width {
            bar.push(if i == pos { '●' } else { '·' });
        }
        println!(
            "{:>2}h    {:>10.4}  {:>10.4}  {:>10.2}  {:>10.2}  {:>+9.4} m  {}",
            h, lunar, solar, zl, zs, tot, bar
        );
    }
    println!(
        "\nrango total del día: {:+.4} m  →  {:+.4} m   (amplitud {:.4} m)",
        lo,
        hi,
        hi - lo
    );
    println!(
        "referencia: pico lunar ecuatorial ~ 0.36 m, solar ~ 0.16 m. \
         Marea observada en costa es ~5–10× mayor por amplificación."
    );
}
