//! Rasterizado de texto del compositor.
//!
//! El compositor sólo sabía pintar rectángulos sólidos y superficies de
//! clientes — no tenía fuentes. Este módulo rasteriza una cadena a un búfer
//! RGBA sobre CPU (con `ab_glyph`, un rasterizador puro-Rust ligero) que
//! luego smithay sube como textura (`MemoryRenderBuffer`). Es la base de la
//! barra de título y del menú: ambos necesitan dibujar etiquetas.
//!
//! El búfer sale en **ARGB8888** premultiplicado, que es lo que
//! `MemoryRenderBuffer::from_slice` con `Fourcc::Argb8888` espera; en memoria
//! little-endian eso son bytes en orden `[B, G, R, A]`.

use std::path::{Path, PathBuf};

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};

/// Rutas de fuentes del sistema que se prueban en orden si la config no fija
/// una. Cubre las familias habituales en Arch/Artix y derivados.
const FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
    "/usr/share/fonts/gnu-free/FreeSans.otf",
    "/usr/share/fonts/TTF/Hack-Regular.ttf",
];

/// Una cadena ya rasterizada: bytes ARGB8888 premultiplicados y su tamaño.
pub struct Rasterized {
    pub rgba: Vec<u8>,
    pub width: i32,
    pub height: i32,
}

/// Una fuente cargada para rasterizar etiquetas del compositor.
pub struct TextRenderer {
    font: FontVec,
}

impl TextRenderer {
    /// Carga una fuente desde un archivo concreto (el override de la config).
    pub fn from_path(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let font = FontVec::try_from_vec(bytes).ok()?;
        Some(Self { font })
    }

    /// Carga la primera fuente disponible: la de `preferred` (config) si
    /// existe, si no la primera de [`FONT_CANDIDATES`]. `None` si no hay
    /// ninguna — entonces el compositor simplemente no pinta etiquetas.
    pub fn system(preferred: Option<&str>) -> Option<Self> {
        let mut paths: Vec<PathBuf> = Vec::new();
        if let Some(p) = preferred {
            paths.push(PathBuf::from(p));
        }
        paths.extend(FONT_CANDIDATES.iter().map(PathBuf::from));
        paths.into_iter().find_map(|p| Self::from_path(&p))
    }

    /// Rasteriza `text` a `px` de alto con `color` (RGBA `0..=255`).
    /// Devuelve los píxeles ARGB8888 premultiplicados y el tamaño, o `None`
    /// si el texto queda vacío / sin glyphs visibles.
    pub fn rasterize(&self, text: &str, px: f32, color: [u8; 4]) -> Option<Rasterized> {
        let px = px.max(1.0);
        let scale = PxScale::from(px);
        let scaled = self.font.as_scaled(scale);
        let ascent = scaled.ascent();
        let height = (scaled.ascent() - scaled.descent()).ceil().max(1.0) as i32;

        // Layout: posiciona cada glyph en la línea base y junta sus contornos.
        let mut pen_x = 0.0f32;
        let mut max_x = 0.0f32;
        let mut outlines = Vec::new();
        for c in text.chars() {
            let gid = self.font.glyph_id(c);
            let glyph = gid.with_scale_and_position(scale, ab_glyph::point(pen_x, ascent));
            if let Some(o) = self.font.outline_glyph(glyph) {
                max_x = max_x.max(o.px_bounds().max.x);
                outlines.push(o);
            }
            pen_x += scaled.h_advance(gid);
        }
        let width = pen_x.ceil().max(max_x.ceil()).max(1.0) as i32;
        if outlines.is_empty() {
            return None;
        }

        let mut rgba = vec![0u8; (width * height * 4) as usize];
        let (cr, cg, cb, ca) = (
            color[0] as f32,
            color[1] as f32,
            color[2] as f32,
            color[3] as f32 / 255.0,
        );
        for o in &outlines {
            let b = o.px_bounds();
            let (ox, oy) = (b.min.x as i32, b.min.y as i32);
            o.draw(|gx, gy, cov| {
                let x = ox + gx as i32;
                let y = oy + gy as i32;
                if x < 0 || y < 0 || x >= width || y >= height {
                    return;
                }
                let a = (cov * ca).clamp(0.0, 1.0);
                if a <= 0.0 {
                    return;
                }
                let i = ((y * width + x) * 4) as usize;
                // Compuesto source-over premultiplicado sobre lo que haya
                // (los glyphs casi no se solapan, pero así es correcto).
                let inv = 1.0 - a;
                let sb = cb * a;
                let sg = cg * a;
                let sr = cr * a;
                let sa = a * 255.0;
                rgba[i] = (sb + rgba[i] as f32 * inv) as u8; // B
                rgba[i + 1] = (sg + rgba[i + 1] as f32 * inv) as u8; // G
                rgba[i + 2] = (sr + rgba[i + 2] as f32 * inv) as u8; // R
                rgba[i + 3] = (sa + rgba[i + 3] as f32 * inv) as u8; // A
            });
        }
        Some(Rasterized { rgba, width, height })
    }
}

/// Compone un pixel premultiplicado (orden de bytes **B,G,R,A**, igual que
/// [`TextRenderer::rasterize`]) sobre el buffer, con cobertura `cov`.
fn blend_px(rgba: &mut [u8], w: i32, h: i32, x: i32, y: i32, color: [u8; 4], cov: f32) {
    if x < 0 || y < 0 || x >= w || y >= h || cov <= 0.0 {
        return;
    }
    let a = (cov * (color[3] as f32 / 255.0)).clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let i = ((y * w + x) * 4) as usize;
    let inv = 1.0 - a;
    rgba[i] = (color[2] as f32 * a + rgba[i] as f32 * inv) as u8; // B
    rgba[i + 1] = (color[1] as f32 * a + rgba[i + 1] as f32 * inv) as u8; // G
    rgba[i + 2] = (color[0] as f32 * a + rgba[i + 2] as f32 * inv) as u8; // R
    rgba[i + 3] = (a * 255.0 + rgba[i + 3] as f32 * inv) as u8; // A
}

/// Dibuja un segmento con anti-aliasing (distancia al segmento → cobertura).
fn draw_line_aa(
    rgba: &mut [u8],
    w: i32,
    h: i32,
    p0: (f32, f32),
    p1: (f32, f32),
    thickness: f32,
    color: [u8; 4],
) {
    let half = thickness / 2.0;
    let (x0, y0) = p0;
    let (x1, y1) = p1;
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len2 = (dx * dx + dy * dy).max(1e-6);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = (((px - x0) * dx + (py - y0) * dy) / len2).clamp(0.0, 1.0);
            let (qx, qy) = (x0 + t * dx, y0 + t * dy);
            let dist = ((px - qx).powi(2) + (py - qy).powi(2)).sqrt();
            let cov = (half - dist + 0.5).clamp(0.0, 1.0);
            blend_px(rgba, w, h, x, y, color, cov);
        }
    }
}

/// Ícono **cerrar** (una «X») dibujado a mano — sin fuente, así nunca sale como
/// tofu (el glyph ✕ faltaba en varias fuentes del sistema). ARGB8888
/// premultiplicado, igual formato que [`TextRenderer::rasterize`].
pub fn icon_close(px: f32, color: [u8; 4]) -> Rasterized {
    let s = px.max(6.0) as i32;
    let mut rgba = vec![0u8; (s * s * 4) as usize];
    let pad = (s as f32 * 0.30).max(2.0);
    let (lo, hi) = (pad, s as f32 - pad);
    let th = (s as f32 * 0.11).clamp(1.4, 2.4);
    draw_line_aa(&mut rgba, s, s, (lo, lo), (hi, hi), th, color);
    draw_line_aa(&mut rgba, s, s, (hi, lo), (lo, hi), th, color);
    Rasterized { rgba, width: s, height: s }
}

/// Ícono **maximizar** (un cuadrado) dibujado a mano — reemplaza el glyph □ que
/// salía como tofu.
pub fn icon_square(px: f32, color: [u8; 4]) -> Rasterized {
    let s = px.max(6.0) as i32;
    let mut rgba = vec![0u8; (s * s * 4) as usize];
    let pad = (s as f32 * 0.28).max(2.0);
    let (lo, hi) = (pad, s as f32 - pad);
    let th = (s as f32 * 0.10).clamp(1.4, 2.2);
    draw_line_aa(&mut rgba, s, s, (lo, lo), (hi, lo), th, color); // arriba
    draw_line_aa(&mut rgba, s, s, (lo, hi), (hi, hi), th, color); // abajo
    draw_line_aa(&mut rgba, s, s, (lo, lo), (lo, hi), th, color); // izquierda
    draw_line_aa(&mut rgba, s, s, (hi, lo), (hi, hi), th, color); // derecha
    Rasterized { rgba, width: s, height: s }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Carga una fuente del sistema o salta el test si el entorno no tiene
    /// ninguna (no queremos fragilizar el smoke del workspace por las fuentes).
    fn font_or_skip() -> Option<TextRenderer> {
        let r = TextRenderer::system(None);
        if r.is_none() {
            eprintln!("text: sin fuentes del sistema; salto el test.");
        }
        r
    }

    #[test]
    fn rasterizes_text_to_a_nonempty_opaque_buffer() {
        let Some(tr) = font_or_skip() else { return };
        let r = tr.rasterize("Hi", 16.0, [255, 255, 255, 255]).unwrap();
        assert!(r.width > 0 && r.height > 0);
        assert_eq!(r.rgba.len(), (r.width * r.height * 4) as usize);
        // Algún píxel tiene cobertura (canal alfa > 0).
        assert!(r.rgba.chunks_exact(4).any(|p| p[3] > 0), "ningún glyph se dibujó");
    }

    #[test]
    fn empty_text_rasterizes_to_nothing() {
        let Some(tr) = font_or_skip() else { return };
        assert!(tr.rasterize("", 16.0, [255, 255, 255, 255]).is_none());
        // El espacio no tiene contorno visible.
        assert!(tr.rasterize("   ", 16.0, [255, 255, 255, 255]).is_none());
    }

    #[test]
    fn color_lands_in_bgra_order_premultiplied() {
        let Some(tr) = font_or_skip() else { return };
        // Rojo puro opaco: el píxel más cubierto debe tener R alto, G/B bajos.
        let r = tr.rasterize("M", 32.0, [255, 0, 0, 255]).unwrap();
        let reddest = r
            .rgba
            .chunks_exact(4)
            .max_by_key(|p| p[3])
            .expect("hay píxeles");
        // ARGB8888 LE = [B, G, R, A].
        assert!(reddest[2] > reddest[0], "R debería superar a B en texto rojo");
        assert!(reddest[2] > reddest[1], "R debería superar a G en texto rojo");
    }
}
