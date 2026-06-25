//! `estabilizar` — el lazo cerrado en acción, headless y certificado por texto.
//!
//! Siembra un mundo de dominium con una palanca **mal calibrada a propósito**
//! (que en lazo abierto explotaría o colapsaría) y deja que el
//! `StabilityController` la corrija para sostener un setpoint. Imprime la
//! trayectoria `N(t)` + la palanca como CSV y un resumen — sin ventana, sin PNG
//! (regla #8: esto se certifica con números).
//!
//! `cargo run -p dominium-control --example estabilizar --release -- [setpoint] [regrowth0] [ticks] [grid]`
//!
//! Ej.: `... -- 600 0.25 1200 80`  (arranque caliente, lo doma)
//!      `... -- 600 0.0  1200 80`  (arranque frío, lo rescata)

use dominium_control::{density_ceiling, run_controlled, Lever, StabilityController};
use dominium_core::{worldgen, Conceptos, SimParams};

fn main() {
    let setpoint: f32 = arg(1).unwrap_or(600.0);
    let regrowth0: f32 = arg(2).unwrap_or(0.25);
    let ticks: u32 = arg(3).map(|v| v as u32).unwrap_or(1200);
    let grid: usize = arg(4).map(|v| v as usize).unwrap_or(80);

    let seedfn = move || worldgen::seed(0xD0_C7_5EED, grid, grid * 5, Conceptos::default());
    let mut world = seedfn();
    let mut params = SimParams::default();
    // Frenos defensivos que la app enciende (campos/energía acotados).
    params.field_saturation = 150.0;
    params.max_energy = 400.0;
    // Densidad-dependencia activa → hay techo ecológico analítico que leer.
    params.density_block = 12;
    params.density_cap = 16;
    // La palanca arranca MAL calibrada: el controlador parte de acá.
    params.regrowth_rate = regrowth0;

    let ceiling = density_ceiling(&world.grid, &params);
    eprintln!(
        "setpoint={setpoint:.0} · palanca=regrowth (arranque {regrowth0:.3}) · grid={grid}² · techo ecológico={}",
        ceiling.map(|c| c.to_string()).unwrap_or_else(|| "—".into())
    );

    // Rango de palanca ancho: le damos autoridad para alcanzar setpoints altos
    // (con el rango default 0..0.25 el caudal de materia topa antes de ~400).
    let mut ctrl = StabilityController::new(setpoint, Lever::Regrowth).with_bounds(0.0, 0.6);
    let (traj, lever_final) =
        run_controlled(&mut world, &mut params, &mut ctrl, ticks, seedfn);

    // CSV cada 40 ticks (trayectoria legible sin inundar).
    println!("tick,N,setpoint,lever_regrowth");
    let step = 40usize;
    for (t, &n) in traj.iter().enumerate() {
        if t % step == 0 || t + 1 == traj.len() {
            // La palanca exacta por-tick no se guarda; reportamos el valor final
            // en la última fila (es donde el lazo ya convergió).
            let lev = if t + 1 == traj.len() {
                format!("{lever_final:.4}")
            } else {
                String::from("")
            };
            println!("{t},{n},{setpoint:.0},{lev}");
        }
    }

    // Resumen: error relativo medio en el último tercio (régimen).
    let start = traj.len() * 2 / 3;
    let tail = &traj[start..];
    let err: f32 = tail
        .iter()
        .map(|&n| (n as f32 - setpoint).abs() / setpoint)
        .sum::<f32>()
        / tail.len() as f32;
    let mean: f32 = tail.iter().map(|&n| n as f32).sum::<f32>() / tail.len() as f32;
    eprintln!(
        "resumen: N_medio_cola={mean:.0} · lever_final={lever_final:.4} · err_cola={:.1}%",
        err * 100.0
    );
}

fn arg(i: usize) -> Option<f32> {
    std::env::args().nth(i).and_then(|s| s.parse().ok())
}
