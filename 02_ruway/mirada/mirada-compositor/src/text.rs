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

/// Ícono **minimizar** (una raya horizontal baja) dibujado a mano. Manda la
/// ventana al scratchpad (≈ minimizar/ocultar).
pub fn icon_minimize(px: f32, color: [u8; 4]) -> Rasterized {
    let s = px.max(6.0) as i32;
    let mut rgba = vec![0u8; (s * s * 4) as usize];
    let pad = (s as f32 * 0.28).max(2.0);
    let (lo, hi) = (pad, s as f32 - pad);
    let th = (s as f32 * 0.11).clamp(1.4, 2.4);
    let yb = s as f32 - pad - th * 0.5; // raya baja, estilo macOS/GTK
    draw_line_aa(&mut rgba, s, s, (lo, yb), (hi, yb), th, color);
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

/// Ícono **pantalla completa**: cuatro corchetes en las esquinas (apuntando
/// hacia afuera). Dibujado a mano, mismo formato que los demás.
pub fn icon_fullscreen(px: f32, color: [u8; 4]) -> Rasterized {
    let s = px.max(6.0) as i32;
    let mut rgba = vec![0u8; (s * s * 4) as usize];
    let pad = (s as f32 * 0.26).max(2.0);
    let (lo, hi) = (pad, s as f32 - pad);
    let th = (s as f32 * 0.11).clamp(1.4, 2.4);
    let arm = (hi - lo) * 0.4; // largo de cada brazo del corchete
    // Esquina sup-izq, sup-der, inf-izq, inf-der (dos brazos cada una).
    draw_line_aa(&mut rgba, s, s, (lo, lo), (lo + arm, lo), th, color);
    draw_line_aa(&mut rgba, s, s, (lo, lo), (lo, lo + arm), th, color);
    draw_line_aa(&mut rgba, s, s, (hi, lo), (hi - arm, lo), th, color);
    draw_line_aa(&mut rgba, s, s, (hi, lo), (hi, lo + arm), th, color);
    draw_line_aa(&mut rgba, s, s, (lo, hi), (lo + arm, hi), th, color);
    draw_line_aa(&mut rgba, s, s, (lo, hi), (lo, hi - arm), th, color);
    draw_line_aa(&mut rgba, s, s, (hi, hi), (hi - arm, hi), th, color);
    draw_line_aa(&mut rgba, s, s, (hi, hi), (hi, hi - arm), th, color);
    Rasterized { rgba, width: s, height: s }
}

/// Ícono **menú** (hamburguesa: tres rayas horizontales). Abre el menú
/// contextual de la ventana.
pub fn icon_menu(px: f32, color: [u8; 4]) -> Rasterized {
    let s = px.max(6.0) as i32;
    let mut rgba = vec![0u8; (s * s * 4) as usize];
    let pad = (s as f32 * 0.26).max(2.0);
    let (lo, hi) = (pad, s as f32 - pad);
    let th = (s as f32 * 0.10).clamp(1.3, 2.2);
    for k in 0..3 {
        let y = lo + (hi - lo) * (k as f32) / 2.0;
        draw_line_aa(&mut rgba, s, s, (lo, y), (hi, y), th, color);
    }
    Rasterized { rgba, width: s, height: s }
}

/// Ícono **flotar/teselar**: dos cuadraditos superpuestos (cascada).
pub fn icon_float(px: f32, color: [u8; 4]) -> Rasterized {
    let s = px.max(6.0) as i32;
    let mut rgba = vec![0u8; (s * s * 4) as usize];
    let pad = (s as f32 * 0.26).max(2.0);
    let th = (s as f32 * 0.10).clamp(1.3, 2.0);
    let side = (s as f32 - 2.0 * pad) * 0.66;
    let off = side * 0.34;
    // Cuadrado de atrás (arriba-derecha) y de adelante (abajo-izquierda).
    let sq = |rgba: &mut [u8], x: f32, y: f32| {
        draw_line_aa(rgba, s, s, (x, y), (x + side, y), th, color);
        draw_line_aa(rgba, s, s, (x, y + side), (x + side, y + side), th, color);
        draw_line_aa(rgba, s, s, (x, y), (x, y + side), th, color);
        draw_line_aa(rgba, s, s, (x + side, y), (x + side, y + side), th, color);
    };
    sq(&mut rgba, pad + off, pad);
    sq(&mut rgba, pad, pad + off);
    Rasterized { rgba, width: s, height: s }
}

/// **Disco** lleno antialiased (un círculo) del color dado — el botón estilo
/// macOS («traffic light»). Mismo formato que los demás íconos.
pub fn icon_disc(px: f32, color: [u8; 4]) -> Rasterized {
    let s = px.max(6.0) as i32;
    let mut rgba = vec![0u8; (s * s * 4) as usize];
    let c = s as f32 / 2.0;
    let r = c - 0.5;
    for y in 0..s {
        for x in 0..s {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            let cov = (r - d + 0.5).clamp(0.0, 1.0); // ~1px de antialias en el borde
            if cov > 0.0 {
                blend_px(&mut rgba, s, s, x, y, color, cov);
            }
        }
    }
    Rasterized { rgba, width: s, height: s }
}

/// Rellena un rect opaco/translúcido en el búfer (color en orden **R,G,B,A**,
/// igual que [`blend_px`]). Recorta a los límites.
fn fill_rect(rgba: &mut [u8], w: i32, h: i32, x: i32, y: i32, rw: i32, rh: i32, color: [u8; 4]) {
    for yy in y.max(0)..(y + rh).min(h) {
        for xx in x.max(0)..(x + rw).min(w) {
            blend_px(rgba, w, h, xx, yy, color, 1.0);
        }
    }
}

/// Compone un búfer premultiplicado (`src`, bytes B,G,R,A) sobre `dst` con
/// source-over, desplazado a `(ox, oy)`. Para incrustar el número en el tile.
fn blit_premul(dst: &mut [u8], dw: i32, dh: i32, src: &[u8], sw: i32, sh: i32, ox: i32, oy: i32) {
    for sy in 0..sh {
        for sx in 0..sw {
            let (x, y) = (ox + sx, oy + sy);
            if x < 0 || y < 0 || x >= dw || y >= dh {
                continue;
            }
            let si = ((sy * sw + sx) * 4) as usize;
            let a = src[si + 3] as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            let inv = 1.0 - a;
            let di = ((y * dw + x) * 4) as usize;
            for k in 0..4 {
                dst[di + k] = (src[si + k] as f32 + dst[di + k] as f32 * inv) as u8;
            }
        }
    }
}

/// Muestreo bilineal de un búfer premultiplicado (B,G,R,A) en `(fx, fy)` (centros
/// de píxel). Fuera de rango = transparente. Devuelve `[B,G,R,A]` en `f32`.
fn sample_bilinear(src: &[u8], sw: i32, sh: i32, fx: f32, fy: f32) -> [f32; 4] {
    let (px, py) = (fx - 0.5, fy - 0.5);
    let (x0, y0) = (px.floor() as i32, py.floor() as i32);
    let (tx, ty) = (px - x0 as f32, py - y0 as f32);
    let mut out = [0.0f32; 4];
    for (dyi, wy) in [(0, 1.0 - ty), (1, ty)] {
        for (dxi, wx) in [(0, 1.0 - tx), (1, tx)] {
            let w = wx * wy;
            if w <= 0.0 {
                continue;
            }
            let (sx, sy) = (x0 + dxi, y0 + dyi);
            if sx < 0 || sy < 0 || sx >= sw || sy >= sh {
                continue; // borde → transparente
            }
            let i = ((sy * sw + sx) * 4) as usize;
            for k in 0..4 {
                out[k] += src[i + k] as f32 * w;
            }
        }
    }
    out
}

/// Rota un búfer ARGB8888 premultiplicado `rot` rad alrededor de su centro,
/// emitiendo el búfer de su AABB (esquinas transparentes). Muestreo inverso
/// bilineal — premultiplicado interpola sin halos.
pub fn rotate_buffer(src: &[u8], sw: i32, sh: i32, rot: f32) -> Rasterized {
    let (s, c) = rot.sin_cos();
    let aw = ((sw as f32 * c.abs()) + (sh as f32 * s.abs())).ceil().max(1.0) as i32;
    let ah = ((sw as f32 * s.abs()) + (sh as f32 * c.abs())).ceil().max(1.0) as i32;
    let mut dst = vec![0u8; (aw * ah * 4) as usize];
    let (scx, scy) = (sw as f32 / 2.0, sh as f32 / 2.0);
    let (dcx, dcy) = (aw as f32 / 2.0, ah as f32 / 2.0);
    // Rotación inversa: local = centro_src + R(-rot)·(p − centro_dst).
    let (si, ci) = (-rot).sin_cos();
    for dy in 0..ah {
        for dx in 0..aw {
            let ux = dx as f32 + 0.5 - dcx;
            let uy = dy as f32 + 0.5 - dcy;
            let lx = ux * ci - uy * si + scx;
            let ly = ux * si + uy * ci + scy;
            let px = sample_bilinear(src, sw, sh, lx, ly);
            let i = ((dy * aw + dx) * 4) as usize;
            for k in 0..4 {
                dst[i + k] = px[k].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Rasterized { rgba: dst, width: aw, height: ah }
}

/// Compone un tile del overview **rotado** `rot` rad a un búfer ARGB8888 del
/// tamaño de su AABB: pinta en espacio local (fondo opaco + ventanas a escala +
/// borde activo opcional + número) y rota por muestreo inverso. Es la forma de
/// ROTAR en el overlay GLES, donde los `SolidColorRenderElement` son
/// axis-aligned. El llamante coloca el búfer centrado en el centro del tile.
#[allow(clippy::too_many_arguments)]
pub fn rasterize_tile_rotated(
    tw: i32,
    th: i32,
    rot: f32,
    tile_bg: [u8; 4],
    border: Option<[u8; 4]>,
    wins: &[(i32, i32, i32, i32, bool)],
    win_bg: [u8; 4],
    win_focus: [u8; 4],
    badge: Option<&Rasterized>,
) -> Rasterized {
    let (tw, th) = (tw.max(1), th.max(1));
    let mut local = vec![0u8; (tw * th * 4) as usize];
    fill_rect(&mut local, tw, th, 0, 0, tw, th, tile_bg);
    for &(wx, wy, ww, wh, focus) in wins {
        fill_rect(&mut local, tw, th, wx, wy, ww, wh, if focus { win_focus } else { win_bg });
    }
    if let Some(bc) = border {
        let t = 3;
        fill_rect(&mut local, tw, th, 0, 0, tw, t, bc); // arriba
        fill_rect(&mut local, tw, th, 0, th - t, tw, t, bc); // abajo
        fill_rect(&mut local, tw, th, 0, 0, t, th, bc); // izquierda
        fill_rect(&mut local, tw, th, tw - t, 0, t, th, bc); // derecha
    }
    if let Some(b) = badge {
        blit_premul(&mut local, tw, th, &b.rgba, b.width, b.height, 8, 6);
    }
    rotate_buffer(&local, tw, th, rot)
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

#[cfg(test)]
mod rot_tests {
    use super::*;

    /// Helper: alpha del pixel `(x,y)` en un búfer ARGB8888 (byte A).
    fn alpha(r: &Rasterized, x: i32, y: i32) -> u8 {
        r.rgba[((y * r.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn rotar_cero_preserva_dimensiones_y_opacidad() {
        let r = rasterize_tile_rotated(
            120, 80, 0.0, [40, 50, 70, 255], None, &[], [0, 0, 0, 0], [0, 0, 0, 0], None,
        );
        assert_eq!((r.width, r.height), (120, 80), "rot 0 → AABB == tile");
        // Centro opaco con el fondo del tile (premult B,G,R,A → B=70).
        let i = ((40 * r.width + 60) * 4) as usize;
        assert_eq!(r.rgba[i + 3], 255, "centro opaco");
        assert!((r.rgba[i] as i32 - 70).abs() <= 2, "B≈70 (azul del fondo)");
    }

    #[test]
    fn rotar_90_grados_intercambia_dimensiones() {
        let r = rasterize_tile_rotated(
            120, 80, std::f32::consts::FRAC_PI_2, [40, 50, 70, 255], None, &[], [0, 0, 0, 0],
            [0, 0, 0, 0], None,
        );
        // 90° → el AABB es 80×120 (±1 por redondeo del ceil).
        assert!((r.width - 80).abs() <= 1 && (r.height - 120).abs() <= 1, "{}x{}", r.width, r.height);
    }

    #[test]
    fn tile_girado_opaco_dentro_y_transparente_en_las_esquinas() {
        let r = rasterize_tile_rotated(
            100, 100, 0.5, [40, 50, 70, 255], Some([50, 120, 240, 255]),
            &[(10, 10, 40, 30, true)], [60, 70, 90, 255], [55, 110, 210, 255], None,
        );
        // El centro del AABB es el centro del tile → opaco.
        assert_eq!(alpha(&r, r.width / 2, r.height / 2), 255, "centro opaco");
        // Las esquinas del AABB caen fuera del rect girado → transparentes.
        assert_eq!(alpha(&r, 0, 0), 0, "esquina sup-izq transparente");
        assert_eq!(alpha(&r, r.width - 1, 0), 0, "esquina sup-der transparente");
    }

    /// Vuelca un tile girado a PNG (sólo con `MIRADA_DUMP_TILE=<ruta>`), para VER
    /// el búfer exacto que el compositor sube como textura — sin GLES.
    #[test]
    fn dump_tile_rotado_a_png() {
        let Ok(path) = std::env::var("MIRADA_DUMP_TILE") else {
            return;
        };
        // Un tile 240×150 con 3 "ventanas" y borde activo, girado ~22°.
        let r = rasterize_tile_rotated(
            240,
            150,
            0.38,
            [31, 33, 43, 255], // TILE_BG
            Some([51, 128, 242, 255]), // ACTIVE_BORDER
            &[(10, 10, 105, 130, false), (125, 10, 105, 60, true), (125, 80, 105, 60, false)],
            [66, 77, 102, 255], // WIN_BG
            [56, 115, 217, 255], // WIN_FOCUS
            None,
        );
        // premult [B,G,R,A] → straight [R,G,B,A] para verlo.
        let mut out = vec![0u8; r.rgba.len()];
        for (o, px) in out.chunks_mut(4).zip(r.rgba.chunks(4)) {
            let a = px[3];
            let unp = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255) / a as u32).min(255) as u8 };
            o[0] = unp(px[2]);
            o[1] = unp(px[1]);
            o[2] = unp(px[0]);
            o[3] = a;
        }
        image::save_buffer(&path, &out, r.width as u32, r.height as u32, image::ColorType::Rgba8)
            .expect("png");
        eprintln!("dump_tile_rotado: {path} ({}x{})", r.width, r.height);
    }
}
