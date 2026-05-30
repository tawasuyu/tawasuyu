//! Conversión de los pixel-formats que escupe una cámara a RGBA8.
//!
//! Es el corazón puro del crate: no toca hardware, no aloca de más
//! (escribe sobre un `dst` reusable) y se testea sin un solo `/dev/video`.
//! Los backends (v4l2, y mañana captura de pantalla) sólo entregan
//! `(PixelFormat, width, height, bytes)` y dejan que esto lo normalice.

/// Formato del buffer crudo que entrega el dispositivo. Cubre lo que
/// una webcam v4l2 típica ofrece más los dos RGB triviales.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// YUV 4:2:2 empacado, 4 bytes = 2 píxeles: `[Y0 U Y1 V]`. El
    /// formato sin comprimir más común en webcams.
    Yuyv,
    /// JPEG por frame (Motion-JPEG). Lo decodifica el crate `image`.
    Mjpeg,
    /// 3 bytes por píxel, orden R,G,B.
    Rgb24,
    /// 3 bytes por píxel, orden B,G,R.
    Bgr24,
    /// 4 bytes por píxel, orden B,G,R,X — el 4º byte (padding/alfa) se
    /// ignora y la salida queda opaca. Es lo que entrega un servidor X11
    /// little-endian (`ImageOrder::LSBFirst`) por `GetImage`: el pixel
    /// `0x00RRGGBB` cae en memoria como `[BB GG RR 00]`. Usado por el
    /// backend de captura de pantalla.
    Bgrx32,
    /// 4 bytes por píxel, orden X,R,G,B — contraparte big-endian del
    /// anterior (`ImageOrder::MSBFirst`). El 4º (último) canal real es B.
    Xrgb32,
}

impl PixelFormat {
    /// Mapea el FourCC v4l2 (4 bytes ASCII) a nuestro enum. `None` si
    /// no sabemos convertirlo — el backend reporta el error con el
    /// código legible.
    pub fn from_fourcc(cc: [u8; 4]) -> Option<Self> {
        match &cc {
            b"YUYV" => Some(Self::Yuyv),
            b"MJPG" => Some(Self::Mjpeg),
            b"RGB3" => Some(Self::Rgb24),
            b"BGR3" => Some(Self::Bgr24),
            _ => None,
        }
    }
}

/// Convierte `src` (en `fmt`, `width`×`height`) a RGBA8 dentro de
/// `dst`, redimensionándolo si hace falta. Devuelve `true` si la
/// conversión fue válida; `false` si el buffer no cuadra con las
/// dimensiones (frame corrupto/truncado) — en ese caso `dst` queda
/// intacto y el caller descarta el frame.
pub fn to_rgba(
    fmt: PixelFormat,
    width: u32,
    height: u32,
    src: &[u8],
    dst: &mut Vec<u8>,
) -> bool {
    let (w, h) = (width as usize, height as usize);
    let pixels = w.checked_mul(h);
    let Some(pixels) = pixels else { return false };
    let rgba_len = pixels * 4;

    match fmt {
        PixelFormat::Yuyv => {
            // 2 píxeles por cada 4 bytes.
            if src.len() < pixels * 2 {
                return false;
            }
            dst.resize(rgba_len, 0);
            yuyv_to_rgba(src, w, h, dst);
            true
        }
        PixelFormat::Rgb24 | PixelFormat::Bgr24 => {
            if src.len() < pixels * 3 {
                return false;
            }
            dst.resize(rgba_len, 0);
            let swap = matches!(fmt, PixelFormat::Bgr24);
            for (px, out) in src.chunks_exact(3).take(pixels).zip(dst.chunks_exact_mut(4)) {
                let (r, b) = if swap { (px[2], px[0]) } else { (px[0], px[2]) };
                out[0] = r;
                out[1] = px[1];
                out[2] = b;
                out[3] = 255;
            }
            true
        }
        PixelFormat::Bgrx32 | PixelFormat::Xrgb32 => {
            if src.len() < pixels * 4 {
                return false;
            }
            dst.resize(rgba_len, 0);
            let xrgb = matches!(fmt, PixelFormat::Xrgb32);
            for (px, out) in src.chunks_exact(4).take(pixels).zip(dst.chunks_exact_mut(4)) {
                if xrgb {
                    // [X, R, G, B]
                    out[0] = px[1];
                    out[1] = px[2];
                    out[2] = px[3];
                } else {
                    // [B, G, R, X]
                    out[0] = px[2];
                    out[1] = px[1];
                    out[2] = px[0];
                }
                out[3] = 255;
            }
            true
        }
        PixelFormat::Mjpeg => mjpeg_to_rgba(src, w, h, dst),
    }
}

/// YUYV (YUV 4:2:2 empacado) → RGBA, BT.601 limited range — la
/// convención de las webcams v4l2 (luma 16..235, croma 16..240). Dos
/// píxeles comparten el par croma `U/V`.
fn yuyv_to_rgba(src: &[u8], w: usize, h: usize, dst: &mut [u8]) {
    // Recorre de a 4 bytes (2 px) y escribe de a 8 bytes (2 px RGBA).
    let row_bytes = w * 2;
    for y in 0..h {
        let row = &src[y * row_bytes..y * row_bytes + row_bytes];
        let out_row = &mut dst[y * w * 4..y * w * 4 + w * 4];
        for (i, quad) in row.chunks_exact(4).enumerate() {
            let y0 = quad[0] as f32;
            let u = quad[1] as f32;
            let y1 = quad[2] as f32;
            let v = quad[3] as f32;
            let (r0, g0, b0) = ycbcr_to_rgb(y0, u, v);
            let (r1, g1, b1) = ycbcr_to_rgb(y1, u, v);
            let o = i * 8;
            out_row[o] = r0;
            out_row[o + 1] = g0;
            out_row[o + 2] = b0;
            out_row[o + 3] = 255;
            out_row[o + 4] = r1;
            out_row[o + 5] = g1;
            out_row[o + 6] = b1;
            out_row[o + 7] = 255;
        }
    }
}

/// BT.601 limited-range YUV → RGB. Coeficientes estándar de v4l2.
#[inline]
fn ycbcr_to_rgb(y: f32, u: f32, v: f32) -> (u8, u8, u8) {
    let c = y - 16.0;
    let d = u - 128.0;
    let e = v - 128.0;
    let r = 1.164 * c + 1.596 * e;
    let g = 1.164 * c - 0.391 * d - 0.813 * e;
    let b = 1.164 * c + 2.018 * d;
    (clamp_u8(r), clamp_u8(g), clamp_u8(b))
}

#[inline]
fn clamp_u8(x: f32) -> u8 {
    x.max(0.0).min(255.0) as u8
}

/// Decodifica un frame Motion-JPEG a RGBA con el crate `image`.
/// Verifica que las dimensiones del JPEG coincidan con las anunciadas
/// por el dispositivo (un MJPEG truncado decodea a otro tamaño).
fn mjpeg_to_rgba(src: &[u8], w: usize, h: usize, dst: &mut Vec<u8>) -> bool {
    let Ok(img) =
        image::load_from_memory_with_format(src, image::ImageFormat::Jpeg)
    else {
        return false;
    };
    let rgba = img.to_rgba8();
    if rgba.width() as usize != w || rgba.height() as usize != h {
        return false;
    }
    let raw = rgba.into_raw();
    dst.clear();
    dst.extend_from_slice(&raw);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_mapping() {
        assert_eq!(PixelFormat::from_fourcc(*b"YUYV"), Some(PixelFormat::Yuyv));
        assert_eq!(PixelFormat::from_fourcc(*b"MJPG"), Some(PixelFormat::Mjpeg));
        assert_eq!(PixelFormat::from_fourcc(*b"H264"), None);
    }

    #[test]
    fn rgb24_passthrough_y_alfa() {
        let src = [10u8, 20, 30, 40, 50, 60]; // 2 px
        let mut dst = Vec::new();
        assert!(to_rgba(PixelFormat::Rgb24, 2, 1, &src, &mut dst));
        assert_eq!(dst, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn bgr24_swap() {
        let src = [30u8, 20, 10]; // B,G,R → R,G,B
        let mut dst = Vec::new();
        assert!(to_rgba(PixelFormat::Bgr24, 1, 1, &src, &mut dst));
        assert_eq!(dst, vec![10, 20, 30, 255]);
    }

    #[test]
    fn yuyv_gris_neutro() {
        // Y=126 (mid), U=V=128 (sin croma) → gris ~ (126-16)*1.164 ≈ 128.
        let src = [126u8, 128, 126, 128];
        let mut dst = Vec::new();
        assert!(to_rgba(PixelFormat::Yuyv, 2, 1, &src, &mut dst));
        // dos píxeles iguales, casi-grises (R=G=B), alfa opaco.
        for px in dst.chunks_exact(4) {
            assert_eq!(px[3], 255);
            assert!((px[0] as i32 - px[1] as i32).abs() <= 2);
            assert!((px[1] as i32 - px[2] as i32).abs() <= 2);
            assert!((px[0] as i32 - 128).abs() <= 4);
        }
    }

    #[test]
    fn yuyv_rojo_saturado() {
        // V alto → empuja rojo. Y=128, U=128, V=255.
        let src = [128u8, 128, 128, 255];
        let mut dst = Vec::new();
        assert!(to_rgba(PixelFormat::Yuyv, 2, 1, &src, &mut dst));
        // R debe dominar sobre B en ambos píxeles.
        assert!(dst[0] > dst[2]);
        assert!(dst[4] > dst[6]);
    }

    #[test]
    fn bgrx32_swap_e_ignora_padding() {
        // [B,G,R,X] → [R,G,B,255]; el padding 0x77 se descarta.
        let src = [30u8, 20, 10, 0x77];
        let mut dst = Vec::new();
        assert!(to_rgba(PixelFormat::Bgrx32, 1, 1, &src, &mut dst));
        assert_eq!(dst, vec![10, 20, 30, 255]);
    }

    #[test]
    fn xrgb32_big_endian() {
        // [X,R,G,B] → [R,G,B,255]; el padding 0x77 (primero) se descarta.
        let src = [0x77u8, 10, 20, 30];
        let mut dst = Vec::new();
        assert!(to_rgba(PixelFormat::Xrgb32, 1, 1, &src, &mut dst));
        assert_eq!(dst, vec![10, 20, 30, 255]);
    }

    #[test]
    fn bgrx32_truncado_se_rechaza() {
        let src = [0u8; 4]; // alcanza para 1 px, no para 2
        let mut dst = vec![0xAB; 8];
        assert!(!to_rgba(PixelFormat::Bgrx32, 2, 1, &src, &mut dst));
        assert_eq!(dst, vec![0xAB; 8]);
    }

    #[test]
    fn buffer_truncado_se_rechaza() {
        let src = [0u8; 3]; // alcanza para 1 px RGB, no para 2 px
        let mut dst = vec![0xAB; 8];
        assert!(!to_rgba(PixelFormat::Rgb24, 2, 1, &src, &mut dst));
        // dst intacto.
        assert_eq!(dst, vec![0xAB; 8]);
    }
}
