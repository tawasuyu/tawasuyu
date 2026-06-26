//! Diag: para cada PNG con DOS ventanas lado a lado (master-stack), mide por
//! mitad izquierda/derecha: (1) la luma del contenido central → certifica el
//! *dim* de no-enfocadas; (2) el «azul» del borde superior (B − R promedio de
//! una banda fina arriba de cada ventana) → certifica el *glow* de foco
//! (border_normal gris ↔ border_focus azul). Sin mirar píxeles (regla 8).
//! Uso: cargo run -p mirada-compositor --example png_two_window_fx -- a.png b.png …

fn luma(p: [u8; 4]) -> f32 {
    0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
}

// ¿cerca de border_focus (92,143,235)? → el marco azul del foco (glow).
fn is_focus_blue(p: [u8; 4]) -> bool {
    (p[0] as i32 - 92).abs() <= 14
        && (p[1] as i32 - 143).abs() <= 14
        && (p[2] as i32 - 235).abs() <= 14
}

fn main() {
    println!("{:>16}  | izq: luma  azul-px | der: luma  azul-px", "frame");
    for path in std::env::args().skip(1) {
        let img = match image::open(&path) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                println!("{path}: ERR {e}");
                continue;
            }
        };
        let (w, h) = (img.width(), img.height());
        // Mitades: izquierda [0,w/2), derecha [w/2,w).
        let halves = [(0u32, w / 2), (w / 2, w)];
        let mut out = String::new();
        for (hx0, hx1) in halves {
            // Contenido: caja central de la mitad.
            let (cx0, cx1) = (hx0 + (hx1 - hx0) / 4, hx1 - (hx1 - hx0) / 4);
            let (cy0, cy1) = (h / 3, 2 * h / 3);
            let mut lsum = 0f64;
            let mut ln = 0u64;
            for y in cy0..cy1 {
                for x in cx0..cx1 {
                    lsum += luma(img.get_pixel(x, y).0) as f64;
                    ln += 1;
                }
            }
            let content_luma = (lsum / ln.max(1) as f64) as f32;
            // Glow: cuántos píxeles de la mitad son del azul de foco (el marco).
            let mut blue_px = 0u64;
            for y in 0..h {
                for x in hx0..hx1 {
                    if is_focus_blue(img.get_pixel(x, y).0) {
                        blue_px += 1;
                    }
                }
            }
            out.push_str(&format!("  luma={content_luma:6.1}  azul-px={blue_px:>6}  |"));
        }
        let base = path.rsplit('/').next().unwrap_or(&path);
        println!("{base:>16} |{out}");
    }
}
