//! Rasterizado CPU de las capas de texto de `tullpu`.
//!
//! Una capa de texto (`ClaseCapa::Texto(ParamsTexto)`) guarda el string
//! editable; su `contenido` es el texto **ya rasterizado** a un buffer Rgba8
//! del tamaño del lienzo. Así el compositor (`tullpu-render`) la trata igual
//! que una capa de píxeles —sin saber tipografía— y exporta/funde sin casos
//! especiales. Acá vive la rasterización con `fontdue` sobre la sans embebida
//! (`llimphi_text::SANS_FONT_BYTES`), la misma fuente que el render normal.

use fontdue::{Font, FontSettings};
use tullpu_core::ParamsTexto;

/// Rasteriza `params.texto` a un buffer Rgba8 `W·H` con la esquina superior-
/// izquierda del bloque en `(params.x, params.y)`. Soporta múltiples líneas
/// (separadas por `\n`). Fuera del buffer se recorta. Devuelve un buffer
/// transparente si el texto está vacío o la fuente no carga.
pub(crate) fn rasterizar_texto(params: &ParamsTexto, w: u32, h: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
    if params.texto.is_empty() {
        return buf;
    }
    let Ok(font) = Font::from_bytes(
        llimphi_ui::llimphi_text::SANS_FONT_BYTES,
        FontSettings::default(),
    ) else {
        return buf;
    };
    let size = params.tamano.clamp(4.0, 512.0);
    let lm = font.horizontal_line_metrics(size);
    let ascent = lm.map(|m| m.ascent).unwrap_or(size);
    let line_h = lm.map(|m| m.new_line_size).unwrap_or(size * 1.2);
    let [cr, cg, cb, ca] = params.color;
    let w_i = w as i32;
    let h_i = h as i32;

    let mut baseline = params.y as f32 + ascent;
    for linea in params.texto.split('\n') {
        let mut pen_x = params.x as f32;
        for ch in linea.chars() {
            let (m, bitmap) = font.rasterize(ch, size);
            // Esquina superior-izquierda del glifo: `xmin` es el side-bearing
            // izquierdo; `ymin` la distancia de la baseline al borde inferior
            // del glifo (positivo = arriba), así el top = baseline - ymin - alto.
            let gx0 = pen_x.round() as i32 + m.xmin;
            let gy0 = baseline.round() as i32 - m.ymin - m.height as i32;
            let bw = m.width as i32;
            for gy in 0..m.height as i32 {
                let py = gy0 + gy;
                if py < 0 || py >= h_i {
                    continue;
                }
                for gx in 0..bw {
                    let px = gx0 + gx;
                    if px < 0 || px >= w_i {
                        continue;
                    }
                    let cov = bitmap[(gy * bw + gx) as usize];
                    if cov == 0 {
                        continue;
                    }
                    let a = (cov as f32 / 255.0) * (ca as f32 / 255.0);
                    if a <= 0.0 {
                        continue;
                    }
                    let di = ((py * w_i + px) as usize) * 4;
                    // src-over del glifo sobre lo acumulado (típicamente vacío,
                    // pero glifos solapados se componen bien igual).
                    let da = buf[di + 3] as f32 / 255.0;
                    let inv = 1.0 - a;
                    let out_a = a + da * inv;
                    if out_a > 0.0 {
                        for (k, &sc) in [cr, cg, cb].iter().enumerate() {
                            let dc = buf[di + k] as f32;
                            buf[di + k] =
                                ((sc as f32 * a + dc * da * inv) / out_a).round().clamp(0.0, 255.0) as u8;
                        }
                        buf[di + 3] = (out_a * 255.0).round() as u8;
                    }
                }
            }
            pen_x += m.advance_width;
        }
        baseline += line_h;
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texto_vacio_es_transparente() {
        let p = ParamsTexto { texto: String::new(), tamano: 24.0, color: [255, 255, 255, 255], x: 0, y: 0 };
        let buf = rasterizar_texto(&p, 16, 16);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn texto_pinta_algun_pixel_opaco() {
        // Una "M" grande en un lienzo chico debe dejar píxeles con alfa > 0.
        let p = ParamsTexto { texto: "M".into(), tamano: 32.0, color: [255, 0, 0, 255], x: 2, y: 2 };
        let buf = rasterizar_texto(&p, 48, 48);
        let opacos = buf.chunks_exact(4).filter(|px| px[3] > 0).count();
        assert!(opacos > 0, "la M debería pintar píxeles");
        // Y donde pinta, el color tiende al rojo.
        let rojo = buf.chunks_exact(4).any(|px| px[3] > 0 && px[0] > px[1] && px[0] > px[2]);
        assert!(rojo, "el texto sale del color pedido");
    }
}
