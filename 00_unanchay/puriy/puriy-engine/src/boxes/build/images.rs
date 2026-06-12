//! Prefetch/decodificación de imágenes (URLs de `<img>`/backgrounds, fetch+decode).
//! Extraído de `boxes/build.rs` (regla #1). Sin cambios de lógica.
use super::*;

/// Workers paralelos para el prefetch. 6 es un compromiso razonable:
/// alto enough para esconder latencia de TCP/TLS (cada handshake ~50-
/// 200ms), bajo enough para no saturar servidores ni el ulimit de
/// sockets del proceso. Browsers reales usan 6-8 por host.
const PREFETCH_WORKERS: usize = 6;

/// Pre-walk del DOM coleccionando URLs absolutas de `<img src>`,
/// `<img srcset>`, `<picture><source srcset>`, y disparando descargas
/// paralelas. La cache global de bytes guarda los resultados —
/// `fetch_and_decode` en `build_node` después hace cache hit.
pub(crate) fn prefetch_image_urls(root: &Handle, base: Option<&url::Url>) {
    let mut urls: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut push = |u: String| {
        if seen.insert(u.clone()) {
            urls.push(u);
        }
    };
    dom::walk(root, &mut |node| {
        let tag = dom::element_name(node);
        match tag.as_deref() {
            Some("img") => {
                if let Some(src) = pick_srcset(&dom::attr(node, "srcset").unwrap_or_default())
                    .or_else(|| dom::attr(node, "src"))
                {
                    if let Some(abs) = resolve_href(base, &src) {
                        push(abs);
                    }
                }
            }
            Some("picture") => {
                for child in node.children.borrow().iter() {
                    if dom::element_name(child).as_deref() == Some("source") {
                        if let Some(s) = dom::attr(child, "srcset") {
                            if let Some(c) = pick_srcset(&s) {
                                if let Some(abs) = resolve_href(base, &c) {
                                    push(abs);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    });
    if urls.is_empty() {
        return;
    }
    // Cache hits no necesitan fetch; los filtramos para ahorrar threads.
    // Además filtramos schemes no-HTTP (`about:`, `file:`, `data:`) —
    // ureq haría un round-trip al timeout para nada.
    let pending: Vec<String> = urls
        .into_iter()
        .filter(|u| {
            url::Url::parse(u)
                .ok()
                .map(|p| matches!(p.scheme(), "http" | "https"))
                .unwrap_or(false)
        })
        .filter(|u| crate::cache::get(u).is_none())
        .collect();
    if pending.is_empty() {
        return;
    }
    // Pool simple: dividir las URLs en chunks de tamaño ceil(N/W) y un
    // thread por chunk. Más simple que un channel + N workers, y para
    // 6-30 URLs típicas de una página el balance es suficiente.
    let chunk_size = pending.len().div_ceil(PREFETCH_WORKERS).max(1);
    let mut handles = Vec::new();
    for chunk in pending.chunks(chunk_size) {
        let chunk = chunk.to_vec();
        handles.push(std::thread::spawn(move || {
            for url in chunk {
                // Best-effort: errores se ignoran. El build_node
                // posterior los reintentará serializado y muestra el
                // alt del `<img>` si igual falla.
                let _ = crate::fetch::fetch_bytes(&url);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
}

/// Segundo pass de prefetch: recolecta URLs de `background-image:
/// url(...)` después de computar styles. Reusa el mismo pool de
/// workers que `prefetch_image_urls`. Computamos sin parent porque
/// `background-image` no se hereda y los valores son independientes
/// del contexto del padre (cosa que sí valdría para `color` o
/// `font-size`).
pub(crate) fn prefetch_background_image_urls(
    root: &Handle,
    styles: &StyleEngine,
    base: Option<&url::Url>,
) {
    let mut urls: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    dom::walk(root, &mut |node| {
        if !matches!(node.data, markup5ever_rcdom::NodeData::Element { .. }) {
            return;
        }
        let style = styles.compute(node);
        let mut push = |u: &str| {
            if let Some(abs) = resolve_href(base, u) {
                if seen.insert(abs.clone()) {
                    urls.push(abs);
                }
            }
        };
        if let Some(u) = style.background_image_url.as_deref() {
            push(u);
        }
        // Capas extra (lista `background: a, b`): prefetch sus url() también.
        for l in &style.background_extra_layers {
            if let crate::style::BackgroundImage::Url(u) = &l.image {
                push(u);
            }
        }
    });
    if urls.is_empty() {
        return;
    }
    let pending: Vec<String> = urls
        .into_iter()
        .filter(|u| {
            url::Url::parse(u)
                .ok()
                .map(|p| matches!(p.scheme(), "http" | "https"))
                .unwrap_or(false)
        })
        .filter(|u| crate::cache::get(u).is_none())
        .collect();
    if pending.is_empty() {
        return;
    }
    let chunk_size = pending.len().div_ceil(PREFETCH_WORKERS).max(1);
    let mut handles = Vec::new();
    for chunk in pending.chunks(chunk_size) {
        let chunk = chunk.to_vec();
        handles.push(std::thread::spawn(move || {
            for url in chunk {
                let _ = crate::fetch::fetch_bytes(&url);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
}

pub(crate) fn fetch_and_decode(url: &str) -> Option<ImageData> {
    let bytes = crate::fetch::fetch_bytes(url).ok()?;
    decode_image_bytes(&bytes)
}

/// Decodifica bytes de imagen (PNG/JPEG por las features de `image`) a RGBA8.
/// `None` si el formato no está habilitado o el decode falla.
pub(crate) fn decode_image_bytes(bytes: &[u8]) -> Option<ImageData> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.format()?; // formato no habilitado por features → None
    let img = reader.decode().ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Some(ImageData { rgba: rgba.into_raw(), width, height })
}

/// Resuelve+decodifica la imagen de un `src`/`srcset`/`background-image`.
/// Los `data:` URLs se decodifican inline (RFC 2397) — `resolve_href` los
/// bloquea a propósito (no son navegables como `<a href>`), pero como fuente
/// de un recurso son legítimos. El resto resuelve contra `base` y baja por
/// HTTP/file. `None` si falta src o falla la decodificación.
pub fn fetch_image_src(base: Option<&url::Url>, src: &str) -> Option<ImageData> {
    if crate::fetch::is_data_url(src.trim()) {
        return decode_image_bytes(&crate::fetch::decode_data_url(src.trim())?);
    }
    let abs = resolve_href(base, src)?;
    fetch_and_decode(&abs)
}

