//! Paridad CPU↔GPU: el compositor GPU debe reproducir al CPU dentro de ±1 por
//! canal (sólo difiere el desempate del redondeo en `.5`).
//!
//! Si la máquina no tiene adaptador GPU (CI headless sin Vulkan), el test se
//! salta con un aviso en vez de fallar — `cargo test` sigue verde.

use image::RgbaImage;
use tullpu_core::{Capa, Lienzo, ModoFusion};
use tullpu_render::{buffer_mascara, buffer_solido, componer, AlmacenEnMemoria};
use tullpu_render_gpu::Compositor;

/// Patrón de gradiente determinista `w*h` rgba8 — da contenido no trivial
/// (alfa variable incluido) para ejercitar las fórmulas de fusión.
fn buffer_gradiente(w: u32, h: u32, sesgo: u8) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let r = ((x * 255) / w.max(1)) as u8;
            let g = ((y * 255) / h.max(1)) as u8;
            let b = sesgo.wrapping_add((x ^ y) as u8);
            let a = (128 + (((x + y) * 64) / (w + h).max(1)) as u8).min(255);
            v.extend_from_slice(&[r, g, b, a]);
        }
    }
    v
}

/// Máximo |diff| por canal entre dos imágenes del mismo tamaño.
fn max_diff(a: &RgbaImage, b: &RgbaImage) -> i32 {
    assert_eq!(a.dimensions(), b.dimensions(), "tamaños distintos");
    a.as_raw()
        .iter()
        .zip(b.as_raw().iter())
        .map(|(&x, &y)| (x as i32 - y as i32).abs())
        .max()
        .unwrap_or(0)
}

/// Intenta construir el compositor GPU; `None` ⇒ no hay adaptador (saltar).
fn compositor() -> Option<Compositor> {
    match Compositor::nuevo() {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("SKIP: sin GPU disponible ({e})");
            None
        }
    }
}

#[test]
fn paridad_todos_los_modos_de_fusion() {
    let Some(gpu) = compositor() else { return };
    let (w, h) = (16, 12);

    // Lista completa de modos soportados por el shader (sin Disolver).
    let modos = [
        ModoFusion::Normal,
        ModoFusion::Multiplicar,
        ModoFusion::Pantalla,
        ModoFusion::Superponer,
        ModoFusion::Aclarar,
        ModoFusion::Oscurecer,
        ModoFusion::Diferencia,
        ModoFusion::Aditivo,
        ModoFusion::SubExpQuemado,
        ModoFusion::SubLinealQuemado,
        ModoFusion::SobreExpAclarado,
        ModoFusion::LuzFuerte,
        ModoFusion::LuzSuave,
        ModoFusion::LuzViva,
        ModoFusion::LuzLineal,
        ModoFusion::LuzPunto,
        ModoFusion::MezclaDura,
        ModoFusion::Exclusion,
        ModoFusion::Resta,
        ModoFusion::Division,
        ModoFusion::HslTono,
        ModoFusion::HslSaturacion,
        ModoFusion::HslColor,
        ModoFusion::HslLuminosidad,
        ModoFusion::ColorMasOscuro,
        ModoFusion::ColorMasClaro,
    ];

    for modo in modos {
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_gradiente(w, h, 30));
        let top = alm.insertar(buffer_gradiente(w, h, 180));
        let mut l = Lienzo::nuevo(w, h);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.blend = modo;
        c.opacidad = 0.8;
        l.apilar(c);

        let cpu = componer(&l, &alm).unwrap();
        let g = gpu.componer(&l, &alm).unwrap();
        let d = max_diff(&cpu, &g);
        assert!(d <= 1, "modo {modo:?}: diff máximo {d} > 1");
    }
}

#[test]
fn paridad_mascara_y_clipping_y_grupos() {
    let Some(gpu) = compositor() else { return };
    let (w, h) = (20, 20);
    let mut alm = AlmacenEnMemoria::nuevo();

    let fondo = alm.insertar(buffer_gradiente(w, h, 10));
    let base = alm.insertar(buffer_solido(w, h, [200, 40, 40, 200]));
    let recorte = alm.insertar(buffer_gradiente(w, h, 90));
    let enmascarado = alm.insertar(buffer_solido(w, h, [40, 200, 90, 255]));
    let dentro_grupo = alm.insertar(buffer_gradiente(w, h, 150));

    // Máscara en degradé horizontal (mitad oculta).
    let mut mbytes = buffer_mascara(w, h, 255);
    for y in 0..h {
        for x in 0..w {
            mbytes[(y * w + x) as usize] = ((x * 255) / w) as u8;
        }
    }
    let masc = alm.insertar(mbytes);

    let mut l = Lienzo::nuevo(w, h);
    l.apilar(Capa::raster("fondo", fondo));

    // Capa con máscara + opacidad + blend.
    let mut cm = Capa::raster("enmascarada", enmascarado);
    cm.mascara = Some(masc);
    cm.opacidad = 0.7;
    cm.blend = ModoFusion::Pantalla;
    l.apilar(cm);

    // Base de clipping + capa recortada a su alfa.
    l.apilar(Capa::raster("base", base));
    let mut cc = Capa::raster("recorte", recorte);
    cc.clipping = true;
    cc.blend = ModoFusion::Multiplicar;
    l.apilar(cc);

    // Grupo con opacidad y un hijo.
    let mut g = Capa::grupo("carpeta");
    g.opacidad = 0.6;
    g.blend = ModoFusion::Superponer;
    let gid = g.id;
    l.apilar(g);
    let mut hijo = Capa::raster("hijo", dentro_grupo);
    hijo.grupo = Some(gid);
    hijo.opacidad = 0.9;
    l.apilar(hijo);

    let cpu = componer(&l, &alm).unwrap();
    let gg = gpu.componer(&l, &alm).unwrap();
    let d = max_diff(&cpu, &gg);
    assert!(d <= 1, "máscara/clip/grupo: diff máximo {d} > 1");
}

#[test]
fn lienzo_con_ajuste_cae_a_cpu() {
    use tullpu_core::OpLocal;
    let Some(gpu) = compositor() else { return };
    let mut alm = AlmacenEnMemoria::nuevo();
    let fondo = alm.insertar(buffer_solido(4, 4, [100, 100, 100, 255]));
    let mut l = Lienzo::nuevo(4, 4);
    l.apilar(Capa::raster("fondo", fondo));
    l.apilar(Capa::ajuste("brillo", OpLocal::Brillo { delta: 0.2 }));

    // El compositor GPU debe rechazar el lienzo (no quemar un resultado malo).
    let r = gpu.componer(&l, &alm);
    assert!(
        matches!(r, Err(tullpu_render_gpu::Error::NoSoportado)),
        "esperaba NoSoportado, fue {r:?}"
    );
}
