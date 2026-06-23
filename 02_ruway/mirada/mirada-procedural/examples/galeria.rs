//! Vuelca los seis patrones procedurales a PNG y los certifica con un histograma
//! de color impreso como TEXTO (sin necesidad de abrir las imágenes): cada
//! patrón debe repartir varios colores de la paleta. Es la prueba visual barata
//! del generador.
//!
//! `cargo run -p mirada-procedural --example galeria -- [dir_salida]`

use std::collections::HashSet;
use std::fs::File;
use std::io::BufWriter;

use mirada_procedural::{generate_rgba, Pattern};

const W: u32 = 480;
const H: u32 = 270;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "/tmp".to_string());
    // Paleta cálida/fría contrastada para que el reparto se note.
    let palette = [
        [0x1a, 0x1c, 0x2e],
        [0x4e, 0x3d, 0x8a],
        [0xe0, 0x6c, 0x75],
        [0xf2, 0xb1, 0x79],
        [0x56, 0xc5, 0xd0],
    ];
    let mut total_ok = 0;
    for p in Pattern::ALL {
        let px = generate_rgba(p, &palette, W, H, 11);
        let path = format!("{dir}/proc_{}.png", p.slug());
        write_png(&path, &px);
        let mut buckets = HashSet::new();
        for c in px.chunks_exact(4) {
            buckets.insert((c[0] / 48, c[1] / 48, c[2] / 48));
        }
        let ok = buckets.len() >= 4;
        total_ok += ok as usize;
        eprintln!(
            "galeria: {:<8} → {path}  ({} cubos de color, {})",
            p.label(),
            buckets.len(),
            if ok { "OK" } else { "PLANO!" }
        );
    }
    assert_eq!(total_ok, Pattern::ALL.len(), "algún patrón salió plano");
    eprintln!("galeria: los {} patrones reparten color ✓", Pattern::ALL.len());
}

fn write_png(path: &str, rgba: &[u8]) {
    let file = File::create(path).expect("crear png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}
