use super::*;

/// Aplica un delta de scroll a la superficie, manteniendo el invariante de
/// anclaje (Fase 5.0). Devuelve `s` con `scroll_px` / `surf_scroll_anchor`
/// actualizados. NO toca `surf_scroll_velocity` — eso lo hacen los callers
/// (`Msg::Scroll` la captura, `step_scroll_inertia` la decae).
pub(crate) fn apply_scroll_delta(mut s: State, delta: f32) -> State {
    let overflow = s.out_overflow.lock().map(|g| *g).unwrap_or(0.0);
    // Re-baseline a la `scroll_y` intencionada del usuario contra el
    // `overflow` actual (Fase 5: anclaje estable bajo append).
    let prev_anchor = if s.surf_scroll_anchor > 0.5 {
        s.surf_scroll_anchor
    } else {
        overflow
    };
    let curr_scroll_y = (prev_anchor - s.scroll_px).clamp(0.0, overflow);
    // `delta > 0` = rueda arriba = ver historial (scroll_y baja).
    let new_scroll_y = (curr_scroll_y - delta).clamp(0.0, overflow);
    // Si el usuario alcanzó el fondo, re-pin al bottom (scroll_px=0).
    // Threshold de 0.5 absorbe ruido sub-pixel.
    if new_scroll_y >= overflow - 0.5 {
        s.scroll_px = 0.0;
        s.surf_scroll_anchor = 0.0;
    } else {
        s.scroll_px = overflow - new_scroll_y;
        s.surf_scroll_anchor = overflow;
    }
    s
}

/// Aplica un paso de scroll inercial: si la velocidad supera el umbral,
/// scrollea por ella y decae por fricción. Si tocó el fondo (re-pin), la
/// inercia se detiene (evita el "fantasma" de seguir scrolleando contra
/// el límite). Lo llama el handler de `Msg::Tick` por frame.
pub(crate) fn step_scroll_inertia(mut s: State) -> State {
    /// Magnitud bajo la cual consideramos que el scroll está quieto, en px.
    const EPSILON: f32 = 0.5;
    /// Factor de fricción aplicado por tick (~100 ms). 0.82 → la inercia
    /// decae a ~10% en ~12 ticks (~1.2 s). Tuneable.
    const FRICTION: f32 = 0.82;
    if s.surf_scroll_velocity.abs() <= EPSILON {
        s.surf_scroll_velocity = 0.0;
        return s;
    }
    let v = s.surf_scroll_velocity;
    s = apply_scroll_delta(s, v);
    // Si el delta nos dejó pinned al fondo, parar la inercia para no
    // simular un "rebote" contra el borde.
    if s.scroll_px <= f32::EPSILON {
        s.surf_scroll_velocity = 0.0;
    } else {
        s.surf_scroll_velocity *= FRICTION;
    }
    s
}
