//! Diagnóstico headless DECISIVO para la rotación viva del Prezi: ¿una textura
//! **de ventana** (que viene de un dmabuf → `EGL_TEXTURE_EXTERNAL_OES`, no un
//! `GL_TEXTURE_2D` como `import_memory`) se dibuja a un offscreen anidado con
//! `render_texture_from_to`?
//!
//! `offscreen_texture_diag` ya probó que un `import_memory` (texture 2D normal,
//! como el badge del número) SÍ se dibuja al offscreen en este metal. Lo que
//! NO cubrió es la textura de una superficie cliente real, que es external-OES.
//! Esa es la única incógnita que queda para `render_tile_live_rotated`.
//!
//! Camino fiel a una ventana:
//!   1. Allocar un dmabuf por gbm (como el buffer de un cliente).
//!   2. Pintarle adentro un patrón conocido (bind dmabuf + dibujar un 2D dentro).
//!   3. `import_dmabuf` → GlesTexture external-OES (= lo que da una ventana).
//!   4. Dibujar ESA textura a un offscreen con `render_texture_from_to`.
//!   5. Readback + contar colores.
//!
//! Lectura:
//! - Readback con los colores del patrón → external-OES SÍ se dibuja al
//!   offscreen → el camino del Prezi (commit 05573e4d) debería funcionar; el
//!   bug, si lo hay, está en la EXTRACCIÓN (`with_renderer_surface_state`
//!   devolviendo None), no en el dibujo. Se confirma corriendo el compositor.
//! - Readback monocromo → external-OES NO se dibuja al offscreen anidado en
//!   esta Mesa → ESE es el bug: hay que dibujar la ventana rotada de otra forma
//!   (Plan B: render_texture rotado en el frame principal, donde sí anda).
//!
//! Correr: `cargo run -p mirada-compositor --example offscreen_dmabuf_diag`

use std::collections::HashSet;
use std::fs::OpenOptions;

use smithay::backend::allocator::dmabuf::AsDmabuf;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{
    Bind, Color32F, ExportMem, Frame as _, ImportDma, ImportMem, Offscreen, Renderer, Texture,
};
use smithay::utils::{Buffer as BufferCoord, Physical, Rectangle, Size, Transform};

const FOURCC: Fourcc = Fourcc::Abgr8888;

fn main() {
    if let Err(e) = run() {
        eprintln!("DIAG FALLÓ: {e}");
        std::process::exit(1);
    }
}

fn patron_rgba(w: usize, h: usize) -> Vec<u8> {
    let mut v = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let (r, g, b) = match (x < w / 2, y < h / 2) {
                (true, true) => (255, 0, 0),
                (false, true) => (0, 255, 0),
                (true, false) => (0, 0, 255),
                (false, false) => (255, 255, 0),
            };
            v[i] = r;
            v[i + 1] = g;
            v[i + 2] = b;
            v[i + 3] = 255;
        }
    }
    v
}

fn contar(bytes: &[u8], w: usize, h: usize) -> (usize, usize, usize) {
    let mut buckets: HashSet<(u8, u8, u8)> = HashSet::new();
    let mut clear_like = 0usize;
    let mut total = 0usize;
    for px in bytes[..w * h * 4].chunks_exact(4) {
        buckets.insert((px[0] / 40, px[1] / 40, px[2] / 40));
        if px[0] < 50 && px[1] < 50 && px[2] < 50 {
            clear_like += 1;
        }
        total += 1;
    }
    (buckets.len(), clear_like, total)
}

fn run() -> Result<(), String> {
    let node = std::env::var("MIRADA_RENDER_NODE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
    println!("render node: {node}");
    let abrir = || {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&node)
            .map_err(|e| format!("abrir {node}: {e}"))
    };
    // `GbmDevice<File>` no es Clone; abrimos el node dos veces — un gbm para el
    // allocator (el dmabuf de la 'ventana') y otro para EGL/el renderer. El
    // dmabuf cruza por fd, mismo GPU: import trivial.
    let gbm_alloc = GbmDevice::new(abrir()?).map_err(|e| format!("GbmDevice::new alloc: {e}"))?;
    let gbm_egl = GbmDevice::new(abrir()?).map_err(|e| format!("GbmDevice::new egl: {e}"))?;
    let mut allocator = GbmAllocator::new(gbm_alloc, GbmBufferFlags::RENDERING);
    let egl_display =
        unsafe { EGLDisplay::new(gbm_egl) }.map_err(|e| format!("EGLDisplay::new: {e}"))?;
    let egl_context = EGLContext::new(&egl_display).map_err(|e| format!("EGLContext::new: {e}"))?;
    let mut renderer =
        unsafe { GlesRenderer::new(egl_context) }.map_err(|e| format!("GlesRenderer::new: {e}"))?;
    println!("GlesRenderer listo.");

    // 1) Patrón conocido como textura 2D de memoria (la usamos para pintar el dmabuf).
    let (sw, sh) = (64usize, 64usize);
    let src_rgba = patron_rgba(sw, sh);
    let src_size: Size<i32, BufferCoord> = (sw as i32, sh as i32).into();
    let tex2d: GlesTexture = renderer
        .import_memory(&src_rgba, FOURCC, src_size, false)
        .map_err(|e| format!("import_memory: {e}"))?;

    // 2) Allocar un dmabuf (como el buffer de una ventana) y PINTARLE el patrón
    //    adentro (bind dmabuf como target + render_texture_from_to del 2D).
    let gbm_buf = allocator
        .create_buffer(sw as u32, sh as u32, FOURCC, &[Modifier::Linear])
        .map_err(|e| format!("create_buffer dmabuf: {e}"))?;
    let mut dmabuf = gbm_buf.export().map_err(|e| format!("export dmabuf: {e}"))?;
    {
        let mut target = renderer
            .bind(&mut dmabuf)
            .map_err(|e| format!("bind dmabuf: {e}"))?;
        let fis: Size<i32, Physical> = (sw as i32, sh as i32).into();
        let dmg = [Rectangle::from_size(fis)];
        let mut frame = renderer
            .render(&mut target, fis, Transform::Normal)
            .map_err(|e| format!("render→dmabuf: {e}"))?;
        frame
            .clear(Color32F::from([0.0, 0.0, 0.0, 1.0]), &dmg)
            .map_err(|e| format!("clear dmabuf: {e}"))?;
        let src = Rectangle::from_size(tex2d.size().to_f64());
        let dst: Rectangle<i32, Physical> = Rectangle::from_size((sw as i32, sh as i32).into());
        frame
            .render_texture_from_to(&tex2d, src, dst, &dmg, &[], Transform::Normal, 1.0, None, &[])
            .map_err(|e| format!("pintar dmabuf: {e}"))?;
        let _ = frame.finish().map_err(|e| format!("finish dmabuf: {e}"))?;
    }

    // 3) Importar el dmabuf como textura — ESTO es lo que da una ventana cliente:
    //    una GlesTexture external-OES (no 2D).
    let win_tex: GlesTexture = renderer
        .import_dmabuf(&dmabuf, None)
        .map_err(|e| format!("import_dmabuf (external-OES): {e}"))?;
    println!(
        "textura de 'ventana' (dmabuf/external-OES) importada: {:?}",
        win_tex.size()
    );

    // 4) Dibujar esa textura external-OES a un offscreen (el camino exacto del
    //    Prezi: render_offscreen_drawing → render_texture_from_to).
    let (tw, th) = (128i32, 128i32);
    let buffer_size: Size<i32, BufferCoord> = (tw, th).into();
    let mut off = Offscreen::<GlesTexture>::create_buffer(&mut renderer, FOURCC, buffer_size)
        .map_err(|e| format!("create_buffer offscreen: {e}"))?;
    let mut target = renderer.bind(&mut off).map_err(|e| format!("bind offscreen: {e}"))?;
    let fis: Size<i32, Physical> = (tw, th).into();
    let dmg = [Rectangle::from_size(fis)];
    {
        let mut frame = renderer
            .render(&mut target, fis, Transform::Normal)
            .map_err(|e| format!("render offscreen: {e}"))?;
        frame
            .clear(Color32F::from([0.1, 0.1, 0.1, 1.0]), &dmg)
            .map_err(|e| format!("clear offscreen: {e}"))?;
        let src = Rectangle::from_size(win_tex.size().to_f64());
        let dst: Rectangle<i32, Physical> = Rectangle::from_size((tw, th).into());
        frame
            .render_texture_from_to(&win_tex, src, dst, &dmg, &[], Transform::Normal, 1.0, None, &[])
            .map_err(|e| format!("render_texture_from_to external-OES: {e}"))?;
        let _ = frame.finish().map_err(|e| format!("finish offscreen: {e}"))?;
    }

    // 5) Readback + conteo.
    let rect: Rectangle<i32, BufferCoord> = Rectangle::from_size(buffer_size);
    let mapping = renderer
        .copy_framebuffer(&target, rect, FOURCC)
        .map_err(|e| format!("copy_framebuffer: {e}"))?;
    let bytes = renderer
        .map_texture(&mapping)
        .map_err(|e| format!("map_texture: {e}"))?;
    let (w, h) = (tw as usize, th as usize);
    if bytes.len() < w * h * 4 {
        return Err(format!("mapping corto: {} < {}", bytes.len(), w * h * 4));
    }
    let (nb, clear_like, total) = contar(&bytes, w, h);
    println!("--- RESULTADO (textura external-OES → offscreen) ---");
    println!("buckets de color distintos: {nb}");
    println!(
        "pixeles 'clear' (oscuros): {clear_like}/{total} ({}%)",
        clear_like * 100 / total.max(1)
    );

    if nb >= 4 && clear_like < total * 9 / 10 {
        println!("\n✅ LAS TEXTURAS DE VENTANA (external-OES) SE DIBUJAN AL OFFSCREEN.");
        println!("   El dibujo del Prezi anda; si falla es por EXTRACCIÓN (texture()");
        println!("   devuelve None). Confirmar corriendo el compositor en DRM.");
    } else {
        println!("\n❌ LAS TEXTURAS external-OES NO SE DIBUJAN AL OFFSCREEN.");
        println!("   Ese es el bug real → Plan B (render_texture rotado en el frame");
        println!("   principal, donde las texturas de ventana sí se dibujan).");
    }
    Ok(())
}
