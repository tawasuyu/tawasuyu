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
