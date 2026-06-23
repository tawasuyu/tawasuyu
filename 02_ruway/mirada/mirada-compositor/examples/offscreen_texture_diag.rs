//! Diagnóstico headless: ¿el GlesRenderer de esta GPU/Mesa dibuja una textura
//! a un offscreen anidado? Es el PRIMER MOVIMIENTO del handoff
//! (`PREZI-ROTACION-HANDOFF.md`): mata la incógnita central de la rotación
//! viva del Prezi sin adivinar por commits.
//!
//! Camino: GbmDevice(renderD128) → EGLDisplay → EGLContext → GlesRenderer →
//! `import_memory` de un patrón RGBA conocido → `Offscreen::create_buffer` +
//! `bind` + `render` + `clear` + `render_texture_from_to` + `finish` →
//! `copy_framebuffer` + `map_texture` → contar colores del readback.
//!
//! Lectura del resultado:
//! - El readback trae los colores del patrón → el offscreen SÍ dibuja texturas.
//!   El bug del Prezi está en cómo se extrae/pasa la textura de la ventana
//!   (commit `05573e4d`); se depura ahí.
//! - El readback es ~monocromo (sólo el clear) → la Mesa NO dibuja texturas a
//!   un offscreen anidado → Plan B (render_texture rotado en el frame principal).
//!
//! Correr: `cargo run -p mirada-compositor --example offscreen_texture_diag`

use std::collections::HashSet;
use std::fs::OpenOptions;

use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::allocator::Fourcc;
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{
    Bind, Color32F, ExportMem, Frame as _, ImportMem, Offscreen, Renderer, Texture,
};
use smithay::utils::{
    Buffer as BufferCoord, Physical, Rectangle, Size, Transform,
};

const FOURCC: Fourcc = Fourcc::Abgr8888;

fn main() {
    if let Err(e) = run() {
        eprintln!("DIAG FALLÓ: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    // 1) GlesRenderer headless sobre el render node.
    let node = std::env::var("MIRADA_RENDER_NODE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
    println!("render node: {node}");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&node)
        .map_err(|e| format!("abrir {node}: {e}"))?;
    let gbm = GbmDevice::new(file).map_err(|e| format!("GbmDevice::new: {e}"))?;
    let egl_display = unsafe { EGLDisplay::new(gbm) }.map_err(|e| format!("EGLDisplay::new: {e}"))?;
    let egl_context = EGLContext::new(&egl_display).map_err(|e| format!("EGLContext::new: {e}"))?;
    let mut renderer =
        unsafe { GlesRenderer::new(egl_context) }.map_err(|e| format!("GlesRenderer::new: {e}"))?;
    println!("GlesRenderer listo.");

    // 2) Textura conocida: un patrón RGBA de 4 cuadrantes (rojo/verde/azul/
    //    amarillo) de 64×64. En memoria Abgr8888 = bytes [R,G,B,A] subidos como
    //    Abgr (smithay lee el orden por el Fourcc; usamos el mismo que el Prezi).
    let (tw_src, th_src) = (64usize, 64usize);
    let mut src_rgba = vec![0u8; tw_src * th_src * 4];
    for y in 0..th_src {
        for x in 0..tw_src {
            let i = (y * tw_src + x) * 4;
            let (r, g, b) = match (x < tw_src / 2, y < th_src / 2) {
                (true, true) => (255, 0, 0),    // rojo
                (false, true) => (0, 255, 0),   // verde
                (true, false) => (0, 0, 255),   // azul
                (false, false) => (255, 255, 0), // amarillo
            };
            src_rgba[i] = r;
            src_rgba[i + 1] = g;
            src_rgba[i + 2] = b;
            src_rgba[i + 3] = 255;
        }
    }
    let src_size: Size<i32, BufferCoord> = (tw_src as i32, th_src as i32).into();
    let tex: GlesTexture = renderer
        .import_memory(&src_rgba, FOURCC, src_size, false)
        .map_err(|e| format!("import_memory: {e}"))?;
    println!("textura patrón importada: {:?}", tex.size());

    // 3) Offscreen: clear gris oscuro + dibujar la textura cubriendo todo.
    let (tw, th) = (128i32, 128i32);
    let buffer_size: Size<i32, BufferCoord> = (tw, th).into();
    let mut off = Offscreen::<GlesTexture>::create_buffer(&mut renderer, FOURCC, buffer_size)
        .map_err(|e| format!("create_buffer: {e}"))?;
    let mut target = renderer.bind(&mut off).map_err(|e| format!("bind: {e}"))?;
    let fisico: Size<i32, Physical> = (tw, th).into();
    let dmg = [Rectangle::from_size(fisico)];
    let clear = Color32F::from([0.1, 0.1, 0.1, 1.0]);
    {
        let mut frame = renderer
            .render(&mut target, fisico, Transform::Normal)
            .map_err(|e| format!("render: {e}"))?;
        frame.clear(clear, &dmg).map_err(|e| format!("clear: {e}"))?;
        let src = Rectangle::from_size(tex.size().to_f64());
        let dst: Rectangle<i32, Physical> = Rectangle::from_size((tw, th).into());
        frame
            .render_texture_from_to(&tex, src, dst, &dmg, &[], Transform::Normal, 1.0, None, &[])
            .map_err(|e| format!("render_texture_from_to: {e}"))?;
        frame.finish().map_err(|e| format!("finish: {e}"))?;
    }

    // 4) Readback + conteo de colores.
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

    let mut buckets: HashSet<(u8, u8, u8)> = HashSet::new();
    let mut clear_like = 0usize;
    let mut total = 0usize;
    for px in bytes[..w * h * 4].chunks_exact(4) {
        buckets.insert((px[0] / 40, px[1] / 40, px[2] / 40));
        // Cuántos pixeles quedaron en el color del clear (gris ~26).
        if px[0] < 50 && px[1] < 50 && px[2] < 50 {
            clear_like += 1;
        }
        total += 1;
    }

    println!("--- RESULTADO ---");
    println!("buckets de color distintos: {}", buckets.len());
    println!(
        "pixeles 'clear' (oscuros): {clear_like}/{total} ({}%)",
        clear_like * 100 / total.max(1)
    );
    // Muestreo de colores presentes (los primeros buckets).
    let mut sample: Vec<_> = buckets.iter().take(12).collect();
    sample.sort();
    println!("muestra buckets (r/40,g/40,b/40): {sample:?}");

    if buckets.len() >= 4 && clear_like < total * 9 / 10 {
        println!("\n✅ EL OFFSCREEN DIBUJA TEXTURAS. El bug del Prezi está en la");
        println!("   extracción/paso de la textura de la ventana (commit 05573e4d).");
    } else {
        println!("\n❌ EL OFFSCREEN NO DIBUJA TEXTURAS (solo el clear).");
        println!("   La Mesa no pinta texturas a un offscreen anidado → Plan B.");
    }
    Ok(())
}
