//! Carga de imágenes PNG para el splash y blit centrado/escalado sobre el
//! framebuffer. Pura (sin DRM): decodifica con el crate `png` (Rust puro, sirve
//! en el initramfs musl) y compone con el mismo `XRGB8888` que el resto.
//!
//! La imagen se escala **preservando proporción** para entrar en la pantalla
//! (contain), centrada; el resto queda en el color de fondo `bg`. El `fade`
//! ∈[0,1] funde todo hacia `bg` (handoff Fase 2), igual que el splash nativo.

use std::path::Path;

/// Imagen decodificada en RGBA8 lineal por filas.
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>, // w*h*4
}

impl Image {
    fn px(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
        let i = (y * self.w + x) * 4;
        (self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3])
    }
}

/// Decodifica un PNG a RGBA8. `None` ante cualquier error (best-effort: el
/// caller cae al splash nativo).
pub fn load_png(path: &Path) -> Option<Image> {
    let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let mut dec = png::Decoder::new(file);
    // Normaliza a 8 bits/canal; expande indexed/grayscale de bajo bit.
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let src = &buf[..info.buffer_size()];
    let rgba = to_rgba8(src, w, h, info.color_type)?;
    Some(Image { w, h, rgba })
}

/// Convierte el buffer decodificado a RGBA8 según el tipo de color.
fn to_rgba8(src: &[u8], w: usize, h: usize, ct: png::ColorType) -> Option<Vec<u8>> {
    let n = w * h;
    let mut out = vec![0u8; n * 4];
    match ct {
        png::ColorType::Rgba => out.copy_from_slice(&src[..n * 4]),
        png::ColorType::Rgb => {
            for i in 0..n {
                out[i * 4] = src[i * 3];
                out[i * 4 + 1] = src[i * 3 + 1];
                out[i * 4 + 2] = src[i * 3 + 2];
                out[i * 4 + 3] = 255;
            }
        }
        png::ColorType::Grayscale => {
            for i in 0..n {
                let g = src[i];
                out[i * 4] = g;
                out[i * 4 + 1] = g;
                out[i * 4 + 2] = g;
                out[i * 4 + 3] = 255;
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for i in 0..n {
                let g = src[i * 2];
                out[i * 4] = g;
                out[i * 4 + 1] = g;
                out[i * 4 + 2] = g;
                out[i * 4 + 3] = src[i * 2 + 1];
            }
        }
        png::ColorType::Indexed => return None, // normalize_to_color8 ya expande
    }
    Some(out)
}

/// Pinta `img` centrado y escalado-a-contener sobre `buf` (XRGB8888), con el
/// resto en `bg` y todo fundido hacia `bg` por `fade`.
pub fn blit_fit(
    buf: &mut [u8],
    w: usize,
    h: usize,
    pitch: usize,
    img: &Image,
    bg: (u8, u8, u8),
    fade: f32,
) {
    let fade = fade.clamp(0.0, 1.0);
    // Escala "contain": entra entero, preservando proporción.
    let scale = (w as f32 / img.w as f32).min(h as f32 / img.h as f32).max(f32::MIN_POSITIVE);
    let dw = ((img.w as f32 * scale) as usize).clamp(1, w);
    let dh = ((img.h as f32 * scale) as usize).clamp(1, h);
    let ox = (w - dw) / 2;
    let oy = (h - dh) / 2;

    for y in 0..h {
        let row = y * pitch;
        if row >= buf.len() {
            break;
        }
        let in_y = y >= oy && y < oy + dh;
        let sy = if in_y { ((y - oy) * img.h) / dh } else { 0 };
        for x in 0..w {
            let idx = row + x * 4;
            if idx + 4 > buf.len() {
                break;
            }
            let col = if in_y && x >= ox && x < ox + dw {
                let sx = ((x - ox) * img.w) / dw;
                let (r, g, b, a) = img.px(sx.min(img.w - 1), sy.min(img.h - 1));
                // Composita la imagen sobre bg según su alfa.
                over(bg, (r, g, b), a)
            } else {
                bg
            };
            // Fade-out hacia bg para el handoff.
            let col = lerp(col, bg, fade);
            buf[idx] = col.2;
            buf[idx + 1] = col.1;
            buf[idx + 2] = col.0;
            buf[idx + 3] = 0;
        }
    }
}

/// `src` sobre `dst` con alfa `a` (0..255).
fn over(dst: (u8, u8, u8), src: (u8, u8, u8), a: u8) -> (u8, u8, u8) {
    let t = a as f32 / 255.0;
    lerp(dst, src, t)
}

fn lerp(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Codifica un PNG RGBA `w×h` de color sólido en memoria.
    fn png_solido(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            let data: Vec<u8> = (0..(w * h)).flat_map(|_| rgba).collect();
            wr.write_image_data(&data).unwrap();
        }
        out
    }

    #[test]
    fn decodifica_png_rgba() {
        let bytes = png_solido(3, 2, [10, 20, 30, 255]);
        let path = std::env::temp_dir().join(format!("arje-img-{}.png", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let img = load_png(&path).expect("debe decodificar");
        assert_eq!((img.w, img.h), (3, 2));
        assert_eq!(img.px(0, 0), (10, 20, 30, 255));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn blit_centra_y_rellena_con_bg() {
        // Imagen 1×1 roja sobre pantalla 16×16 con bg azul: el centro es rojo,
        // las esquinas bg (la imagen 1×1 escala a ~16×16 → casi todo rojo, así
        // que uso una imagen "cuadrada" chica para dejar margen).
        let img = Image { w: 2, h: 2, rgba: vec![200, 0, 0, 255, 200, 0, 0, 255, 200, 0, 0, 255, 200, 0, 0, 255] };
        let (w, h, pitch) = (32usize, 16usize, 32 * 4);
        let bg = (0, 0, 40);
        let mut buf = vec![0u8; pitch * h];
        blit_fit(&mut buf, w, h, pitch, &img, bg, 0.0);
        let at = |x: usize, y: usize| {
            let i = y * pitch + x * 4;
            (buf[i + 2], buf[i + 1], buf[i]) // R,G,B
        };
        // Centro: dentro de la imagen (cuadrada centrada) → rojo.
        assert_eq!(at(w / 2, h / 2), (200, 0, 0));
        // Esquina izquierda (fuera del cuadrado centrado) → bg.
        assert_eq!(at(0, h / 2), bg);
    }

    #[test]
    fn fade_uno_funde_la_imagen_a_bg() {
        let img = Image { w: 1, h: 1, rgba: vec![200, 0, 0, 255] };
        let (w, h, pitch) = (8usize, 8usize, 8 * 4);
        let bg = (0, 0, 40);
        let mut buf = vec![0u8; pitch * h];
        blit_fit(&mut buf, w, h, pitch, &img, bg, 1.0);
        let i = (h / 2) * pitch + (w / 2) * 4;
        assert_eq!((buf[i + 2], buf[i + 1], buf[i]), bg, "fade=1 deja todo en bg");
    }
}
