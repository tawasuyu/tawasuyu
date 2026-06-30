//! Miniaturas de las sesiones FUS para el lock.
//!
//! Cuando se engancha el candado, el compositor rinde las ventanas de **cada**
//! sesión hosteada a un offscreen, lo achica en CPU y lo deja en un archivo del
//! runtime dir; le pasa al greeter las rutas por su stdin (`THUMBS …`). El lock
//! las pinta como tarjetas —la activa («la última») seleccionada por defecto— y
//! permite saltar a otra. Formato propio crudo (cabecera + RGBA) para no sumar
//! un encoder PNG: el greeter lo lee directo a un `peniko::Image`.
//!
//! **Privacidad:** hay quien bloquea justo para ocultar su pantalla. Por eso es
//! configurable vía `MIRADA_LOCK_PREVIEW` (lo fija el panel / wawapanel): `live`
//! (default) captura la preview; `hidden`/`off` no captura nada y el lock cae a
//! tarjetas genéricas (nombre, sin imagen).
//!
//! **Por verificar en sesión gráfica:** la captura per-sesión necesita varias
//! sesiones vivas y GPU; headless sólo se certifica el lado del greeter (que lee
//! el formato y pinta las tarjetas, con archivos sintéticos).

use std::path::PathBuf;
use std::sync::OnceLock;

use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};

use mirada_brain::SessionId;

use crate::estado::App;

/// Ancho de la miniatura (px). El alto sale de respetar el aspecto de la salida.
const THUMB_W: u32 = 320;
/// Magia del formato crudo: `MTH1` + w(u32 LE) + h(u32 LE) + RGBA8 (w*h*4).
const MAGIC: &[u8; 4] = b"MTH1";

/// Política de preview del lock, leída del entorno la primera vez. La fija el
/// panel (wawapanel) vía `MIRADA_LOCK_PREVIEW`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LockPreview {
    /// Captura la preview viva de cada sesión (default).
    Live,
    /// No captura — el lock muestra tarjetas genéricas (privacidad).
    Hidden,
}

/// La política de preview del lock (cacheada). `live` por defecto; `hidden`,
/// `off`, `0`, `no` la apagan.
pub(crate) fn lock_preview() -> LockPreview {
    static P: OnceLock<LockPreview> = OnceLock::new();
    *P.get_or_init(|| match std::env::var("MIRADA_LOCK_PREVIEW") {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "hidden" | "off" | "0" | "no" | "false" => LockPreview::Hidden,
            _ => LockPreview::Live,
        },
        Err(_) => LockPreview::Live,
    })
}

/// Directorio de las miniaturas (en el runtime dir del compositor, o `/tmp`).
fn thumb_dir() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("mirada-thumbs")
}

/// Captura la preview de cada sesión hosteada y devuelve `(id, ruta)`. Vacío si
/// la preview está apagada (`hidden`) o si nada se pudo rendir. Una sesión sin
/// ventanas (o cuya GPU falla) se omite: el lock le pone tarjeta genérica.
pub(crate) fn capturar(app: &App, renderer: &mut GlesRenderer) -> Vec<(SessionId, PathBuf)> {
    if lock_preview() == LockPreview::Hidden {
        return Vec::new();
    }
    let (out_w, out_h) = app.output_size;
    if out_w <= 0 || out_h <= 0 {
        return Vec::new();
    }
    let dir = thumb_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return Vec::new();
    }
    let tbh = app.decorations.titlebar_height;
    let ids: Vec<SessionId> = app.roster.iter().map(|(id, _)| id).collect();
    let mut out = Vec::new();
    for id in ids {
        // Elementos de las ventanas de ESTA sesión (sin shell/greeter), en sus
        // posiciones globales — el offscreen cubre la salida entera.
        //
        // INVARIANTE (no romper): se filtra por `w.visible` —la visibilidad de
        // *layout*, pegajosa y agnóstica de sesión— y a propósito **NO** por
        // `session_visible(w)` (el gate FUS «¿es la activa?»). Son ejes
        // ortogonales: el compose en vivo exige los dos (`w.visible &&
        // session_visible(w)`), pero acá queremos justo las sesiones de fondo.
        // Sus ventanas conservan `visible == true` (su último layout, congelado
        // al saltar de sesión: nada pone `visible=false` al hostearlas) aunque
        // `session_visible` sea `false`. Si alguien agrega `&& session_visible`
        // «por prolijidad», sólo se capturaría la activa y el resto caería a
        // tarjeta genérica — que es el bug que esto previene.
        let mut elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
        for w in app
            .windows
            .iter()
            .filter(|w| w.visible && !w.is_shell && !w.is_greeter && w.session == id)
        {
            if !crate::buffer_render_sano(&w.surface) {
                continue;
            }
            let loc = crate::render_loc(w, out_h, tbh);
            elems.extend(render_elements_from_surface_tree(
                renderer,
                &w.surface,
                loc,
                1.0,
                w.effects.opacity as f32 / 255.0,
                Kind::Unspecified,
            ));
        }
        if elems.is_empty() {
            continue; // sesión sin ventanas pintables → tarjeta genérica
        }
        let Some(full) = crate::screencopy::render_elements_offscreen(renderer, (out_w, out_h), &elems)
        else {
            continue;
        };
        let (tw, th) = thumb_size(out_w as u32, out_h as u32);
        let small = downscale(&full, out_w as u32, out_h as u32, tw, th);
        let rgba = bgra_a_rgba(&small);
        let path = dir.join(format!("sesion-{}.thumb", id.0));
        if escribir(&path, tw, th, &rgba).is_ok() {
            out.push((id, path));
        }
    }
    out
}

/// Captura la **escena activa visible** (las ventanas de la sesión activa, sin
/// shell ni greeter) a una `GlesTexture` del tamaño del output — la captura
/// congelada que el **hero de lock** encoge hasta el thumbnail. `None` si no hay
/// nada que rendir o la GPU falla. A diferencia de [`capturar`] (que quiere las
/// sesiones de fondo), acá filtramos por `session_visible`: lo que se ve al
/// bloquear.
pub(crate) fn capturar_output(app: &App, renderer: &mut GlesRenderer) -> Option<GlesTexture> {
    let (out_w, out_h) = app.output_size;
    if out_w <= 0 || out_h <= 0 {
        return None;
    }
    let tbh = app.decorations.titlebar_height;
    let mut elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
    for w in app
        .windows
        .iter()
        .filter(|w| w.visible && !w.is_shell && !w.is_greeter && app.session_visible(w))
    {
        if !crate::buffer_render_sano(&w.surface) {
            continue;
        }
        let loc = crate::render_loc(w, out_h, tbh);
        elems.extend(render_elements_from_surface_tree(
            renderer,
            &w.surface,
            loc,
            1.0,
            w.effects.opacity as f32 / 255.0,
            Kind::Unspecified,
        ));
    }
    if elems.is_empty() {
        return None;
    }
    crate::screencopy::capturar_textura(renderer, (out_w, out_h), &elems)
}

/// El alto de la miniatura para `THUMB_W` de ancho, respetando el aspecto.
fn thumb_size(out_w: u32, out_h: u32) -> (u32, u32) {
    let w = THUMB_W.min(out_w.max(1));
    let h = ((w as u64 * out_h.max(1) as u64) / out_w.max(1) as u64).max(1) as u32;
    (w, h)
}

/// Achica `src` (RGBA `sw×sh`) a `dw×dh` por promedio de caja. Pura — testeable.
fn downscale(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    let mut dst = vec![0u8; dw * dh * 4];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < sw * sh * 4 {
        return dst;
    }
    for dy in 0..dh {
        let y0 = dy * sh / dh;
        let y1 = (((dy + 1) * sh / dh).max(y0 + 1)).min(sh);
        for dx in 0..dw {
            let x0 = dx * sw / dw;
            let x1 = (((dx + 1) * sw / dw).max(x0 + 1)).min(sw);
            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * sw + x) * 4;
                    r += src[i] as u32;
                    g += src[i + 1] as u32;
                    b += src[i + 2] as u32;
                    a += src[i + 3] as u32;
                    n += 1;
                }
            }
            let n = n.max(1);
            let o = (dy * dw + dx) * 4;
            dst[o] = (r / n) as u8;
            dst[o + 1] = (g / n) as u8;
            dst[o + 2] = (b / n) as u8;
            dst[o + 3] = (a / n) as u8;
        }
    }
    dst
}

/// El readback del offscreen viene `Xrgb8888` (bytes `B,G,R,X`); el greeter
/// quiere RGBA opaco. Intercambia R↔B y fuerza alfa a 255.
fn bgra_a_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; bgra.len()];
    for (px, o) in bgra.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        o[0] = px[2];
        o[1] = px[1];
        o[2] = px[0];
        o[3] = 255;
    }
    out
}

/// Escribe el archivo crudo: `MTH1` + w + h + RGBA.
fn escribir(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("thumb.tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(MAGIC)?;
    f.write_all(&w.to_le_bytes())?;
    f.write_all(&h.to_le_bytes())?;
    f.write_all(rgba)?;
    f.flush()?;
    // Rename atómico: el greeter nunca lee un archivo a medio escribir.
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspecto_se_respeta() {
        assert_eq!(thumb_size(1920, 1080), (320, 180));
        assert_eq!(thumb_size(1280, 800), (320, 200));
        // Salida más chica que el thumb: no agranda.
        assert_eq!(thumb_size(200, 100), (200, 100));
    }

    #[test]
    fn downscale_2x2_a_1x1_promedia() {
        // Cuatro píxeles → uno: promedio de cada canal.
        let src = vec![
            0, 0, 0, 255, // negro
            255, 255, 255, 255, // blanco
            255, 0, 0, 255, // (BGRA da igual: promedia por canal)
            0, 0, 255, 255,
        ];
        let d = downscale(&src, 2, 2, 1, 1);
        assert_eq!(d, vec![127, 63, 127, 255]);
    }

    #[test]
    fn swap_bgra_a_rgba_opaco() {
        // BGRX (B=1,G=2,R=3,X=9) → RGBA (3,2,1,255).
        assert_eq!(bgra_a_rgba(&[1, 2, 3, 9]), vec![3, 2, 1, 255]);
    }
}
