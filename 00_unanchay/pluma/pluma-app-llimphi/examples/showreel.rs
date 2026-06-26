//! **Showreel** de pluma — render headless determinista del editor multilienzo
//! real, para el README del repo standalone. Reusa la `vista()` de la app sobre
//! un modelo sintético con tres cuerpos paralelos (es → qu → en) alineados por
//! hebras; la lógica vive en `pluma_app::showreel` (compartida con el binario,
//! que la expone también como `pluma-app-llimphi --showreel`).
//!
//! ```text
//! cargo run -p pluma-app-llimphi --example showreel --release -- [out_dir] [n] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames_pluma`, `n=300`, `W=1600`, `H=900`.

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args
        .next()
        .unwrap_or_else(|| "showreel_frames_pluma".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    pluma_app::showreel::render_frames(&out_dir, n, w, h);
}
