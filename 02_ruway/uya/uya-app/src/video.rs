// =============================================================================
//  uya-app::video — códec de cuadros (JPEG por cuadro / MJPEG).
// -----------------------------------------------------------------------------
//  Comprime cada cuadro RGBA a JPEG para el cable (~20-40× menos bytes que el
//  RGBA crudo) y lo decodifica de vuelta a RGBA en recepción. Sin estado entre
//  cuadros: baja latencia, robusto a pérdidas (cada cuadro es independiente) —
//  exactamente lo que quiere una videollamada. AV1 (rav1e) daría más
//  compresión pero es demasiado lento para tiempo real.
// =============================================================================

/// Calidad JPEG por defecto (0..=100). 70 es un buen compromiso para video:
/// nítido a simple vista, ~1/30 del tamaño del RGBA.
pub(crate) const CALIDAD: u8 = 70;

/// Comprime un cuadro RGBA8 a JPEG. JPEG no lleva alfa, así que primero
/// descartamos el canal A (RGBA → RGB). `None` si el encode falla.
pub(crate) fn encodar_jpeg(rgba: &[u8], ancho: u32, alto: u32, calidad: u8) -> Option<Vec<u8>> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&px[..3]);
    }
    let mut salida = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut salida, calidad);
    enc.encode(&rgb, ancho, alto, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(salida)
}

/// Decodifica un JPEG a RGBA8. Devuelve `(ancho, alto, rgba)` o `None` si el
/// JPEG está corrupto.
pub(crate) fn decodar_jpeg(datos: &[u8]) -> Option<(u16, u16, Vec<u8>)> {
    let img = image::load_from_memory_with_format(datos, image::ImageFormat::Jpeg).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some((w as u16, h as u16, rgba.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_jpeg_preserva_dimensiones() {
        // Un cuadro 8×8 RGBA gris (la compresión JPEG no es lossless, así que
        // sólo verificamos dimensiones y que el roundtrip no rompe).
        let (w, h) = (8u32, 8u32);
        let rgba = vec![128u8; (w * h * 4) as usize];
        let jpeg = encodar_jpeg(&rgba, w, h, CALIDAD).expect("encode");
        assert!(jpeg.len() >= 2 && &jpeg[..2] == b"\xff\xd8", "magic JPEG");
        let (dw, dh, salida) = decodar_jpeg(&jpeg).expect("decode");
        assert_eq!((dw as u32, dh as u32), (w, h));
        assert_eq!(salida.len(), rgba.len());
    }
}
