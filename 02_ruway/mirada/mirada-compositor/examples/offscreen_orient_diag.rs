//! Diagnóstico de ORIENTACIÓN del camino offscreen del Prezi («queda de
//! cabeza»). Replica EXACTO lo que hace `screencopy::render_offscreen_drawing`
//! (create_buffer Abgr8888 + bind + render Normal + clear + render_texture_from_to
//! + finish + copy_framebuffer Xrgb8888 + map_texture + corrección flip por
//! `mapping.flipped()`) con un patrón de 4 esquinas conocidas, y reporta qué
//! color cae en cada esquina del readback. Para textura 2D (import_memory, = el
//! badge) y para external-OES (dmabuf, = una ventana). Distingue:
//!   - identidad: TL=rojo TR=verde BL=azul BR=amarillo
//!   - flip vertical: TL=azul TR=amarillo BL=rojo BR=verde
//!   - flip horizontal: TL=verde TR=rojo BL=amarillo BR=azul
//!   - 180°: TL=amarillo TR=azul BL=verde BR=rojo
//!
//! Correr: `cargo run -p mirada-compositor --example offscreen_orient_diag`

use std::fs::OpenOptions;

use smithay::backend::allocator::dmabuf::AsDmabuf;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{
    Bind, Color32F, ExportMem, Frame as _, ImportDma, ImportMem, Offscreen, Renderer, Texture,
    TextureMapping,
};
use smithay::utils::{Buffer as BufferCoord, Physical, Rectangle, Size, Transform};

// El readback usa este fourcc, igual que `render_offscreen_drawing` (= Xrgb8888,
// bytes [B,G,R,X]). El buffer offscreen se crea Abgr8888 como en el Prezi.
const READ_FOURCC: Fourcc = Fourcc::Xrgb8888;
const BUF_FOURCC: Fourcc = Fourcc::Abgr8888;

fn patron_rgba(w: usize, h: usize) -> Vec<u8> {
    let mut v = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let (r, g, b) = match (x < w / 2, y < h / 2) {
                (true, true) => (255, 0, 0),     // TL rojo
                (false, true) => (0, 255, 0),    // TR verde
                (true, false) => (0, 0, 255),    // BL azul
                (false, false) => (255, 255, 0), // BR amarillo
            };
            v[i] = r;
            v[i + 1] = g;
            v[i + 2] = b;
            v[i + 3] = 255;
        }
    }
    v
}

/// Nombre del color dominante de un pixel Xrgb8888 = [B,G,R,X].
fn nombre_bgrx(px: &[u8]) -> &'static str {
    let (b, g, r) = (px[0], px[1], px[2]);
    let alto = |v: u8| v > 150;
    let bajo = |v: u8| v < 100;
    match (alto(r), alto(g), alto(b)) {
        (true, false, false) => "rojo",
        (false, true, false) => "verde",
        (false, false, true) => "azul",
        (true, true, false) => "amarillo",
        _ if bajo(r) && bajo(g) && bajo(b) => "negro/clear",
        _ => "mixto",
    }
}

/// Lee las 4 esquinas (con un margen para no caer en el borde de cuadrante).
fn esquinas(out: &[u8], w: usize, h: usize) -> [(&'static str, &'static str); 4] {
    let m = (w.min(h) / 8).max(2);
    let at = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        nombre_bgrx(&out[i..i + 4])
    };
    [
        ("TL", at(m, m)),
        ("TR", at(w - 1 - m, m)),
        ("BL", at(m, h - 1 - m)),
        ("BR", at(w - 1 - m, h - 1 - m)),
    ]
}

fn diagnostico(esq: &[(&'static str, &'static str); 4]) -> &'static str {
    let c = |n: &str| esq.iter().find(|(p, _)| *p == n).map(|(_, c)| *c).unwrap_or("?");
    match (c("TL"), c("TR"), c("BL"), c("BR")) {
        ("rojo", "verde", "azul", "amarillo") => "IDENTIDAD (orientación correcta)",
        ("azul", "amarillo", "rojo", "verde") => "FLIP VERTICAL (de cabeza arriba/abajo)",
        ("verde", "rojo", "amarillo", "azul") => "FLIP HORIZONTAL (espejado izq/der)",
        ("amarillo", "azul", "verde", "rojo") => "ROTADO 180° (de cabeza)",
        _ => "OTRO (ver esquinas)",
    }
}

/// El núcleo: replica `render_offscreen_drawing` dibujando `tex` a todo el
/// offscreen, y devuelve el buffer corregido de orientación.
fn offscreen_dibujando(
    renderer: &mut GlesRenderer,
    tex: &GlesTexture,
    tw: i32,
    th: i32,
) -> Result<Vec<u8>, String> {
    let buffer_size: Size<i32, BufferCoord> = (tw, th).into();
    let mut off = Offscreen::<GlesTexture>::create_buffer(renderer, BUF_FOURCC, buffer_size)
        .map_err(|e| format!("create_buffer: {e}"))?;
    let mut target = renderer.bind(&mut off).map_err(|e| format!("bind: {e}"))?;
    let fis: Size<i32, Physical> = (tw, th).into();
    let dmg = [Rectangle::from_size(fis)];
    {
        let mut frame = renderer
            .render(&mut target, fis, Transform::Normal)
            .map_err(|e| format!("render: {e}"))?;
        frame
            .clear(Color32F::from([0.0, 0.0, 0.0, 1.0]), &dmg)
            .map_err(|e| format!("clear: {e}"))?;
        let src = Rectangle::from_size(tex.size().to_f64());
        let dst: Rectangle<i32, Physical> = Rectangle::from_size((tw, th).into());
        frame
            .render_texture_from_to(tex, src, dst, &dmg, &[], Transform::Normal, 1.0, None, &[])
            .map_err(|e| format!("render_texture_from_to: {e}"))?;
        let _ = frame.finish().map_err(|e| format!("finish: {e}"))?;
    }
    let rect: Rectangle<i32, BufferCoord> = Rectangle::from_size(buffer_size);
    let mapping = renderer
        .copy_framebuffer(&target, rect, READ_FOURCC)
        .map_err(|e| format!("copy_framebuffer: {e}"))?;
    let bytes = renderer.map_texture(&mapping).map_err(|e| format!("map_texture: {e}"))?;
    let (w, h) = (tw as usize, th as usize);
    if bytes.len() < w * h * 4 {
        return Err("mapping corto".into());
    }
    let crudo = bytes[..w * h * 4].to_vec();
    let ec = esquinas(&crudo, w, h);
    println!("  CRUDO (sin corregir): {ec:?} → {}", diagnostico(&ec));
    let mut out = bytes[..w * h * 4].to_vec();
    // Misma corrección que render_offscreen_drawing.
    if mapping.flipped() {
        let row = w * 4;
        for y in 0..h / 2 {
            let (a, b) = (y * row, (h - 1 - y) * row);
            for k in 0..row {
                out.swap(a + k, b + k);
            }
        }
    }
    println!("  (mapping.flipped() = {})", mapping.flipped());
    Ok(out)
}

fn run() -> Result<(), String> {
    let node = std::env::var("MIRADA_RENDER_NODE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
    let abrir = || {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&node)
            .map_err(|e| format!("abrir {node}: {e}"))
    };
    let gbm_alloc = GbmDevice::new(abrir()?).map_err(|e| format!("gbm alloc: {e}"))?;
    let gbm_egl = GbmDevice::new(abrir()?).map_err(|e| format!("gbm egl: {e}"))?;
    let mut allocator = GbmAllocator::new(gbm_alloc, GbmBufferFlags::RENDERING);
    let egl_display = unsafe { EGLDisplay::new(gbm_egl) }.map_err(|e| format!("EGLDisplay: {e}"))?;
    let egl_context = EGLContext::new(&egl_display).map_err(|e| format!("EGLContext: {e}"))?;
    let mut renderer = unsafe { GlesRenderer::new(egl_context) }.map_err(|e| format!("Gles: {e}"))?;

    let (sw, sh) = (64usize, 64usize);
    let rgba = patron_rgba(sw, sh);
    let src_size: Size<i32, BufferCoord> = (sw as i32, sh as i32).into();

    // CASO 1 — textura 2D (import_memory, = el badge del número).
    println!("== CASO 2D (import_memory, como el badge) ==");
    let tex2d = renderer
        .import_memory(&rgba, BUF_FOURCC, src_size, false)
        .map_err(|e| format!("import_memory: {e}"))?;
    let out2d = offscreen_dibujando(&mut renderer, &tex2d, 128, 128)?;
    let e2d = esquinas(&out2d, 128, 128);
    println!("  esquinas: {e2d:?}");
    println!("  → {}", diagnostico(&e2d));

    // CASO 2 — textura external-OES (dmabuf, = una ventana cliente). Se pinta el
    // patrón adentro y se reimporta, igual que offscreen_dmabuf_diag.
    println!("== CASO external-OES (dmabuf, como una ventana) ==");
    let gbm_buf = allocator
        .create_buffer(sw as u32, sh as u32, BUF_FOURCC, &[Modifier::Linear])
        .map_err(|e| format!("create_buffer dmabuf: {e}"))?;
    let mut dmabuf = gbm_buf.export().map_err(|e| format!("export: {e}"))?;
    {
        let mut target = renderer.bind(&mut dmabuf).map_err(|e| format!("bind dmabuf: {e}"))?;
        let fis: Size<i32, Physical> = (sw as i32, sh as i32).into();
        let dmg = [Rectangle::from_size(fis)];
        let mut frame = renderer
            .render(&mut target, fis, Transform::Normal)
            .map_err(|e| format!("render dmabuf: {e}"))?;
        let src = Rectangle::from_size(tex2d.size().to_f64());
        let dst: Rectangle<i32, Physical> = Rectangle::from_size((sw as i32, sh as i32).into());
        frame
            .render_texture_from_to(&tex2d, src, dst, &dmg, &[], Transform::Normal, 1.0, None, &[])
            .map_err(|e| format!("pintar dmabuf: {e}"))?;
        let _ = frame.finish().map_err(|e| format!("finish dmabuf: {e}"))?;
    }
    let texoes = renderer
        .import_dmabuf(&dmabuf, None)
        .map_err(|e| format!("import_dmabuf: {e}"))?;
    let outoes = offscreen_dibujando(&mut renderer, &texoes, 128, 128)?;
    let eoes = esquinas(&outoes, 128, 128);
    println!("  esquinas: {eoes:?}");
    println!("  → {}", diagnostico(&eoes));

    println!("\nNOTA: el patrón pintado en el dmabuf pasó por un render Normal, así");
    println!("que un flip del CASO external-OES respecto al 2D delata el flip de");
    println!("Wayland en la textura de superficie (lo que hace 'de cabeza' al Prezi).");
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("DIAG FALLÓ: {e}");
        std::process::exit(1);
    }
}
