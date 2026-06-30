//! Geometría del **hero de lock** — la transición soñada: al bloquear, la
//! pantalla viva se *encoge* hasta el thumbnail de la sesión activa en el
//! greeter.
//!
//! El nudo de un hero (transición de elemento compartido) entre dos procesos
//! —el compositor que tiene la textura viva y el greeter que muestra las
//! miniaturas— es **coincidir en el rect destino**. Como llimphi no le devuelve
//! a la app el rect computado por taffy, replicarlo a mano sería frágil. La
//! solución: un **rect determinístico compartido**, función pura del tamaño de
//! la salida, que *ambos* lados calculan igual. El greeter ubica el thumbnail
//! activo en [`landing_rect`]; el compositor anima la captura congelada hasta el
//! mismo rect con [`hero_rect`]. Sin readback de taffy, sin round-trip.
//!
//! Todo entero/`f32` sobre `core` — sin `libm`, sin `std`: compila igual para el
//! compositor (Linux) que para cualquier consumidor `no_std`.

use crate::geometry::Rect;

/// Escala del thumbnail de aterrizaje, en % del lado de la salida. Uniforme en
/// ancho y alto (preserva el aspecto de la pantalla, así el encogido es una
/// reducción a escala — se ve natural).
const LANDING_SCALE_PCT: i32 = 34;
/// Borde superior del aterrizaje, en % del alto de la salida (zona superior-
/// centro, por encima del centro de la tarjeta de login).
const LANDING_TOP_PCT: i32 = 16;

/// Escala del **zoom-in** del hero de lock: la captura crece hasta este % del
/// lado de la salida (uniforme, centrada → el contenido se agranda y la salida
/// recorta los bordes). >100 = zoom-in (la pantalla viva se mete hacia el
/// usuario antes de revelar el greeter), en vez del encogido a thumbnail.
const ZOOM_IN_SCALE_PCT: i32 = 140;

/// El rect destino del hero: dónde aterriza el thumbnail de la sesión **activa**
/// en una salida `out_w × out_h`. Centrado en horizontal, en el tercio superior,
/// a una escala uniforme del tamaño de la salida (conserva su aspecto).
///
/// El greeter pinta ahí el thumbnail activo y el compositor encoge la captura
/// hasta ahí — por eso es la **única fuente** de esa posición.
pub fn landing_rect(out_w: i32, out_h: i32) -> Rect {
    let out_w = out_w.max(0);
    let out_h = out_h.max(0);
    let w = out_w * LANDING_SCALE_PCT / 100;
    let h = out_h * LANDING_SCALE_PCT / 100;
    let x = (out_w - w) / 2;
    let y = out_h * LANDING_TOP_PCT / 100;
    Rect::new(x, y, w, h)
}

/// El rect destino del hero en modo **zoom-in**: la captura crece a
/// [`ZOOM_IN_SCALE_PCT`] del tamaño de la salida, centrada (origen negativo: los
/// bordes salen del marco y la salida los recorta). Escala uniforme → preserva
/// el aspecto, así la `dst/full` que usa el render sigue siendo válida. A
/// diferencia de [`landing_rect`] (encoge a un thumbnail), esto **agranda**: la
/// pantalla viva hace zoom-in mientras el velo sube, y al terminar se revela el
/// greeter.
pub fn zoom_in_rect(out_w: i32, out_h: i32) -> Rect {
    let out_w = out_w.max(0);
    let out_h = out_h.max(0);
    let w = out_w * ZOOM_IN_SCALE_PCT / 100;
    let h = out_h * ZOOM_IN_SCALE_PCT / 100;
    let x = (out_w - w) / 2; // negativo: crece centrado, recortando bordes
    let y = (out_h - h) / 2;
    Rect::new(x, y, w, h)
}

/// Suaviza `t` con un *smoothstep* cúbico (`3t² − 2t³`): arranca y termina sin
/// tirón (derivada cero en 0 y 1). Clampa `t` a `[0, 1]`.
pub fn ease_in_out(t: f32) -> f32 {
    let t = if t < 0.0 {
        0.0
    } else if t > 1.0 {
        1.0
    } else {
        t
    };
    t * t * (3.0 - 2.0 * t)
}

/// Interpola un entero `a → b` por `t ∈ [0,1]` (redondeando al más cercano).
fn lerp_i32(a: i32, b: i32, t: f32) -> i32 {
    let d = (b - a) as f32;
    a + (d * t + if d >= 0.0 { 0.5 } else { -0.5 }) as i32
}

/// Interpola dos rectángulos esquina a esquina (lineal en `t`, sin easing).
pub fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    Rect::new(
        lerp_i32(from.x, to.x, t),
        lerp_i32(from.y, to.y, t),
        lerp_i32(from.w, to.w, t),
        lerp_i32(from.h, to.h, t),
    )
}

/// El rect de la captura en el instante `t ∈ [0,1]` del hero: de `full`
/// (pantalla completa, `t=0`) a `target` (el thumbnail, `t=1`), con el easing
/// suave aplicado. `target` = [`landing_rect`] de la misma salida.
pub fn hero_rect(full: Rect, target: Rect, t: f32) -> Rect {
    lerp_rect(full, target, ease_in_out(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landing_centrado_y_a_escala() {
        let r = landing_rect(1920, 1080);
        // 34% del tamaño, preservando aspecto (uniforme).
        assert_eq!((r.w, r.h), (652, 367));
        // Centrado en horizontal.
        assert_eq!(r.x, (1920 - 652) / 2);
        // En el tercio superior.
        assert_eq!(r.y, 1080 * 16 / 100);
        // Cabe dentro de la salida.
        assert!(r.x >= 0 && r.y >= 0 && r.x + r.w <= 1920 && r.y + r.h <= 1080);
    }

    #[test]
    fn landing_no_panica_en_degenerados() {
        assert_eq!(landing_rect(0, 0), Rect::new(0, 0, 0, 0));
        let _ = landing_rect(-10, -10); // clamp a 0, sin panic
    }

    #[test]
    fn zoom_in_crece_centrado_y_uniforme() {
        let r = zoom_in_rect(1920, 1080);
        // 140% del tamaño, uniforme (preserva aspecto).
        assert_eq!((r.w, r.h), (1920 * 140 / 100, 1080 * 140 / 100));
        // Centrado → origen negativo (los bordes salen del marco) y simétrico.
        assert_eq!(r.x, (1920 - r.w) / 2);
        assert_eq!(r.y, (1080 - r.h) / 2);
        assert!(r.x < 0 && r.y < 0, "zoom-in crece más allá de la salida");
        // Es un zoom-IN: más grande que la salida.
        assert!(r.w > 1920 && r.h > 1080);
    }

    #[test]
    fn zoom_in_no_panica_en_degenerados() {
        assert_eq!(zoom_in_rect(0, 0), Rect::new(0, 0, 0, 0));
        let _ = zoom_in_rect(-10, -10); // clamp a 0, sin panic
    }

    #[test]
    fn ease_extremos_y_medio() {
        assert_eq!(ease_in_out(0.0), 0.0);
        assert_eq!(ease_in_out(1.0), 1.0);
        assert_eq!(ease_in_out(0.5), 0.5); // simétrico
        // Clampa fuera de rango.
        assert_eq!(ease_in_out(-1.0), 0.0);
        assert_eq!(ease_in_out(2.0), 1.0);
    }

    #[test]
    fn ease_es_monotona() {
        let mut prev = ease_in_out(0.0);
        let mut t = 0.05;
        while t <= 1.0 {
            let cur = ease_in_out(t);
            assert!(cur >= prev, "no monótona en t={t}: {cur} < {prev}");
            prev = cur;
            t += 0.05;
        }
    }

    #[test]
    fn hero_extremos_son_full_y_target() {
        let full = Rect::new(0, 0, 1920, 1080);
        let target = landing_rect(1920, 1080);
        assert_eq!(hero_rect(full, target, 0.0), full);
        assert_eq!(hero_rect(full, target, 1.0), target);
    }

    #[test]
    fn hero_en_medio_esta_entre_los_dos() {
        let full = Rect::new(0, 0, 1920, 1080);
        let target = landing_rect(1920, 1080);
        let mid = hero_rect(full, target, 0.5);
        // El rect intermedio es más chico que full y más grande que target,
        // y su centro va viajando hacia el del target.
        assert!(mid.w < full.w && mid.w > target.w);
        assert!(mid.h < full.h && mid.h > target.h);
        assert!(mid.y > full.y && mid.y <= target.y + target.h);
    }

    #[test]
    fn lerp_rect_endpoints() {
        let a = Rect::new(0, 0, 100, 100);
        let b = Rect::new(40, 20, 20, 20);
        assert_eq!(lerp_rect(a, b, 0.0), a);
        assert_eq!(lerp_rect(a, b, 1.0), b);
    }
}
