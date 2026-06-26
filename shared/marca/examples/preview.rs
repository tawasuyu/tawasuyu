//! Render de un cuadro del wallpaper de marca animado a un PPM, para
//! previsualizarlo sin levantar el compositor.
//!
//! `cargo run -p marca --example preview -- [t] [w] [h] [salida.ppm]`
//! (defaults: t=2.0, 1280×720, /tmp/marca-preview.ppm). Convertir a PNG con
//! `ffmpeg -i salida.ppm salida.png` si hace falta mirarlo.

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let t: f32 = a.first().and_then(|s| s.parse().ok()).unwrap_or(2.0);
    let w: u32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1280);
    let h: u32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(720);
    let out = a.get(3).cloned().unwrap_or_else(|| "/tmp/marca-preview.ppm".into());

    let bgra = marca::animated_frame(t, w, h);
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    ppm.reserve((w * h * 3) as usize);
    for px in bgra.chunks_exact(4) {
        ppm.push(px[2]); // R
        ppm.push(px[1]); // G
        ppm.push(px[0]); // B
    }
    std::fs::write(&out, ppm).expect("escribir PPM");
    println!("wallpaper de marca animado · t={t}s {w}×{h} → {out}");
}
