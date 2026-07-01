//! Geometría **pura** del cubo de Win+Tab (sin GPU — testeable a mano).
//!
//! Los dos escritorios son dos caras adyacentes de un cubo que gira sobre el eje
//! vertical (estilo Compiz). La cámara mira el centro del cubo desde el frente;
//! con perspectiva de pinhole, cada cara se proyecta a un **trapecio vertical**
//! (aristas izquierda/derecha verticales, alturas distintas por el escorzo).
//!
//! La transición va de `phi=0` (la cara **actual** llena la pantalla) a
//! `phi=π/2` (la cara **destino** la llena). `dir = +1` trae el vecino derecho
//! (la cara nueva entra por la derecha, la vieja sale por la izquierda); `-1` es
//! el espejo.
//!
//! El backend DRM compone el cubo dibujando cada cara en **N franjas verticales**
//! con `GlesFrame::render_texture` (una matriz afín por franja): las aristas de
//! cada franja caen EXACTAS sobre el trapecio real (proyección de una recta 3D es
//! una recta 2D), y sólo el muestreo de textura dentro de la franja es afín en
//! vez de perspectivo-correcto — imperceptible con N alto. Acá vive sólo la
//! matemática: dado `(cara, franja, phi, dir, w, h)` devuelve la matriz afín de
//! destino (cuad unidad → píxeles) y la de textura (cuad unidad → uv de la cara).

use core::f32::consts::FRAC_PI_2;

/// Distancia de la cámara al centro del cubo (en unidades donde el semi-lado del
/// cubo = 1). La focal se fija en `CAM_DIST - 1` para que la cara frontal llene
/// exactamente la pantalla a `phi=0`. Con `3.0`, la cara del fondo (`Z=-1`) queda
/// a la mitad de tamaño → un escorzo marcado pero no caricaturesco.
pub const CAM_DIST: f32 = 3.0;

/// Cuánto se **aleja la cámara** (zoom-out) a mitad de giro. Sin esto, la arista
/// cercana de una cara que rota queda MÁS cerca que el plano frontal y proyecta
/// más grande que la pantalla (se saldría por los bordes). Alejar a mitad de
/// giro encoge el cubo para que entre entero y flote sobre el fondo —el look
/// clásico del cubo de escritorio—. `0` a `phi=0`/`π/2` (la cara llena la
/// pantalla), máximo a `phi=π/4`.
pub const ZOOM_OUT: f32 = 0.42;

/// Franjas verticales por cara. La silueta es exacta a cualquier `N` (las
/// aristas son rectas), pero el muestreo de textura es afín por franja: con `96`
/// el escalón residual del borde inferior queda sub-píxel (verificado headless,
/// `cube_png`). 96×2 caras = 192 `render_texture` por cuadro — trivial para una
/// transición de ~13 cuadros.
pub const STRIPS: usize = 96;

/// El factor de zoom (`≤ 1`) para el ángulo `phi`: `1` en los extremos (cara
/// plena), `1 - ZOOM_OUT` a mitad de giro. Encoge la proyección hacia el centro.
pub fn zoom(phi: f32) -> f32 {
    1.0 - ZOOM_OUT * (2.0 * phi).sin().max(0.0)
}

/// Cuál de las dos caras visibles durante el giro.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    /// El escritorio del que salís (frontal a `phi=0`).
    Current,
    /// El escritorio al que entrás (frontal a `phi=π/2`).
    Next,
}

/// El ángulo de giro `phi ∈ [0, π/2]` para un progreso `t ∈ [0,1]` (lineal; el
/// llamante puede pasar `t` ya suavizado).
pub fn angle(t: f32) -> f32 {
    t.clamp(0.0, 1.0) * FRAC_PI_2
}

/// Coordenadas locales `(X0, Z0)` del cubo para el parámetro horizontal `u` de
/// una cara (antes de rotar). La cara actual es el plano frontal `Z0=1`,
/// `X0=2u-1`; la cara destino es la lateral que comparte la arista derecha/izq.
fn face_local(face: Face, u: f32, dir: f32) -> (f32, f32) {
    match face {
        Face::Current => (2.0 * u - 1.0, 1.0),
        // Lateral: `X0 = ±1` constante; `Z0` va de la arista compartida (`u=0`,
        // `Z0=1`) al fondo (`u=1`, `Z0=-1`).
        Face::Next => (dir, 1.0 - 2.0 * u),
    }
}

/// Proyecta el punto `(u,v) ∈ [0,1]²` de una cara a píxel `(px, py)` en una
/// pantalla `w×h`, con el cubo girado `phi` y dirección `dir` (`+1` derecha).
/// `(0,0)` es la esquina superior-izquierda de la cara; `v` crece hacia abajo.
pub fn project(face: Face, u: f32, v: f32, phi: f32, dir: f32, w: f32, h: f32) -> (f32, f32) {
    let (x0, z0) = face_local(face, u, dir);
    let yl = 2.0 * v - 1.0;
    let theta = dir * phi;
    let (s, c) = theta.sin_cos();
    // Rotación en el plano x-z (x derecha, z hacia la cámara).
    let x = x0 * c - z0 * s;
    let z = x0 * s + z0 * c;
    let scale = (CAM_DIST - 1.0) / (CAM_DIST - z) * zoom(phi);
    let px = w * 0.5 * (1.0 + x * scale);
    let py = h * 0.5 * (1.0 + yl * scale);
    (px, py)
}

/// La profundidad `Z` del centro de una cara — para ordenar el pintado
/// (painter's: la más lejana primero, así la frontal la tapa donde se solapan).
pub fn center_depth(face: Face, phi: f32, dir: f32) -> f32 {
    let (x0, z0) = face_local(face, 0.5, dir);
    let theta = dir * phi;
    let (s, c) = theta.sin_cos();
    x0 * s + z0 * c
}

/// Una transformación afín `(u,v) → (x,y)`: `x = a·u + b·v + c`, `y = d·u + e·v + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    /// Aplica la afín a un punto `(u,v)`.
    pub fn apply(&self, u: f32, v: f32) -> (f32, f32) {
        (self.a * u + self.b * v + self.c, self.d * u + self.e * v + self.f)
    }

    /// Las 9 componentes **column-major** de la `Matrix3` (cgmath/GL) equivalente
    /// — `gl_Position.xy = M · (u, v, 1)`. Filas `[a b c; d e f; 0 0 1]`.
    pub fn cols(&self) -> [f32; 9] {
        [self.a, self.d, 0.0, self.b, self.e, 0.0, self.c, self.f, 1.0]
    }
}

/// La matriz afín de **destino** (cuad unidad → píxeles) de la franja `i` de `n`
/// de una cara: ancla 3 esquinas exactas del trapecio (sup-izq, sup-der,
/// inf-izq). Las aristas verticales y el borde superior quedan EXACTOS sobre el
/// trapecio real; la esquina inf-der difiere en `~h·Δescala` (→0 con `n` alto).
pub fn strip_dst(face: Face, i: usize, n: usize, phi: f32, dir: f32, w: f32, h: f32) -> Affine {
    let n = n.max(1);
    let u0 = i as f32 / n as f32;
    let u1 = (i + 1) as f32 / n as f32;
    let (x00, y00) = project(face, u0, 0.0, phi, dir, w, h);
    let (x10, y10) = project(face, u1, 0.0, phi, dir, w, h);
    let (x01, y01) = project(face, u0, 1.0, phi, dir, w, h);
    Affine {
        a: x10 - x00,
        b: x01 - x00,
        c: x00,
        d: y10 - y00,
        e: y01 - y00,
        f: y00,
    }
}

/// La matriz afín de **textura** (cuad unidad → uv normalizado de la cara) de la
/// franja `i` de `n`: la franja muestrea la columna `[i/n, (i+1)/n]` de la
/// textura del escritorio, con `v` directo.
pub fn strip_tex(i: usize, n: usize) -> Affine {
    let n = n.max(1);
    let u0 = i as f32 / n as f32;
    let u1 = (i + 1) as f32 / n as f32;
    Affine { a: u1 - u0, b: 0.0, c: u0, d: 0.0, e: 1.0, f: 0.0 }
}

/// Como [`strip_tex`] pero **espejada en horizontal** (`u → 1-u`): la franja
/// geométrica `[i/n, (i+1)/n]` muestrea la columna reflejada `[1-(i+1)/n,
/// 1-i/n]`, de derecha a izquierda dentro de la franja. La usa la cara `Next`
/// cuando `dir < 0`: ahí su arista compartida (geometría `u=0`) es la DERECHA de
/// la textura, no la izquierda, así que sin espejar el escritorio entrante se ve
/// invertido horizontalmente.
pub fn strip_tex_flipped(i: usize, n: usize) -> Affine {
    let n = n.max(1);
    let u0 = i as f32 / n as f32;
    let u1 = (i + 1) as f32 / n as f32;
    // quad-u=0 → 1-u0 (arista izq. de la franja muestrea la textura reflejada);
    // quad-u=1 → 1-u1. Pendiente negativa = reflejo.
    Affine { a: u0 - u1, b: 0.0, c: 1.0 - u0, d: 0.0, e: 1.0, f: 0.0 }
}

/// Dibuja el cubo en el `frame` (un offscreen ya limpiado al color de fondo):
/// las dos caras, cada una en `strips` franjas verticales, ordenadas painter
/// (la más lejana primero). `tex_current`/`tex_next` son los dos escritorios
/// como texturas. `(w,h)` el tamaño del target, `phi` el ángulo, `dir` ±1.
///
/// Usa `GlesFrame::render_texture` con una matriz afín por franja (la matemática
/// pura de arriba): es 2D-afín, pero las franjas siguen el trapecio real, así
/// que el resultado es un cubo en perspectiva sin GL crudo.
pub fn draw_cube(
    frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>,
    w: i32,
    h: i32,
    tex_current: &smithay::backend::renderer::gles::GlesTexture,
    tex_next: &smithay::backend::renderer::gles::GlesTexture,
    phi: f32,
    dir: f32,
    strips: usize,
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    use cgmath::Matrix3;

    fn mat3(af: &Affine) -> Matrix3<f32> {
        let c = af.cols();
        Matrix3::new(c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7], c[8])
    }

    let (wf, hf) = (w as f32, h as f32);
    // Painter: la cara con menor Z (más lejos) se pinta primero; la frontal la
    // tapa donde se solapan (al inicio sólo la actual, al final sólo la destino).
    let mut faces = [
        (Face::Current, tex_current, center_depth(Face::Current, phi, dir)),
        (Face::Next, tex_next, center_depth(Face::Next, phi, dir)),
    ];
    if faces[0].2 > faces[1].2 {
        faces.swap(0, 1);
    }
    for (face, tex, _) in faces {
        // La cara destino comparte una arista con la actual; con dir<0 esa arista
        // es la DERECHA de la destino, así que su textura va espejada respecto del
        // muestreo por-u (que asume u=0 = izquierda). Sin esto el escritorio que
        // entra por la izquierda se ve invertido horizontalmente.
        let flip = face == Face::Next && dir < 0.0;
        for i in 0..strips.max(1) {
            let dst = mat3(&strip_dst(face, i, strips, phi, dir, wf, hf));
            let texm = mat3(&if flip {
                strip_tex_flipped(i, strips)
            } else {
                strip_tex(i, strips)
            });
            frame.render_texture(tex, texm, dst, None::<[f32; 0]>, 1.0, None, &[])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 1920.0;
    const H: f32 = 1080.0;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn a_phi_cero_la_cara_actual_llena_la_pantalla() {
        // Esquinas (0,0)→(0,0) y (1,1)→(W,H); la cara cubre exacto el viewport.
        let (x, y) = project(Face::Current, 0.0, 0.0, 0.0, 1.0, W, H);
        assert!(close(x, 0.0) && close(y, 0.0), "sup-izq = ({x},{y})");
        let (x, y) = project(Face::Current, 1.0, 1.0, 0.0, 1.0, W, H);
        assert!(close(x, W) && close(y, H), "inf-der = ({x},{y})");
        let (x, y) = project(Face::Current, 0.5, 0.5, 0.0, 1.0, W, H);
        assert!(close(x, W / 2.0) && close(y, H / 2.0), "centro = ({x},{y})");
    }

    #[test]
    fn a_phi_noventa_la_cara_destino_llena_la_pantalla() {
        let phi = FRAC_PI_2;
        let (x, y) = project(Face::Next, 0.0, 0.0, phi, 1.0, W, H);
        // u=0 (arista compartida) cae a la izquierda a phi=90.
        assert!(close(x, 0.0) && close(y, 0.0), "sup-izq destino = ({x},{y})");
        let (x, y) = project(Face::Next, 1.0, 1.0, phi, 1.0, W, H);
        assert!(close(x, W) && close(y, H), "inf-der destino = ({x},{y})");
    }

    #[test]
    fn el_escorzo_achica_el_lado_lejano() {
        // A mitad de giro, la cara actual rota: su arista que se aleja (mayor Z
        // negativo) proyecta MÁS corta que la cercana.
        let phi = FRAC_PI_2 / 2.0; // 45°
        let alto_borde = |u: f32| {
            let (_, yt) = project(Face::Current, u, 0.0, phi, 1.0, W, H);
            let (_, yb) = project(Face::Current, u, 1.0, phi, 1.0, W, H);
            yb - yt
        };
        // u=0 es la arista que se aleja (sale por la izquierda) → más corta que u=1.
        assert!(alto_borde(0.0) < alto_borde(1.0), "el lado que se aleja debe escorzar");
        // Ambas alturas dentro del viewport.
        assert!(alto_borde(0.0) > 0.0 && alto_borde(1.0) <= H + 0.01);
    }

    #[test]
    fn painter_ordena_actual_arriba_al_inicio_y_destino_al_final() {
        // phi=0: la actual está al frente (Z mayor) que la destino.
        assert!(center_depth(Face::Current, 0.0, 1.0) > center_depth(Face::Next, 0.0, 1.0));
        // phi=90: se invierte.
        assert!(
            center_depth(Face::Next, FRAC_PI_2, 1.0) > center_depth(Face::Current, FRAC_PI_2, 1.0)
        );
    }

    #[test]
    fn strip_tex_flipped_es_el_reflejo_horizontal_del_normal() {
        let n = 4;
        // Franja 0 (izquierda geométrica) muestrea la DERECHA de la textura.
        let f0 = strip_tex_flipped(0, n);
        assert!(close(f0.apply(0.0, 0.0).0, 1.0), "franja izq quad-u=0 → textura derecha");
        assert!(close(f0.apply(1.0, 0.0).0, 0.75), "→ 0.75");
        // Franja n-1 (derecha geométrica) muestrea la IZQUIERDA de la textura.
        let fl = strip_tex_flipped(n - 1, n);
        assert!(close(fl.apply(1.0, 0.0).0, 0.0), "franja der quad-u=1 → textura izquierda");
        // `v` intacto (el espejo es sólo horizontal).
        assert!(close(f0.apply(0.3, 0.7).1, 0.7));
        // Reflejo exacto del normal: u_flip == 1 - u_normal en cualquier punto.
        let normal = strip_tex(1, n);
        let flip = strip_tex_flipped(1, n);
        assert!(close(flip.apply(0.5, 0.0).0, 1.0 - normal.apply(0.5, 0.0).0), "reflejo exacto");
    }

    #[test]
    fn las_franjas_cubren_la_cara_sin_huecos_en_las_aristas() {
        // La arista derecha de una franja coincide con la izquierda de la siguiente
        // (esquinas exactas compartidas) → sin huecos verticales.
        let n = 24;
        let phi = 0.7;
        for i in 0..n - 1 {
            let s0 = strip_dst(Face::Current, i, n, phi, 1.0, W, H);
            let s1 = strip_dst(Face::Current, i + 1, n, phi, 1.0, W, H);
            // (1,0) de la franja i == (0,0) de la i+1 (borde superior compartido).
            let (x0, y0) = s0.apply(1.0, 0.0);
            let (x1, y1) = s1.apply(0.0, 0.0);
            assert!(close(x0, x1) && close(y0, y1), "hueco superior en franja {i}");
            // (1,1) de i contra (0,1) de i+1: comparten x (arista vertical).
            let (xb0, _) = s0.apply(1.0, 1.0);
            let (xb1, _) = s1.apply(0.0, 1.0);
            assert!(close(xb0, xb1), "x de arista inferior no coincide en franja {i}");
        }
    }

    /// Evidencia headless: compone el cubo con dos texturas-escritorio sintéticas
    /// a varios ángulos y escribe PNGs (a `$CUBE_OUT` o el dir actual). Ignorado
    /// por defecto (pide render node / GPU). Correr:
    /// `cargo test -p mirada-compositor cube_png -- --ignored --nocapture`.
    #[test]
    #[ignore = "headless GPU demo — escribe PNGs del cubo"]
    fn cube_png() {
        use smithay::backend::allocator::gbm::GbmDevice;
        use smithay::backend::allocator::Fourcc;
        use smithay::backend::egl::{EGLContext, EGLDisplay};
        use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
        use smithay::backend::renderer::{Bind, Color32F, ExportMem, Frame as _, ImportMem, Offscreen, Renderer};
        use smithay::utils::{Buffer as Buf, Physical, Rectangle, Size, Transform};

        const FOURCC: Fourcc = Fourcc::Abgr8888;
        // Una textura-escritorio sintética: fondo + grilla + marco grueso +
        // esquinas de colores fijos (TL blanco, TR amarillo, BL cian, BR magenta)
        // → permite leer orientación (¿hay flip?) y perspectiva en el PNG.
        fn face(tw: i32, th: i32, base: [u8; 3], border: [u8; 3]) -> Vec<u8> {
            let (tw, th) = (tw as usize, th as usize);
            let mut px = vec![0u8; tw * th * 4];
            let set = |px: &mut [u8], x: usize, y: usize, c: [u8; 3]| {
                let i = (y * tw + x) * 4;
                px[i] = c[0];
                px[i + 1] = c[1];
                px[i + 2] = c[2];
                px[i + 3] = 255;
            };
            for y in 0..th {
                for x in 0..tw {
                    let grid = x % 48 == 0 || y % 48 == 0;
                    let c = if grid { [base[0] + 30, base[1] + 30, base[2] + 30] } else { base };
                    set(&mut px, x, y, c);
                }
            }
            let bw = 10usize;
            for y in 0..th {
                for x in 0..tw {
                    if x < bw || y < bw || x >= tw - bw || y >= th - bw {
                        set(&mut px, x, y, border);
                    }
                }
            }
            let q = 48usize;
            let corners = [
                (0usize, 0usize, [255, 255, 255]),     // TL blanco
                (tw - q, 0, [255, 255, 0]),            // TR amarillo
                (0, th - q, [0, 255, 255]),            // BL cian
                (tw - q, th - q, [255, 0, 255]),       // BR magenta
            ];
            for (cx, cy, c) in corners {
                for y in cy..(cy + q).min(th) {
                    for x in cx..(cx + q).min(tw) {
                        set(&mut px, x, y, c);
                    }
                }
            }
            px
        }

        let node = std::env::var("MIRADA_RENDER_NODE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
        let file = std::fs::OpenOptions::new().read(true).write(true).open(&node).expect("render node");
        let gbm = GbmDevice::new(file).expect("gbm");
        let disp = unsafe { EGLDisplay::new(gbm) }.expect("egl display");
        let ctx = EGLContext::new(&disp).expect("egl ctx");
        let mut r = unsafe { GlesRenderer::new(ctx) }.expect("gles");

        let (fw, fh) = (480, 270);
        let ta: GlesTexture =
            r.import_memory(&face(fw, fh, [70, 20, 20], [0, 230, 0]), FOURCC, Size::<i32, Buf>::from((fw, fh)), false).unwrap();
        let tb: GlesTexture =
            r.import_memory(&face(fw, fh, [20, 20, 80], [255, 140, 0]), FOURCC, Size::<i32, Buf>::from((fw, fh)), false).unwrap();

        let (w, h) = (960i32, 540i32);
        let out = std::env::var("CUBE_OUT").unwrap_or_else(|_| ".".into());
        for (tag, t) in [("00", 0.0f32), ("30", 0.30), ("50", 0.50), ("70", 0.70), ("100", 1.0)] {
            let phi = angle(t);
            let mut off = Offscreen::<GlesTexture>::create_buffer(&mut r, FOURCC, Size::<i32, Buf>::from((w, h))).unwrap();
            let mut target = r.bind(&mut off).unwrap();
            let fis = Size::<i32, Physical>::from((w, h));
            {
                let mut frame = r.render(&mut target, fis, Transform::Normal).unwrap();
                frame.clear(Color32F::from([0.05, 0.05, 0.07, 1.0]), &[Rectangle::from_size(fis)]).unwrap();
                draw_cube(&mut frame, w, h, &ta, &tb, phi, 1.0, STRIPS).unwrap();
                let _ = frame.finish().unwrap();
            }
            let rect = Rectangle::<i32, Buf>::from_size((w, h).into());
            let map = r.copy_framebuffer(&target, rect, FOURCC).unwrap();
            let bytes = r.map_texture(&map).unwrap();
            let img = image::RgbaImage::from_raw(w as u32, h as u32, bytes[..(w * h * 4) as usize].to_vec()).unwrap();
            let path = format!("{out}/cube_t{tag}.png");
            img.save(&path).unwrap();
            println!("cube_png · escrito {path}");
        }
    }

    #[test]
    fn strip_tex_particiona_la_textura() {
        let n = 4;
        assert_eq!(strip_tex(0, n).apply(0.0, 0.0).0, 0.0);
        assert_eq!(strip_tex(3, n).apply(1.0, 0.0).0, 1.0);
        // continuidad: fin de una franja = inicio de la siguiente.
        assert!(close(strip_tex(1, n).apply(1.0, 0.0).0, strip_tex(2, n).apply(0.0, 0.0).0));
    }

    #[test]
    fn dir_negativo_es_espejo_horizontal() {
        // El destino con dir=-1 a phi=90 también llena la pantalla, pero la arista
        // compartida (u=0) cae a la DERECHA (espejo del caso dir=+1).
        let (x, _) = project(Face::Next, 0.0, 0.0, FRAC_PI_2, -1.0, W, H);
        assert!(close(x, W), "arista compartida a la derecha con dir=-1, x={x}");
    }
}
