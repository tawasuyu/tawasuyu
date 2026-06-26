//! Diag: imprime la luminancia media de la región central (50 %) de cada PNG
//! que se le pase. Para certificar rampas de efectos (fade-in sube, fade-close
//! baja) como serie numérica, sin mirar píxeles (regla 8).
//! Uso: cargo run -p mirada-compositor --example png_region_luma -- f0.png f1.png …

fn main() {
    let mut prev: Option<f32> = None;
    for path in std::env::args().skip(1) {
        let img = match image::open(&path) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                println!("{path}: ERR {e}");
                continue;
            }
        };
        let (w, h) = (img.width(), img.height());
        let (x0, x1) = (w / 4, 3 * w / 4);
        let (y0, y1) = (h / 4, 3 * h / 4);
        let mut sum = 0f64;
        let mut n = 0u64;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = img.get_pixel(x, y).0;
                // Luma Rec.601.
                sum += 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64;
                n += 1;
            }
        }
        let luma = (sum / n as f64) as f32;
        let delta = prev.map(|p| luma - p).unwrap_or(0.0);
        let base = path.rsplit('/').next().unwrap_or(&path);
        println!("{base:>16}  luma-central={luma:6.2}  Δ={delta:+6.2}");
        prev = Some(luma);
    }
}
