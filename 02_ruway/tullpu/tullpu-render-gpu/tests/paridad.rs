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
fn paridad_capas_de_ajuste() {
    use tullpu_core::OpLocal;
    let Some(gpu) = compositor() else { return };
    let (w, h) = (24, 18);

    // Una op de cada clase: LUT canal-independiente (Invertir/Brillo/Contraste/
    // Niveles/Curvas) y HSL (Saturacion/Tonalidad).
    let ops: Vec<(&str, OpLocal)> = vec![
        ("invertir", OpLocal::Invertir),
        ("brillo", OpLocal::Brillo { delta: 0.25 }),
        ("brillo_neg", OpLocal::Brillo { delta: -0.3 }),
        ("contraste", OpLocal::Contraste { factor: 1.6 }),
        (
            "niveles",
            OpLocal::Niveles { entrada_min: 0.1, entrada_max: 0.85, gamma: 1.4 },
        ),
        (
            "curvas",
            OpLocal::Curvas { puntos: vec![(0.0, 0.1), (0.4, 0.65), (1.0, 0.9)] },
        ),
        ("saturacion_baja", OpLocal::Saturacion { factor: 0.3 }),
        ("saturacion_alta", OpLocal::Saturacion { factor: 1.8 }),
        ("tonalidad", OpLocal::Tonalidad { grados: 90.0 }),
    ];

    // Máscara en degradé vertical para ejercitar la mezcla por píxel.
    let mut mbytes = buffer_mascara(w, h, 255);
    for y in 0..h {
        for x in 0..w {
            mbytes[(y * w + x) as usize] = ((y * 255) / h) as u8;
        }
    }

    for (nombre, op) in ops {
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_gradiente(w, h, 60));
        let masc = alm.insertar(mbytes.clone());
        let mut l = Lienzo::nuevo(w, h);
        l.apilar(Capa::raster("fondo", fondo));
        let mut a = Capa::ajuste(nombre, op);
        a.opacidad = 0.8;
        a.mascara = Some(masc);
        l.apilar(a);

        let cpu = componer(&l, &alm).unwrap();
        let g = gpu.componer(&l, &alm).unwrap();
        let d = max_diff(&cpu, &g);
        assert!(d <= 1, "ajuste {nombre}: diff máximo {d} > 1");
    }
}

#[test]
fn paridad_tilereado_por_bandas() {
    // Composición tilereada (bandas chicas forzadas) vs CPU: prueba que el
    // ensamblado por bandas y el índice GLOBAL del RNG de Disolver sobreviven al
    // tiling. Escena rica: fondo, capa con máscara+blend, clipping, grupo y una
    // capa Disolver (la sensible al índice global).
    use tullpu_core::OpLocal;
    let Some(gpu) = compositor() else { return };
    let (w, h) = (20, 41); // alto primo → la última banda queda corta
    let mut alm = AlmacenEnMemoria::nuevo();

    let fondo = alm.insertar(buffer_gradiente(w, h, 15));
    let base = alm.insertar(buffer_solido(w, h, [180, 60, 60, 210]));
    let recorte = alm.insertar(buffer_gradiente(w, h, 100));
    let disuelta = alm.insertar(buffer_gradiente(w, h, 200));
    let hijo_buf = alm.insertar(buffer_solido(w, h, [50, 170, 90, 255]));

    let mut mbytes = buffer_mascara(w, h, 255);
    for y in 0..h {
        for x in 0..w {
            mbytes[(y * w + x) as usize] = (((x + y) * 255) / (w + h)) as u8;
        }
    }
    let masc = alm.insertar(mbytes);

    let mut l = Lienzo::nuevo(w, h);
    l.apilar(Capa::raster("fondo", fondo));
    let mut cm = Capa::raster("masc", disuelta);
    cm.mascara = Some(masc);
    cm.blend = ModoFusion::Pantalla;
    cm.opacidad = 0.75;
    l.apilar(cm);
    l.apilar(Capa::raster("base", base));
    let mut cc = Capa::raster("recorte", recorte);
    cc.clipping = true;
    cc.blend = ModoFusion::Multiplicar;
    l.apilar(cc);
    // Ajuste (LUT) para cubrir también el camino de ajuste tilereado.
    let mut aj = Capa::ajuste("curvas", OpLocal::Curvas { puntos: vec![(0.0, 0.05), (0.5, 0.6), (1.0, 0.95)] });
    aj.opacidad = 0.7;
    l.apilar(aj);
    // Grupo con un hijo en modo Disolver (RNG por índice global).
    let mut g = Capa::grupo("carpeta");
    g.opacidad = 0.85;
    let gid = g.id;
    l.apilar(g);
    let mut hijo = Capa::raster("disuelto", hijo_buf);
    hijo.grupo = Some(gid);
    hijo.blend = ModoFusion::Disolver;
    hijo.opacidad = 0.55;
    l.apilar(hijo);

    let cpu = componer(&l, &alm).unwrap();
    // Una sola banda (camino normal) y bandas de 7 filas (camino tilereado).
    let entero = gpu.componer(&l, &alm).unwrap();
    let tilereado = gpu.componer_con_filas_por_banda(&l, &alm, 7).unwrap();

    assert!(max_diff(&cpu, &entero) <= 1, "entero vs CPU > 1");
    // El tilereado debe coincidir con el no-tilereado BIT a BIT: misma math, sólo
    // cambia la partición.
    assert_eq!(
        max_diff(&entero, &tilereado),
        0,
        "tilereado difiere del entero — el banding cambió el resultado"
    );
    assert!(max_diff(&cpu, &tilereado) <= 1, "tilereado vs CPU > 1");
}

#[test]
fn paridad_disolver() {
    // Disolver es un umbralizador binario sembrado por el Uuid de la capa: si el
    // splitmix64 emulado coincide con la CPU, la paridad es EXACTA (diff 0). Se
    // ejercita con alfa variable (degradé) y opacidad < 1 para que el umbral
    // discrimine de verdad.
    let Some(gpu) = compositor() else { return };
    let (w, h) = (32, 24);
    let mut alm = AlmacenEnMemoria::nuevo();
    let fondo = alm.insertar(buffer_solido(w, h, [20, 20, 60, 255]));
    let top = alm.insertar(buffer_gradiente(w, h, 120));
    let mut l = Lienzo::nuevo(w, h);
    l.apilar(Capa::raster("fondo", fondo));
    let mut c = Capa::raster("disuelta", top);
    c.blend = ModoFusion::Disolver;
    c.opacidad = 0.6;
    l.apilar(c);

    let cpu = componer(&l, &alm).unwrap();
    let g = gpu.componer(&l, &alm).unwrap();
    let d = max_diff(&cpu, &g);
    assert_eq!(d, 0, "disolver debe ser bit-exacto vs CPU, diff {d}");
}
