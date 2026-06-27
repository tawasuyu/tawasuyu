//! Miniaturas de las sesiones para el lock (lado del greeter).
//!
//! El compositor captura la preview de cada sesión FUS, la deja en un archivo
//! crudo del runtime dir y nos pasa las rutas por stdin (`THUMBS id=ruta …`).
//! Acá las parseamos y cargamos a un `peniko::Image` para que el lock pinte una
//! tarjeta por sesión —la activa («la última») resaltada—. El formato es el que
//! escribe `mirada-compositor/src/thumbs.rs`: `MTH1` + w(u32 LE) + h(u32 LE) +
//! RGBA8. Si algo falla, devolvemos `None` y el lock cae a tarjeta genérica.

use llimphi_image::Image;

/// Parsea una línea `THUMBS <id>=<ruta> …` empujada por el compositor. Devuelve
/// los pares `(id, ruta)`. `None` si la línea no es un `THUMBS` bien formado.
pub fn parse_thumbs(line: &str) -> Option<Vec<(u32, String)>> {
    let rest = line.trim().strip_prefix("THUMBS")?;
    let mut out = Vec::new();
    for tok in rest.split_whitespace() {
        let (id, path) = tok.split_once('=')?;
        out.push((id.parse().ok()?, path.to_string()));
    }
    Some(out)
}

/// Carga una miniatura cruda (`MTH1`) a `(imagen, w, h)`. Devuelve también las
/// dimensiones —las leemos de la cabecera— para no depender de cómo expone el
/// tamaño la versión de `peniko` en uso. `None` si el archivo no está, la magia
/// no cuadra o el tamaño no cierra con los bytes.
pub fn load(path: &str) -> Option<(Image, u32, u32)> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 12 || &bytes[0..4] != b"MTH1" {
        return None;
    }
    let w = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    let h = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    let need = (w as usize) * (h as usize) * 4;
    if w == 0 || h == 0 || bytes.len() < 12 + need {
        return None;
    }
    let rgba = bytes[12..12 + need].to_vec();
    Some((llimphi_image::from_rgba8(rgba, w, h), w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_thumbs() {
        let t = parse_thumbs("THUMBS 0=/run/a.thumb 1=/run/b.thumb").unwrap();
        assert_eq!(
            t,
            vec![(0, "/run/a.thumb".to_string()), (1, "/run/b.thumb".to_string())]
        );
        // Sin tokens (nada capturado) es válido: lista vacía.
        assert!(parse_thumbs("THUMBS").unwrap().is_empty());
    }

    #[test]
    fn rechaza_basura() {
        assert!(parse_thumbs("SESSIONS 0 0:ana").is_none());
        assert!(parse_thumbs("THUMBS nocolon").is_none());
        assert!(parse_thumbs("THUMBS x=/r").is_none());
    }

    #[test]
    fn carga_rechaza_magia_mala() {
        // Sin magia válida → None (no panic).
        assert!(load("/no/existe/jamas.thumb").is_none());
    }
}
