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
        // Re-pinned al fondo: la ventana del archive vuelve a "cola" liviana
        // (las últimas N), así no carga de más cuando no se la mira.
        if let Ok(mut c) = s.surf_spilled_visible.lock() {
            c.window_start = None;
        }
    } else {
        s.scroll_px = overflow - new_scroll_y;
        s.surf_scroll_anchor = overflow;
        s = maybe_page_spill_back(s, new_scroll_y, overflow);
    }
    s
}

/// Altura de una línea de output (espeja `view::command_card::ROW_H`, privado
/// a ese módulo). Usada para la matemática de anclaje del paginado del archive.
const SPILL_ROW_H: f32 = 16.0;

/// Fase 5.12 — al rozar el borde superior del contenido, paginá el archive
/// spilled hacia atrás cargando una página más vieja. Prepender K líneas no
/// cambia la distancia al fondo (`scroll_px` queda igual): sólo subimos el
/// ancla por `K·row_h` para que la línea que el usuario mira no salte cuando el
/// próximo render agregue esas líneas arriba.
fn maybe_page_spill_back(mut s: State, new_scroll_y: f32, overflow: f32) -> State {
    let row_h = SPILL_ROW_H * s.font_zoom.clamp(0.5, 3.0);
    let spilled_count = s
        .surf_history
        .lock()
        .map(|h| h.spilled_count())
        .unwrap_or(0);
    let window_start = s.surf_spilled_visible.lock().ok().and_then(|c| c.window_start);
    let Some(new_start) =
        crate::spill_page_back(window_start, spilled_count, new_scroll_y, row_h)
    else {
        return s;
    };
    let effective = crate::spill_effective_start(window_start, spilled_count);
    let k = effective.saturating_sub(new_start);
    if let Ok(mut c) = s.surf_spilled_visible.lock() {
        c.window_start = Some(new_start);
    }
    // El render sumará K líneas arriba → overflow crece K·row_h. Subimos el
    // ancla igual para preservar la posición visual (scroll_px ya quedó fijo).
    s.surf_scroll_anchor = overflow + k as f32 * row_h;
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
