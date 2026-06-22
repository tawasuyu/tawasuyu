//! Volcado antes/después del realce de framebuffer (Fase 4-post).
//!
//! Bootea doomgeneric, warpea a E1M1, asienta la cámara en el spawn, toma el
//! framebuffer 640×400 y escribe DOS PNG: el crudo (`Enhance::OFF`) y el
//! realzado (`Enhance::RICH`). Sin GPU — sólo el motor C + ops de píxel.
//!
//! ```sh
//! cargo run -p supay-doom-llimphi --example fb_enhance_shot --release
//! # → /tmp/supay_fb_off.png  y  /tmp/supay_fb_rich.png
//! ```

use supay_core::{DoomEngine, DOOM_HEIGHT, DOOM_PIXELS, DOOM_WIDTH};
use supay_render_llimphi::postproc::{enhance_framebuffer, Enhance};

fn fb_to_rgba(fb: &[u32]) -> Vec<u8> {
    let mut out = vec![0u8; DOOM_PIXELS * 4];
    for (i, px) in fb.iter().enumerate() {
        let o = i * 4;
        out[o] = ((px >> 16) & 0xff) as u8;
        out[o + 1] = ((px >> 8) & 0xff) as u8;
        out[o + 2] = (px & 0xff) as u8;
        out[o + 3] = 0xff;
    }
    out
}

fn write_png(path: &str, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("crear png");
    let w = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(w, DOOM_WIDTH as u32, DOOM_HEIGHT as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
    eprintln!("escrito {path}");
}

fn main() {
    let mut engine = DoomEngine::new(vec![
        "supay".into(),
        "-iwad".into(),
        "doom1.wad".into(),
        "-warp".into(),
        "1".into(),
        "1".into(),
    ]);
    if !engine.real {
        eprintln!("fb_enhance_shot: motor stub (¿falta doom1.wad en cwd?). Abortando.");
        std::process::exit(1);
    }
    // Asentar: ~40 ticks para que el motor cargue el mapa y pinte el spawn.
    for _ in 0..40 {
        engine.tick();
    }
    let fb = engine.framebuffer();
    let raw = fb_to_rgba(&fb);

    const STATUSBAR_PX: usize = DOOM_HEIGHT * 32 / 200;
    let mut off = raw.clone();
    enhance_framebuffer(&mut off, DOOM_WIDTH, DOOM_HEIGHT, &Enhance::OFF, STATUSBAR_PX);
    write_png("/tmp/supay_fb_off.png", &off);

    let mut rich = raw.clone();
    enhance_framebuffer(&mut rich, DOOM_WIDTH, DOOM_HEIGHT, &Enhance::RICH, STATUSBAR_PX);
    write_png("/tmp/supay_fb_rich.png", &rich);

    // Stat numérica para certificar sin mirar: el realce sube el brillo medio
    // en las zonas claras (bloom) y cambia el spread de color (saturación).
    let mean = |b: &[u8]| {
        let s: u64 = b.iter().step_by(4).map(|&x| x as u64).sum();
        s as f64 / (DOOM_PIXELS as f64)
    };
    eprintln!(
        "R medio: off={:.1}  rich={:.1}",
        mean(&off),
        mean(&rich)
    );
}
