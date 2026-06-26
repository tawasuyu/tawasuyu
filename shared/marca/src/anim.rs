//! El **wallpaper de marca animado** — render procedural por frame.
//!
//! Es la versión viva del fondo por defecto (`docs/brand/wallpaper.svg`): la
//! **chakana** (cruz andina escalonada, geometría EXACTA del SVG) en el cruce de
//! un **plano cartesiano** cuyos cuatro cuadrantes son las cuatro fases del
//! ciclo de la información. Lo animado:
//!
//! - un **fluido** que viaja de adentro hacia afuera por cada eje y, al llegar a
//!   la punta, **enciende la flecha** del extremo;
//! - la **iluminación de la chakana** variando levemente (respira).
//!
//! `animated_frame(t, w, h)` es **puro**: devuelve un buffer BGRA opaco
//! (`[B,G,R,255]` por píxel, listo para `Argb8888` little-endian) del tamaño
//! pedido, para el instante `t` en segundos. El compositor lo regenera
//! estrangulado (~20 fps) y lo pinta como fondo. Sin dependencias: sólo `std` y
//! aritmética.

/// Cuerpo de la chakana en unidades del SVG (viewBox -3..3), centro en `(0,0)`,
/// medio-extensión 2.5. Polígono cerrado — vértices tal cual `docs/brand/chakana.svg`.
const CHAKANA: &[(f32, f32)] = &[
    (-0.5, -2.5), (0.5, -2.5), (0.5, -2.0), (1.0, -2.0), (1.0, -1.5),
    (1.5, -1.5), (1.5, -1.0), (2.0, -1.0), (2.0, -0.5), (2.5, -0.5),
    (2.5, 0.5), (2.0, 0.5), (2.0, 1.0), (1.5, 1.0), (1.5, 1.5), (1.0, 1.5),
    (1.0, 2.0), (0.5, 2.0), (0.5, 2.5), (-0.5, 2.5), (-0.5, 2.0), (-1.0, 2.0),
    (-1.0, 1.5), (-1.5, 1.5), (-1.5, 1.0), (-2.0, 1.0), (-2.0, 0.5), (-2.5, 0.5),
    (-2.5, -0.5), (-2.0, -0.5), (-2.0, -1.0), (-1.5, -1.0), (-1.5, -1.5),
    (-1.0, -1.5), (-1.0, -2.0), (-0.5, -2.0),
];

// Paleta de marca (de `llimphi-theme::dark`, vía el SVG).
const BG: [f32; 3] = [14.0, 16.0, 22.0]; // #0E1016
const ACCENT: [f32; 3] = [110.0, 140.0, 220.0]; // #6E8CDC
const CORE: [f32; 3] = [201.0, 214.0, 242.0]; // #C9D6F2
const BODY: [f32; 3] = [25.0, 30.0, 46.0]; // ≈ #1B2030 (relleno chakana)
// Cuadrantes (fase → color de tema). El plano: x<cx izquierda, y<cy arriba.
const Q_PERCIBIR: [f32; 3] = [185.0, 201.0, 232.0]; // arriba-izq  #B9C9E8
const Q_CONOCER: [f32; 3] = [232.0, 201.0, 122.0]; // arriba-der  #E8C97A
const Q_RAIZ: [f32; 3] = [143.0, 181.0, 140.0]; // abajo-izq   #8FB58C
const Q_HACER: [f32; 3] = [232.0, 155.0, 110.0]; // abajo-der   #E89B6E

/// Período maestro del ciclo, en segundos. **Todo** cierra exactamente acá:
/// `animated_frame(t) == animated_frame(t + LOOP_SECS)` byte a byte (probado).
/// El fluido hace 2 viajes por loop; la respiración, 1. Lento y calmo.
pub const LOOP_SECS: f32 = 24.0;

/// Cuánto del semieje ocupa cada brazo del plano (deja `1-REACH` de margen a cada
/// borde, así la cruz **no toca** los bordes y no se corta bajo barra/overscan).
const REACH: f32 = 0.80;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `true` si `(x,y)` cae dentro del polígono (ray casting).
fn in_poly(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// El frame del wallpaper de marca animado en `t` segundos, BGRA opaco `w×h`.
pub fn animated_frame(t: f32, w: u32, h: u32) -> Vec<u8> {
    let w = w.max(1) as usize;
    let h = h.max(1) as usize;
    let mut px = vec![0u8; w * h * 4];
    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let maxr = (cx * cx + cy * cy).sqrt().max(1.0);

    // ── Fondo: base plana + tinte de cuadrante (hacia las esquinas) + halo
    //    central accent. Un solo sqrt por píxel. ───────────────────────────────
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt() / maxr; // 0 centro .. 1 esquina
            let q = if dx < 0.0 {
                if dy < 0.0 { Q_PERCIBIR } else { Q_RAIZ }
            } else if dy < 0.0 {
                Q_CONOCER
            } else {
                Q_HACER
            };
            let qf = 0.07 * smoothstep(0.15, 1.0, d); // tinte sutil hacia afuera
            let gf = 0.10 * (1.0 - d).clamp(0.0, 1.0).powf(1.6); // halo al centro
            let r = BG[0] + q[0] * qf + ACCENT[0] * gf;
            let g = BG[1] + q[1] * qf + ACCENT[1] * gf;
            let b = BG[2] + q[2] * qf + ACCENT[2] * gf;
            let i = (y * w + x) * 4;
            px[i] = b.min(255.0) as u8;
            px[i + 1] = g.min(255.0) as u8;
            px[i + 2] = r.min(255.0) as u8;
            px[i + 3] = 255;
        }
    }

    // ── Ejes + fluido + flechas ──────────────────────────────────────────────
    // Brazo: del centro a ~92% del semieje. El fluido es una gaussiana que viaja
    // de 0 a 1 (centro→punta) y se repite; dos pulsos desfasados dan continuidad.
    let arm_x = cx * REACH;
    let arm_y = cy * REACH;
    // Fase del loop maestro (0..1). El fluido recorre el brazo 2 veces por loop.
    let lt = (t / LOOP_SECS).fract();
    let phase = (lt * 2.0).fract();
    // Posición(es) del pulso a lo largo del brazo (0..1).
    let pulses = [phase, (phase + 0.5).fract()];
    // Intensidad del fluido en distancia normalizada `u` (0 centro, 1 punta). La
    // distancia es **circular** (`d - d.round()`): la onda que llega a la punta
    // reemerge en el centro en el MISMO instante, sin teletransporte → el barrido
    // no salta al rebobinar el pulso.
    let fluid = |u: f32| -> f32 {
        let mut s = 0.0f32;
        for &p in &pulses {
            let mut d = u - p;
            d -= d.round();
            let dd = d / 0.10;
            s += (-(dd * dd)).exp();
        }
        s.min(1.0)
    };
    // Brillo de cada flecha: pico cuando un pulso pasa por la punta (u≈1), también
    // con distancia **circular** → el encendido sube y baja suave y CIERRA sin
    // salto (era esto lo que parpadeaba en el color de la punta).
    let arrow_glow = {
        let mut s = 0.0f32;
        for &p in &pulses {
            let mut d = 1.0 - p;
            d -= d.round();
            let dd = d / 0.14;
            s += (-(dd * dd)).exp();
        }
        s.min(1.0)
    };
    let half = 1.5f32; // medio-grosor del eje (px)
    let add = |px: &mut [u8], x: i32, y: i32, c: [f32; 3], a: f32| {
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h || a <= 0.0 {
            return;
        }
        let i = (y as usize * w + x as usize) * 4;
        px[i] = (px[i] as f32 + c[2] * a).min(255.0) as u8;
        px[i + 1] = (px[i + 1] as f32 + c[1] * a).min(255.0) as u8;
        px[i + 2] = (px[i + 2] as f32 + c[0] * a).min(255.0) as u8;
    };
    // Eje horizontal.
    for x in 0..w {
        let u = (x as f32 - cx).abs() / arm_x;
        if u > 1.0 {
            continue;
        }
        let bright = 0.07 + 0.45 * fluid(u); // línea tenue + fluido
        for dy in -(half as i32)..=(half as i32) {
            add(&mut px, x as i32, cy as i32 + dy, ACCENT, bright);
        }
    }
    // Eje vertical.
    for y in 0..h {
        let u = (y as f32 - cy).abs() / arm_y;
        if u > 1.0 {
            continue;
        }
        let bright = 0.07 + 0.45 * fluid(u);
        for dx in -(half as i32)..=(half as i32) {
            add(&mut px, cx as i32 + dx, y as i32, ACCENT, bright);
        }
    }
    // Flechas en los cuatro extremos (triángulos), encendidas por el fluido.
    let ah = (arm_x.min(arm_y) * 0.035).max(8.0); // tamaño de la punta
    let arrow_a = 0.22 + 0.45 * arrow_glow;
    draw_arrow(&mut px, w, h, cx + arm_x, cy, 1.0, 0.0, ah, ACCENT, arrow_a);
    draw_arrow(&mut px, w, h, cx - arm_x, cy, -1.0, 0.0, ah, ACCENT, arrow_a);
    draw_arrow(&mut px, w, h, cx, cy + arm_y, 0.0, 1.0, ah, ACCENT, arrow_a);
    draw_arrow(&mut px, w, h, cx, cy - arm_y, 0.0, -1.0, ah, ACCENT, arrow_a);

    // ── Chakana centrada, iluminación variando levemente ─────────────────────
    // Respiración atada al loop maestro: 1 ciclo por `LOOP_SECS`, continua en el
    // cierre (`breath(0)=breath(1)=0`). Amplitud baja → sutil.
    let breath = 0.5 - 0.5 * (lt * core::f32::consts::TAU).cos();
    let lum = 0.90 + 0.10 * breath;
    let scale = (w.min(h) as f32) * 0.11 / 2.5; // medio-extensión ≈ 11% del menor
    let ext = (2.5 * scale).ceil() as i32 + 2;
    let cxi = cx as i32;
    let cyi = cy as i32;
    for y in (cyi - ext).max(0)..(cyi + ext).min(h as i32) {
        for x in (cxi - ext).max(0)..(cxi + ext).min(w as i32) {
            let ux = (x as f32 - cx) / scale;
            let uy = (y as f32 - cy) / scale;
            let r = (ux * ux + uy * uy).sqrt();
            let i = (y as usize * w + x as usize) * 4;
            if r <= 0.5 {
                // Núcleo luminoso (sobre-escribe).
                px[i] = (CORE[2] * lum).min(255.0) as u8;
                px[i + 1] = (CORE[1] * lum).min(255.0) as u8;
                px[i + 2] = (CORE[0] * lum).min(255.0) as u8;
            } else if in_poly(CHAKANA, ux, uy) {
                // El cuerpo es oscuro (≈ fondo, como el SVG); lo que DIBUJA la cruz
                // escalonada es el **trazo** accent en el borde. Los lados de la
                // chakana son todos axis-aligned, así que 4 vecinos detectan el
                // borde exacto. El trazo respira con `lum`.
                let swu = (2.6 / scale).max(0.04); // grosor del trazo en unidades
                let edge = !in_poly(CHAKANA, ux + swu, uy)
                    || !in_poly(CHAKANA, ux - swu, uy)
                    || !in_poly(CHAKANA, ux, uy + swu)
                    || !in_poly(CHAKANA, ux, uy - swu);
                if edge {
                    let s = 0.66 + 0.24 * lum;
                    px[i] = (ACCENT[2] * s).min(255.0) as u8;
                    px[i + 1] = (ACCENT[1] * s).min(255.0) as u8;
                    px[i + 2] = (ACCENT[0] * s).min(255.0) as u8;
                } else {
                    let halo = 0.5 * (1.0 - smoothstep(0.5, 1.2, r)); // glow del núcleo
                    px[i] = (BODY[2] * lum + ACCENT[2] * halo).min(255.0) as u8;
                    px[i + 1] = (BODY[1] * lum + ACCENT[1] * halo).min(255.0) as u8;
                    px[i + 2] = (BODY[0] * lum + ACCENT[0] * halo).min(255.0) as u8;
                }
            } else {
                // Fuera del cuerpo: halo difuso muy sutil alrededor de la chakana.
                let halo = 0.20 * (1.0 - smoothstep(2.5, 3.4, r)) * smoothstep(2.4, 2.6, r);
                if halo > 0.001 {
                    add(&mut px, x, y, ACCENT, halo);
                }
            }
        }
    }

    px
}

/// Dibuja una flecha (triángulo) con la punta en `(tx,ty)` apuntando en la
/// dirección `(dirx,diry)` (unitaria sobre un eje), tamaño `s`, color `c`, alfa
/// `a` (aditivo). Rasteriza por bbox + test de pertenencia.
#[allow(clippy::too_many_arguments)]
fn draw_arrow(
    px: &mut [u8],
    w: usize,
    h: usize,
    tx: f32,
    ty: f32,
    dirx: f32,
    diry: f32,
    s: f32,
    c: [f32; 3],
    a: f32,
) {
    // Triángulo: punta en (tx,ty) + dir*s*1.6; base a (tx,ty) ± perp*s.
    let tip = (tx + dirx * s * 1.7, ty + diry * s * 1.7);
    let perpx = -diry;
    let perpy = dirx;
    let b1 = (tx + perpx * s, ty + perpy * s);
    let b2 = (tx - perpx * s, ty - perpy * s);
    let tri = [tip, b1, b2];
    let minx = tip.0.min(b1.0).min(b2.0).floor().max(0.0) as usize;
    let maxx = (tip.0.max(b1.0).max(b2.0).ceil() as usize).min(w.saturating_sub(1));
    let miny = tip.1.min(b1.1).min(b2.1).floor().max(0.0) as usize;
    let maxy = (tip.1.max(b1.1).max(b2.1).ceil() as usize).min(h.saturating_sub(1));
    for y in miny..=maxy {
        for x in minx..=maxx {
            if in_poly(&tri, x as f32, y as f32) {
                let i = (y * w + x) * 4;
                px[i] = (px[i] as f32 + c[2] * a).min(255.0) as u8;
                px[i + 1] = (px[i + 1] as f32 + c[1] * a).min(255.0) as u8;
                px[i + 2] = (px[i + 2] as f32 + c[0] * a).min(255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_tiene_tamano_y_es_opaco() {
        let f = animated_frame(1.0, 64, 48);
        assert_eq!(f.len(), 64 * 48 * 4);
        // Todos los alfas en 255 (fondo opaco).
        assert!(f.chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn el_centro_es_el_nucleo_luminoso() {
        let w = 200u32;
        let h = 200u32;
        let f = animated_frame(0.0, w, h);
        let cx = (w / 2) as usize;
        let cy = (h / 2) as usize;
        let i = (cy * w as usize + cx) * 4;
        // El núcleo (CORE ≈ 201,214,242) es claro: R alto.
        assert!(f[i + 2] > 150, "el centro debería ser el núcleo claro");
    }

    #[test]
    fn el_frame_cambia_con_el_tiempo() {
        // El fluido se mueve: dos instantes distintos difieren en algún píxel.
        let a = animated_frame(0.0, 160, 120);
        let b = animated_frame(1.3, 160, 120);
        assert!(a != b, "la animación debe variar con t");
    }

    #[test]
    fn cierra_el_ciclo_byte_a_byte() {
        // El reclamo del usuario: «debe cerrar donde empezó». Con todo atado al
        // período maestro, el frame en `t` y en `t + LOOP_SECS` es IDÉNTICO.
        let (w, h) = (200u32, 120u32);
        assert_eq!(
            animated_frame(0.0, w, h),
            animated_frame(LOOP_SECS, w, h),
            "el frame en t=0 y t=LOOP debe ser idéntico (loop sin costura)"
        );
        // Y en un instante arbitrario del medio, también.
        assert_eq!(
            animated_frame(2.5, w, h),
            animated_frame(2.5 + LOOP_SECS, w, h),
            "el frame debe repetirse cada LOOP_SECS en cualquier fase"
        );
    }

    #[test]
    fn las_puntas_no_parpadean() {
        // El reclamo: «salta el color de las flechas». Barremos t por TODO el loop
        // y miramos el R de la base de la punta derecha: con la distancia circular
        // el encendido es suave — sin saltos bruscos frame a frame.
        let (w, h) = (320u32, 180u32);
        let cx = (w / 2) as f32;
        let cy = (h / 2) as usize;
        let px_x = (cx + cx * REACH) as usize; // base de la flecha derecha
        let idx = (cy * w as usize + px_x) * 4 + 2; // canal R (accent aditivo)
        // Paso fino (dt=0.05 s): una pendiente suave se achica con el paso, pero
        // una discontinuidad (el teletransporte que parpadeaba, ~50 niveles) no.
        let steps = 480;
        let mut prev = animated_frame(0.0, w, h)[idx] as i32;
        let mut maxjump = 0i32;
        for k in 1..=steps {
            let t = LOOP_SECS * k as f32 / steps as f32;
            let cur = animated_frame(t, w, h)[idx] as i32;
            maxjump = maxjump.max((cur - prev).abs());
            prev = cur;
        }
        assert!(maxjump <= 8, "la punta parpadea: salto máximo de {maxjump} niveles");
    }

    #[test]
    fn deja_margen_sin_cortar_arriba_y_abajo() {
        // A resolución real (768p) con `REACH=0.80`, la cruz y las puntas de
        // flecha dejan margen: la banda superior e inferior (3%) del eje central
        // es fondo puro — sin el accent brillante del eje/flecha. Cota: cualquier
        // pixel pintado por eje/flecha supera con creces este umbral.
        let (w, h) = (1366u32, 768u32);
        let f = animated_frame(1.0, w, h);
        let cx = (w / 2) as usize;
        let band = (h as f32 * 0.03) as usize;
        const ACCENT_R: u8 = 45; // fondo en la banda << esto; eje/flecha >> esto
        for y in 0..band {
            let top = f[(y * w as usize + cx) * 4 + 2];
            let bot = f[(((h as usize - 1 - y) * w as usize) + cx) * 4 + 2];
            assert!(top < ACCENT_R, "fila {y} (arriba) cortada: R={top}");
            assert!(bot < ACCENT_R, "fila {} (abajo) cortada: R={bot}", h as usize - 1 - y);
        }
    }

    #[test]
    fn in_poly_reconoce_centro_y_esquina() {
        assert!(in_poly(CHAKANA, 0.0, 0.0), "el centro está dentro");
        assert!(!in_poly(CHAKANA, 2.9, 2.9), "la esquina lejana está fuera");
    }
}
