//! Diag: ¿existe el color de foco (border_focus azul) en la captura, y dónde?
//! Cuenta píxeles cercanos a border_focus (92,143,235) y a border_normal
//! (56,56,69), y da su bounding-box. Responde si el glow de foco se pinta.
//! Uso: cargo run -p mirada-compositor --example png_find_color -- captura.png

fn near(p: [u8; 4], c: [u8; 3], tol: i32) -> bool {
    (p[0] as i32 - c[0] as i32).abs() <= tol
        && (p[1] as i32 - c[1] as i32).abs() <= tol
        && (p[2] as i32 - c[2] as i32).abs() <= tol
}

fn main() {
    let path = std::env::args().nth(1).expect("png");
    let img = image::open(&path).expect("abrir").to_rgba8();
    let (w, h) = (img.width(), img.height());
    for (name, c) in [("border_focus(azul)", [92u8, 143, 235]), ("border_normal(gris)", [56, 56, 69])] {
        let (mut n, mut x0, mut y0, mut x1, mut y1) = (0u64, w, h, 0u32, 0u32);
        for y in 0..h {
            for x in 0..w {
                if near(img.get_pixel(x, y).0, c, 12) {
                    n += 1;
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        if n > 0 {
            println!("{name:>20}: {n:>7} px  bbox=({x0},{y0})..({x1},{y1})");
        } else {
            println!("{name:>20}: 0 px (no aparece)");
        }
    }
    println!("(imagen {w}x{h})");
}
