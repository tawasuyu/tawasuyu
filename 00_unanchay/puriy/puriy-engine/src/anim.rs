//! Runtime de tween de animación CSS — Fase B4.
//!
//! Hasta acá `@keyframes` (Fase 7.31) y los shorthands `animation`/
//! `transition` (Fase 7.32) se parseaban a data inerte en
//! [`crate::style::ComputedStyle`]. Este módulo es el núcleo que consume
//! esa data: dado un [`AnimationBinding`] y el tiempo transcurrido desde
//! que la animación arrancó, computa el **progreso eased ∈ [0,1] dentro
//! del ciclo actual** — el punto del timeline al que hay que muestrear
//! los keyframes.
//!
//! Esta microfase (B4.1) es puro cálculo, sin tocar el box tree ni el
//! chrome. La interpolación de valores de keyframes (B4.2) y el cableado
//! del frame loop del chrome (B4.3) montan encima.
//!
//! El modelo sigue el spec CSS Animations Level 1 + CSS Easing Level 1:
//! `delay` corre el arranque; `duration` define el largo de un ciclo;
//! `iteration-count` cuántos ciclos; `direction` si los ciclos impares se
//! reflejan; `fill-mode` si el primer/último frame "pega" fuera de la
//! ventana activa; `timing-function` mapea el progreso lineal del ciclo
//! al progreso efectivo.

use crate::boxes::Color;
use crate::style::{
    parse_color, parse_opacity, parse_transforms, AnimationBinding, AnimationDirection,
    AnimationFillMode, AnimationIterations, AnimationPlayState, EasingFunction, Keyframes,
    Transform, TransitionBinding,
};

/// Mapea el progreso lineal `t ∈ [0,1]` de un ciclo al progreso efectivo
/// según la función de easing. Valores fuera de `[0,1]` se clampean
/// (CSS no extrapola easing fuera del ciclo, salvo cubic-bezier que sí
/// puede sobrepasar en Y — eso lo permitimos al no clampear la salida).
pub fn apply_easing(f: EasingFunction, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match f {
        EasingFunction::Linear => t,
        // Los cuatro presets son cubic-beziers fijos del spec.
        EasingFunction::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0, t),
        EasingFunction::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
        EasingFunction::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
        EasingFunction::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
        EasingFunction::CubicBezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, t),
        // `step-start` ≡ `steps(1, jump-start)`, `step-end` ≡ `steps(1, jump-end)`.
        EasingFunction::StepStart => step_ease(1, true, t),
        EasingFunction::StepEnd => step_ease(1, false, t),
        EasingFunction::Steps(n, jump_start) => step_ease(n, jump_start, t),
    }
}

/// Función escalonada del spec CSS Easing (`steps(n, jump-start|jump-end)`).
/// Sólo cubrimos los dos jump-terms que el parser produce (`start`/`end`);
/// `jump-both`/`jump-none` no se parsean todavía.
fn step_ease(n: u32, jump_start: bool, t: f32) -> f32 {
    let steps = n.max(1) as f32;
    let mut current = (t * steps).floor();
    if jump_start {
        current += 1.0;
    }
    if t >= 0.0 && current < 0.0 {
        current = 0.0;
    }
    // `jumps` para jump-start/jump-end es igual a `steps`.
    if t <= 1.0 && current > steps {
        current = steps;
    }
    current / steps
}

/// Evalúa una cubic-bezier de timing CSS en `t` (que es la coordenada X,
/// el progreso temporal lineal) y devuelve la Y (el progreso efectivo).
/// P0=(0,0), P3=(1,1) son fijos; sólo varían los dos puntos de control.
/// Resuelve `bezier_x(u) = t` por Newton-Raphson con fallback a bisección,
/// y devuelve `bezier_y(u)`.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let u = solve_bezier_x(x1, x2, t);
    bezier_axis(y1, y2, u)
}

/// Componente de una cubic-bezier (eje X o Y) en el parámetro `u ∈ [0,1]`,
/// con P0=0 y P3=1: `3(1-u)²·u·c1 + 3(1-u)·u²·c2 + u³`.
fn bezier_axis(c1: f32, c2: f32, u: f32) -> f32 {
    let omu = 1.0 - u;
    3.0 * omu * omu * u * c1 + 3.0 * omu * u * u * c2 + u * u * u
}

/// Derivada de `bezier_axis` respecto de `u` — para Newton-Raphson.
fn bezier_axis_deriv(c1: f32, c2: f32, u: f32) -> f32 {
    let omu = 1.0 - u;
    3.0 * omu * omu * c1 + 6.0 * omu * u * (c2 - c1) + 3.0 * u * u * (1.0 - c2)
}

/// Encuentra `u` tal que `bezier_x(u) == x`. Newton-Raphson arrancando en
/// `u = x` (buen guess porque X suele ser casi-lineal); si la derivada es
/// degenerada o no converge, cae a bisección que siempre converge.
fn solve_bezier_x(x1: f32, x2: f32, x: f32) -> f32 {
    let mut u = x;
    for _ in 0..8 {
        let fx = bezier_axis(x1, x2, u) - x;
        if fx.abs() < 1e-6 {
            return u;
        }
        let d = bezier_axis_deriv(x1, x2, u);
        if d.abs() < 1e-6 {
            break;
        }
        u -= fx / d;
    }
    // Bisección de respaldo.
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    let mut u = x;
    for _ in 0..32 {
        let fx = bezier_axis(x1, x2, u);
        if (fx - x).abs() < 1e-6 {
            return u;
        }
        if fx < x {
            lo = u;
        } else {
            hi = u;
        }
        u = (lo + hi) * 0.5;
    }
    u
}

/// Aplica `direction` al progreso lineal `local_t ∈ [0,1)` de un ciclo,
/// dado el índice de iteración (0-based). Devuelve el progreso lineal
/// "dirigido" (todavía sin easing).
fn directed_local(iteration: u32, local_t: f32, dir: AnimationDirection) -> f32 {
    let odd = iteration % 2 == 1;
    match dir {
        AnimationDirection::Normal => local_t,
        AnimationDirection::Reverse => 1.0 - local_t,
        AnimationDirection::Alternate => {
            if odd {
                1.0 - local_t
            } else {
                local_t
            }
        }
        AnimationDirection::AlternateReverse => {
            if odd {
                local_t
            } else {
                1.0 - local_t
            }
        }
    }
}

/// Computa el progreso eased ∈ [0,1] al que muestrear los keyframes, dado
/// el [`AnimationBinding`] y el tiempo transcurrido (en segundos) desde el
/// inicio nominal de la animación (t=0 = el momento en que el elemento
/// adquirió la animación, *antes* de aplicar `delay`).
///
/// Devuelve:
/// - `Some(eased)` cuando hay que aplicar overlay en ese progreso (fase
///   activa, o fase pre/post con `fill-mode` que "pega" el frame límite).
/// - `None` cuando el elemento debe renderear su estilo base (antes del
///   delay sin `backwards`/`both`, o terminada sin `forwards`/`both`).
pub fn animation_progress(binding: &AnimationBinding, elapsed_s: f32) -> Option<f32> {
    // `animation-play-state: paused` congela la animación. Como el chrome no
    // contabiliza tiempo-corrido por elemento (no hay tracking de pausas
    // dinámicas), sólo soportamos el caso estático: el binding declara
    // `paused` en CSS y la animación queda congelada en su primer frame
    // (inicio de la fase activa, progreso 0). Resume dinámico vía JS/:hover
    // no se refleja — divergencia documentada.
    let elapsed_s = if matches!(binding.play_state, AnimationPlayState::Paused) {
        binding.delay_s
    } else {
        elapsed_s
    };
    let dur = binding.duration_s;
    let dir = binding.direction;
    let fill = binding.fill_mode;

    // Tiempo dentro de la fase activa (puede ser negativo: delay positivo
    // todavía sin arrancar; o delay negativo arranca mid-animation).
    let active = elapsed_s - binding.delay_s;

    // --- Fase de delay (antes del primer ciclo) ---
    if active < 0.0 {
        let shows_backwards =
            matches!(fill, AnimationFillMode::Backwards | AnimationFillMode::Both);
        if !shows_backwards {
            return None;
        }
        // "Pega" el primer frame de la iteración 0.
        let directed = directed_local(0, 0.0, dir);
        return Some(apply_easing(binding.timing, directed));
    }

    // Duración <= 0: el ciclo es instantáneo. Tratamos como ya terminada
    // (cae directo a la lógica de fin con iteración entera).
    let total_iters = match binding.iterations {
        AnimationIterations::Infinite => f32::INFINITY,
        AnimationIterations::Count(n) => n.max(0.0),
    };

    if total_iters == 0.0 {
        // 0 iteraciones: nunca corre. Sólo forwards/both pegan el frame
        // final (que coincide con el inicial al no haber ciclos).
        return finished_progress(binding, 0.0, dir, fill);
    }

    if dur <= 0.0 {
        // Ciclo de duración nula: salta directo al estado final.
        return finished_progress(binding, total_iters, dir, fill);
    }

    let cycles = active / dur; // cuántos ciclos transcurrieron (fraccional)

    // --- Fase post (animación terminada, iteración finita) ---
    if total_iters.is_finite() && cycles >= total_iters {
        return finished_progress(binding, total_iters, dir, fill);
    }

    // --- Fase activa ---
    let iteration = cycles.floor();
    let local_t = cycles - iteration;
    let directed = directed_local(iteration as u32, local_t, dir);
    Some(apply_easing(binding.timing, directed))
}

/// Progreso eased "pegado" al final de la animación, según `fill-mode`.
/// `total_iters` es el conteo (posiblemente fraccional) de ciclos al
/// terminar; con conteo entero el último ciclo queda en local_t=1.0.
fn finished_progress(
    binding: &AnimationBinding,
    total_iters: f32,
    dir: AnimationDirection,
    fill: AnimationFillMode,
) -> Option<f32> {
    let shows_forwards = matches!(fill, AnimationFillMode::Forwards | AnimationFillMode::Both);
    if !shows_forwards {
        return None;
    }
    // Iteración y local_t del punto de cierre.
    let (iteration, local_t) = if total_iters <= 0.0 {
        (0u32, 0.0_f32)
    } else if total_iters.fract() == 0.0 {
        // Conteo entero: el último ciclo completo termina en local 1.0.
        ((total_iters as u32).saturating_sub(1), 1.0)
    } else {
        (total_iters.floor() as u32, total_iters.fract())
    };
    let directed = directed_local(iteration, local_t, dir);
    Some(apply_easing(binding.timing, directed))
}

/// Overlay de propiedades animadas a aplicar sobre el estilo base de un
/// nodo en un instante dado. Cada campo es `Some` sólo si algún keyframe
/// de la animación toca esa propiedad — los `None` dejan pasar el estilo
/// base. B4.2 cubre las cuatro propiedades de mayor uso real en
/// `@keyframes`; el resto (width/height, márgenes, etc.) se sumará cuando
/// aparezca un caso.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimatedOverlay {
    pub opacity: Option<f32>,
    pub color: Option<Color>,
    pub background: Option<Color>,
    pub transforms: Option<Vec<Transform>>,
}

impl AnimatedOverlay {
    /// `true` si el overlay no toca ninguna propiedad — el caller puede
    /// saltearse el merge y renderear el estilo base tal cual.
    pub fn is_empty(&self) -> bool {
        self.opacity.is_none()
            && self.color.is_none()
            && self.background.is_none()
            && self.transforms.is_none()
    }
}

/// Interpola los valores de un `@keyframes` en el progreso `t ∈ [0,1]`
/// (que ya viene eased desde [`animation_progress`]). Para cada propiedad
/// animable recolecta los pasos que la declaran (en orden de offset) y
/// muestrea entre los dos que rodean `t`. Si `t` cae antes del primer
/// paso que declara la propiedad se usa ese primero; si cae después del
/// último, ese último (clamp en los bordes — no extrapola).
///
/// **Divergencia documentada**: el spec sintetiza el `from`/`to`
/// faltante a partir del *computed value* base del elemento; acá no
/// tenemos ese valor en este punto del pipeline, así que sólo
/// interpolamos entre pasos que declaran la propiedad explícitamente. El
/// patrón común (`from {…} to {…}` con la prop en ambos, o `0%`/`100%`)
/// funciona; un keyframe que sólo declara la prop en `50%` la mantendrá
/// constante fuera de ese punto en vez de fundir contra la base.
pub fn sample_keyframes(kf: &Keyframes, t: f32) -> AnimatedOverlay {
    AnimatedOverlay {
        opacity: sample_prop(kf, &["opacity"], t, |s| parse_opacity(s), lerp_f32),
        color: sample_prop(kf, &["color"], t, |s| parse_color(s), lerp_color),
        background: sample_prop(
            kf,
            &["background-color", "background"],
            t,
            |s| parse_color(s),
            lerp_color,
        ),
        transforms: sample_prop(kf, &["transform"], t, |s| parse_transforms(s), lerp_transforms),
    }
}

/// Recolecta `(offset, valor)` de los pasos que declaran alguna de
/// `prop_names` (la primera que matchee por paso gana), los muestrea en
/// `t` e interpola con `lerp`. `parse` convierte el value crudo del
/// keyframe al tipo tipado; pasos cuyo value no parsea se ignoran.
fn sample_prop<T: Clone>(
    kf: &Keyframes,
    prop_names: &[&str],
    t: f32,
    parse: impl Fn(&str) -> Option<T>,
    lerp: impl Fn(&T, &T, f32) -> T,
) -> Option<T> {
    // Pasos vienen ordenados por offset ascendente (garantía de Keyframes).
    let mut points: Vec<(f32, T)> = Vec::new();
    for step in &kf.steps {
        for (prop, value) in &step.declarations {
            if prop_names.iter().any(|n| prop.eq_ignore_ascii_case(n)) {
                if let Some(v) = parse(value.trim()) {
                    points.push((step.offset, v));
                }
                break; // una declaración por paso para esta familia de props
            }
        }
    }
    if points.is_empty() {
        return None;
    }
    if t <= points[0].0 {
        return Some(points[0].1.clone());
    }
    let last = points.len() - 1;
    if t >= points[last].0 {
        return Some(points[last].1.clone());
    }
    // Buscar el par que rodea a `t`.
    for w in points.windows(2) {
        let (oa, va) = (&w[0].0, &w[0].1);
        let (ob, vb) = (&w[1].0, &w[1].1);
        if t >= *oa && t <= *ob {
            let span = ob - oa;
            let local = if span > 0.0 { (t - oa) / span } else { 0.0 };
            return Some(lerp(va, vb, local));
        }
    }
    Some(points[last].1.clone())
}

/// Interpolación lineal escalar. Pública: la usan tanto el sampler de
/// keyframes como el runtime de transiciones (Fase F) y el chrome.
pub fn lerp_f32(a: &f32, b: &f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
}

/// Interpola dos colores canal por canal (incluye alfa), redondeando a u8.
pub fn lerp_color(a: &Color, b: &Color, t: f32) -> Color {
    Color {
        r: lerp_u8(a.r, b.r, t),
        g: lerp_u8(a.g, b.g, t),
        b: lerp_u8(a.b, b.b, t),
        a: lerp_u8(a.a, b.a, t),
    }
}

/// Interpola dos listas de transforms posición-a-posición cuando las
/// variantes coinciden en cada índice (caso común: `from`/`to` con el
/// mismo `transform: translate(...) scale(...)`). Si difieren en largo o
/// en tipo de variante, snapea a la lista destino (`b`) — CSS hace
/// interpolación matricial ahí, que es más cara y poco frecuente en
/// keyframes simples; lo dejamos documentado para B4.x si aparece.
pub fn lerp_transforms(a: &Vec<Transform>, b: &Vec<Transform>, t: f32) -> Vec<Transform> {
    if a.len() != b.len() {
        return b.clone();
    }
    let mut out = Vec::with_capacity(a.len());
    for (ta, tb) in a.iter().zip(b.iter()) {
        out.push(match (ta, tb) {
            (Transform::Translate(ax, ay), Transform::Translate(bx, by)) => {
                Transform::Translate(ax + (bx - ax) * t, ay + (by - ay) * t)
            }
            (Transform::Scale(ax, ay), Transform::Scale(bx, by)) => {
                Transform::Scale(ax + (bx - ax) * t, ay + (by - ay) * t)
            }
            (Transform::Rotate(ad), Transform::Rotate(bd)) => {
                Transform::Rotate(ad + (bd - ad) * t)
            }
            // Variantes distintas en este índice → snap al destino.
            _ => *tb,
        });
    }
    out
}

// ===== Transiciones CSS (Fase F) =====
//
// Núcleo puro del runtime de `transition`. A diferencia de las animaciones
// (keyframes con timeline absoluto), una transición arranca cuando el valor
// computado de una propiedad CAMBIA por un cambio de estado (`:hover`,
// `:focus`, clase vía JS). El reloj se ancla en ese instante; estas
// funciones reciben `elapsed_s` = tiempo desde el cambio. El cableado del
// reloj + el tracking de estado por elemento vive en el chrome (fase
// posterior) — acá sólo está el cálculo, igual que `anim` arrancó en B4.1.

/// Progreso eased `∈ [0,1]` de una transición, dado el tiempo transcurrido
/// desde que el valor cambió. Maneja `delay` (positivo retrasa el arranque;
/// negativo arranca mid-transición) y `duration`. Antes del arranque
/// devuelve `0.0` (valor de origen); una vez completada, `1.0` (destino).
/// Duración nula → salto inmediato a `1.0`.
pub fn transition_progress(binding: &TransitionBinding, elapsed_s: f32) -> f32 {
    let active = elapsed_s - binding.delay_s;
    if active <= 0.0 {
        return 0.0;
    }
    if binding.duration_s <= 0.0 {
        return 1.0;
    }
    let t = (active / binding.duration_s).clamp(0.0, 1.0);
    apply_easing(binding.timing, t)
}

/// ¿Esta binding cubre la propiedad `prop`? `all` cubre cualquier propiedad
/// animable; `none` no cubre nada; el resto matchea por nombre exacto
/// (case-insensitive).
pub fn transition_covers(binding: &TransitionBinding, prop: &str) -> bool {
    if binding.property.eq_ignore_ascii_case("none") {
        return false;
    }
    binding.property.eq_ignore_ascii_case("all") || binding.property.eq_ignore_ascii_case(prop)
}

/// De una lista de `transition`s, elige la que aplica a `prop`. CSS dice
/// que "la última declaración que nombra la propiedad gana", así que
/// recorremos de atrás hacia adelante y devolvemos la primera que cubre
/// (incluido un `all` tardío que pisa a una específica anterior). `None`
/// si ninguna la cubre → la propiedad cambia instantáneamente.
pub fn transition_for<'a>(
    list: &'a [TransitionBinding],
    prop: &str,
) -> Option<&'a TransitionBinding> {
    list.iter().rev().find(|b| transition_covers(b, prop))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{
        AnimationBinding, AnimationDirection, AnimationFillMode, AnimationIterations,
        EasingFunction, KeyframeStep, Keyframes, TransitionBinding,
    };

    fn trans(prop: &str, dur: f32, delay: f32) -> TransitionBinding {
        TransitionBinding {
            property: prop.into(),
            duration_s: dur,
            timing: EasingFunction::Linear,
            delay_s: delay,
        }
    }

    fn aprox(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "esperaba ~{b}, fue {a}");
    }

    // ---- apply_easing ----

    #[test]
    fn linear_es_identidad() {
        aprox(apply_easing(EasingFunction::Linear, 0.0), 0.0);
        aprox(apply_easing(EasingFunction::Linear, 0.3), 0.3);
        aprox(apply_easing(EasingFunction::Linear, 1.0), 1.0);
    }

    #[test]
    fn easing_clampea_fuera_de_rango() {
        aprox(apply_easing(EasingFunction::Linear, -0.5), 0.0);
        aprox(apply_easing(EasingFunction::Linear, 1.5), 1.0);
    }

    #[test]
    fn cubic_bezier_pasa_por_los_extremos() {
        // Cualquier bezier fija (0,0) y (1,1).
        aprox(apply_easing(EasingFunction::Ease, 0.0), 0.0);
        aprox(apply_easing(EasingFunction::Ease, 1.0), 1.0);
        aprox(apply_easing(EasingFunction::EaseInOut, 0.5), 0.5); // simétrica en el medio
    }

    #[test]
    fn ease_in_arranca_lento() {
        // ease-in: en t=0.5 el progreso efectivo es < 0.5 (acelera al final).
        let y = apply_easing(EasingFunction::EaseIn, 0.5);
        assert!(y < 0.5, "ease-in en 0.5 debería ir atrasado, fue {y}");
    }

    #[test]
    fn ease_out_arranca_rapido() {
        let y = apply_easing(EasingFunction::EaseOut, 0.5);
        assert!(y > 0.5, "ease-out en 0.5 debería ir adelantado, fue {y}");
    }

    #[test]
    fn cubic_bezier_custom_resuelve_x() {
        // Una bezier conocida: linear disfrazada con puntos en la diagonal.
        let f = EasingFunction::CubicBezier(0.33, 0.33, 0.66, 0.66);
        aprox(apply_easing(f, 0.5), 0.5);
    }

    #[test]
    fn step_end_escalona_hacia_abajo() {
        // steps(4, end): t=0 → 0, t=0.24 → 0, t=0.25 → 0.25, t=0.99 → 0.75, t=1 → 1.
        let f = EasingFunction::Steps(4, false);
        aprox(apply_easing(f, 0.0), 0.0);
        aprox(apply_easing(f, 0.24), 0.0);
        aprox(apply_easing(f, 0.25), 0.25);
        aprox(apply_easing(f, 0.99), 0.75);
        aprox(apply_easing(f, 1.0), 1.0);
    }

    #[test]
    fn step_start_escalona_hacia_arriba() {
        // steps(4, start): t=0 → 0.25, t=0.5 → 0.75, t=1 → 1.
        let f = EasingFunction::Steps(4, true);
        aprox(apply_easing(f, 0.0), 0.25);
        aprox(apply_easing(f, 0.5), 0.75);
        aprox(apply_easing(f, 1.0), 1.0);
    }

    #[test]
    fn step_start_y_end_aliases() {
        aprox(apply_easing(EasingFunction::StepStart, 0.0), 1.0);
        aprox(apply_easing(EasingFunction::StepEnd, 0.0), 0.0);
        aprox(apply_easing(EasingFunction::StepEnd, 1.0), 1.0);
    }

    // ---- animation_progress ----

    fn bind(dur: f32, iters: AnimationIterations) -> AnimationBinding {
        AnimationBinding {
            name: "x".into(),
            duration_s: dur,
            timing: EasingFunction::Linear,
            delay_s: 0.0,
            iterations: iters,
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
        }
    }

    #[test]
    fn progreso_lineal_simple() {
        let b = bind(2.0, AnimationIterations::Count(1.0));
        aprox(animation_progress(&b, 0.0).unwrap(), 0.0);
        aprox(animation_progress(&b, 1.0).unwrap(), 0.5);
        // En t=2.0 ya terminó y fill=None → None.
        assert_eq!(animation_progress(&b, 2.0), None);
    }

    #[test]
    fn play_state_paused_congela_en_el_primer_frame() {
        let mut b = bind(2.0, AnimationIterations::Infinite);
        b.play_state = AnimationPlayState::Paused;
        // Sin importar el reloj, una animación pausada queda en progreso 0
        // (su primer frame), no avanza con el elapsed.
        aprox(animation_progress(&b, 0.0).unwrap(), 0.0);
        aprox(animation_progress(&b, 5.0).unwrap(), 0.0);
        aprox(animation_progress(&b, 100.0).unwrap(), 0.0);
        // Running (default) sí avanza: a mitad de los 2s → 0.5.
        b.play_state = AnimationPlayState::Running;
        aprox(animation_progress(&b, 1.0).unwrap(), 0.5);
    }

    #[test]
    fn delay_positivo_sin_fill_no_aplica() {
        let mut b = bind(1.0, AnimationIterations::Count(1.0));
        b.delay_s = 1.0;
        assert_eq!(animation_progress(&b, 0.5), None);
        aprox(animation_progress(&b, 1.5).unwrap(), 0.5);
    }

    #[test]
    fn fill_backwards_pega_el_primer_frame_en_delay() {
        let mut b = bind(1.0, AnimationIterations::Count(1.0));
        b.delay_s = 1.0;
        b.fill_mode = AnimationFillMode::Backwards;
        aprox(animation_progress(&b, 0.0).unwrap(), 0.0);
    }

    #[test]
    fn fill_forwards_pega_el_ultimo_frame_al_terminar() {
        let mut b = bind(1.0, AnimationIterations::Count(1.0));
        b.fill_mode = AnimationFillMode::Forwards;
        aprox(animation_progress(&b, 5.0).unwrap(), 1.0);
    }

    #[test]
    fn infinite_nunca_termina() {
        let b = bind(2.0, AnimationIterations::Infinite);
        // En t=2.0 arranca el segundo ciclo (local_t=0).
        aprox(animation_progress(&b, 2.0).unwrap(), 0.0);
        aprox(animation_progress(&b, 3.0).unwrap(), 0.5);
        aprox(animation_progress(&b, 1000.5).unwrap(), 0.25);
    }

    #[test]
    fn direction_reverse_invierte() {
        let mut b = bind(2.0, AnimationIterations::Infinite);
        b.direction = AnimationDirection::Reverse;
        aprox(animation_progress(&b, 0.0).unwrap(), 1.0);
        aprox(animation_progress(&b, 1.0).unwrap(), 0.5);
    }

    #[test]
    fn direction_alternate_refleja_ciclos_impares() {
        let mut b = bind(1.0, AnimationIterations::Infinite);
        b.direction = AnimationDirection::Alternate;
        // Ciclo 0 (par): normal.
        aprox(animation_progress(&b, 0.25).unwrap(), 0.25);
        // Ciclo 1 (impar): reflejado.
        aprox(animation_progress(&b, 1.25).unwrap(), 0.75);
    }

    #[test]
    fn iteraciones_fraccionales_terminan_a_mitad() {
        let mut b = bind(2.0, AnimationIterations::Count(1.5));
        b.fill_mode = AnimationFillMode::Forwards;
        // 1.5 ciclos × 2s = 3s. En t=3 termina; el punto final es local 0.5
        // de la iteración 1 (dirección Normal → progreso 0.5).
        aprox(animation_progress(&b, 3.0).unwrap(), 0.5);
    }

    #[test]
    fn delay_negativo_arranca_mid_animation() {
        let mut b = bind(2.0, AnimationIterations::Count(1.0));
        b.delay_s = -1.0; // arranca como si ya hubiera pasado 1s
        aprox(animation_progress(&b, 0.0).unwrap(), 0.5);
    }

    // ---- sample_keyframes ----

    fn kf(steps: &[(f32, &[(&str, &str)])]) -> Keyframes {
        Keyframes {
            steps: steps
                .iter()
                .map(|(offset, decls)| KeyframeStep {
                    offset: *offset,
                    declarations: decls
                        .iter()
                        .map(|(p, v)| (p.to_string(), v.to_string()))
                        .collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn opacity_interpola_linealmente() {
        let k = kf(&[(0.0, &[("opacity", "0")]), (1.0, &[("opacity", "1")])]);
        aprox(sample_keyframes(&k, 0.0).opacity.unwrap(), 0.0);
        aprox(sample_keyframes(&k, 0.5).opacity.unwrap(), 0.5);
        aprox(sample_keyframes(&k, 1.0).opacity.unwrap(), 1.0);
    }

    #[test]
    fn propiedad_ausente_queda_none() {
        let k = kf(&[(0.0, &[("opacity", "0")]), (1.0, &[("opacity", "1")])]);
        let ov = sample_keyframes(&k, 0.5);
        assert!(ov.color.is_none());
        assert!(ov.background.is_none());
        assert!(ov.transforms.is_none());
        assert!(!ov.is_empty()); // opacity sí está
    }

    #[test]
    fn keyframes_vacio_da_overlay_vacio() {
        let k = kf(&[]);
        assert!(sample_keyframes(&k, 0.5).is_empty());
    }

    #[test]
    fn color_interpola_por_canal() {
        let k = kf(&[(0.0, &[("color", "#000000")]), (1.0, &[("color", "#ffffff")])]);
        let c = sample_keyframes(&k, 0.5).color.unwrap();
        assert_eq!((c.r, c.g, c.b), (128, 128, 128));
    }

    #[test]
    fn background_acepta_background_color_y_background() {
        let k1 = kf(&[(0.0, &[("background-color", "#000")]), (1.0, &[("background-color", "#fff")])]);
        assert!(sample_keyframes(&k1, 0.5).background.is_some());
        let k2 = kf(&[(0.0, &[("background", "#000")]), (1.0, &[("background", "#fff")])]);
        assert!(sample_keyframes(&k2, 0.5).background.is_some());
    }

    #[test]
    fn transform_interpola_posicion_a_posicion() {
        let k = kf(&[
            (0.0, &[("transform", "translate(0px, 0px) scale(1)")]),
            (1.0, &[("transform", "translate(100px, 50px) scale(3)")]),
        ]);
        let ts = sample_keyframes(&k, 0.5).transforms.unwrap();
        assert_eq!(ts.len(), 2);
        match (ts[0], ts[1]) {
            (Transform::Translate(x, y), Transform::Scale(sx, sy)) => {
                aprox(x, 50.0);
                aprox(y, 25.0);
                aprox(sx, 2.0);
                aprox(sy, 2.0);
            }
            other => panic!("estructura inesperada: {other:?}"),
        }
    }

    #[test]
    fn transform_mismatch_snapea_al_destino() {
        let k = kf(&[
            (0.0, &[("transform", "rotate(0deg)")]),
            (1.0, &[("transform", "scale(2)")]),
        ]);
        // Estructuras distintas → snap a `to`.
        let ts = sample_keyframes(&k, 0.5).transforms.unwrap();
        assert_eq!(ts, vec![Transform::Scale(2.0, 2.0)]);
    }

    #[test]
    fn antes_del_primer_paso_clampa_al_primero() {
        // La prop sólo se declara en 0.5 y 1.0; en t=0.2 toma la de 0.5.
        let k = kf(&[(0.5, &[("opacity", "0.4")]), (1.0, &[("opacity", "1")])]);
        aprox(sample_keyframes(&k, 0.2).opacity.unwrap(), 0.4);
    }

    #[test]
    fn tres_pasos_usa_el_par_correcto() {
        let k = kf(&[
            (0.0, &[("opacity", "0")]),
            (0.5, &[("opacity", "1")]),
            (1.0, &[("opacity", "0")]),
        ]);
        // En 0.25 va a mitad del primer tramo (0→1).
        aprox(sample_keyframes(&k, 0.25).opacity.unwrap(), 0.5);
        // En 0.75 va a mitad del segundo tramo (1→0).
        aprox(sample_keyframes(&k, 0.75).opacity.unwrap(), 0.5);
    }

    #[test]
    fn value_no_parseable_se_ignora() {
        // El paso 0.5 tiene basura; sólo quedan 0.0 y 1.0 como puntos válidos.
        let k = kf(&[
            (0.0, &[("opacity", "0")]),
            (0.5, &[("opacity", "no-soy-un-numero")]),
            (1.0, &[("opacity", "1")]),
        ]);
        aprox(sample_keyframes(&k, 0.5).opacity.unwrap(), 0.5);
    }

    // ---- transiciones ----

    #[test]
    fn transition_progress_respeta_delay_duracion() {
        let t = trans("opacity", 2.0, 1.0);
        // Antes del delay → todavía en origen (0).
        aprox(transition_progress(&t, 0.5), 0.0);
        aprox(transition_progress(&t, 1.0), 0.0);
        // delay=1, a mitad de los 2s de duración (elapsed 2.0) → 0.5.
        aprox(transition_progress(&t, 2.0), 0.5);
        // Completada y más allá → 1 (clamp).
        aprox(transition_progress(&t, 3.0), 1.0);
        aprox(transition_progress(&t, 99.0), 1.0);
    }

    #[test]
    fn transition_progress_duracion_nula_es_instantanea() {
        let t = trans("color", 0.0, 0.0);
        aprox(transition_progress(&t, 0.001), 1.0);
    }

    #[test]
    fn transition_progress_delay_negativo_arranca_avanzado() {
        // delay -1 sobre 2s: en elapsed 0 ya va por la mitad.
        let t = trans("opacity", 2.0, -1.0);
        aprox(transition_progress(&t, 0.0), 0.5);
    }

    #[test]
    fn transition_covers_all_none_y_especifica() {
        assert!(transition_covers(&trans("all", 1.0, 0.0), "opacity"));
        assert!(transition_covers(&trans("opacity", 1.0, 0.0), "opacity"));
        assert!(!transition_covers(&trans("opacity", 1.0, 0.0), "color"));
        assert!(!transition_covers(&trans("none", 1.0, 0.0), "opacity"));
    }

    #[test]
    fn transition_for_la_ultima_que_cubre_gana() {
        // `all 1s` luego `opacity 2s`: opacity usa la específica (la última
        // que la nombra).
        let list = vec![trans("all", 1.0, 0.0), trans("opacity", 2.0, 0.0)];
        assert_eq!(transition_for(&list, "opacity").unwrap().duration_s, 2.0);
        // color sólo lo cubre el `all`.
        assert_eq!(transition_for(&list, "color").unwrap().duration_s, 1.0);
        // Un `all` tardío pisa a una específica anterior.
        let list2 = vec![trans("opacity", 2.0, 0.0), trans("all", 1.0, 0.0)];
        assert_eq!(transition_for(&list2, "opacity").unwrap().duration_s, 1.0);
        // Ninguna cubre → None.
        assert!(transition_for(&[trans("color", 1.0, 0.0)], "opacity").is_none());
    }

    #[test]
    fn lerps_publicos_basicos() {
        aprox(lerp_f32(&0.0, &10.0, 0.5), 5.0);
        let c = lerp_color(
            &Color { r: 0, g: 0, b: 0, a: 255 },
            &Color { r: 100, g: 200, b: 50, a: 255 },
            0.5,
        );
        assert_eq!((c.r, c.g, c.b, c.a), (50, 100, 25, 255));
        let ts = lerp_transforms(
            &vec![Transform::Translate(0.0, 0.0)],
            &vec![Transform::Translate(100.0, 40.0)],
            0.25,
        );
        assert_eq!(ts[0], Transform::Translate(25.0, 10.0));
    }
}
