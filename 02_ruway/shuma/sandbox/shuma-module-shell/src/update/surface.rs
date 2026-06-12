use super::*;

/// Actualiza la selección viva del cuerpo de output en modo superficie. El
/// primer Move arranca (`anchor = head = point_at(ax, ay)`); los siguientes
/// extienden (`head = point_at(acc)`); End deja la selección fijada pero
/// `surf_selecting = false` para que un próximo Move arranque limpio.
pub(crate) fn apply_surf_select_drag(
    mut s: State,
    phase: llimphi_ui::DragPhase,
    dx: f32,
    dy: f32,
    ax: f32,
    ay: f32,
) -> State {
    use llimphi_ui::DragPhase;
    use llimphi_widget_terminal::{point_at_geo, SelectionRange};
    // Snapshot del layout publicado por la `view` el frame previo. Sin él
    // no podemos resolver `(lx, ly)` a `Point` — es no-op silencioso.
    let snap = match s.surf_layout.lock() {
        Ok(g) => g.clone(),
        Err(p) => p.into_inner().clone(),
    };
    let Some(snap) = snap else {
        return s;
    };
    match phase {
        DragPhase::Move => {
            if !s.surf_selecting {
                // Primer evento del drag: ancla en (ax, ay).
                s.surf_selecting = true;
                s.surf_drag_acc = (ax, ay);
                let p = point_at_geo(
                    &snap.items_geo,
                    snap.scroll_y,
                    snap.viewport_h,
                    snap.metrics,
                    snap.gutter_w,
                    &snap.store,
                    ax,
                    ay,
                );
                s.surf_selection = p.map(SelectionRange::collapsed);
            } else {
                // Extender: acumulamos delta sobre la posición previa.
                s.surf_drag_acc.0 += dx;
                s.surf_drag_acc.1 += dy;
                let p = point_at_geo(
                    &snap.items_geo,
                    snap.scroll_y,
                    snap.viewport_h,
                    snap.metrics,
                    snap.gutter_w,
                    &snap.store,
                    s.surf_drag_acc.0,
                    s.surf_drag_acc.1,
                );
                if let (Some(sel), Some(p)) = (s.surf_selection.as_mut(), p) {
                    sel.head = p;
                }
            }
        }
        DragPhase::End => {
            s.surf_selecting = false;
            // Si el drag fue tan corto que la selección quedó colapsada,
            // limpiamos — un click sin arrastre no debería dejar una
            // selección vacía visible (es la misma UX que xterm/gnome-term).
            if let Some(sel) = s.surf_selection {
                if sel.is_empty() {
                    s.surf_selection = None;
                }
            }
        }
    }
    s
}

/// Copia al clipboard el texto de la selección viva (paridad con el
/// `:copy` del modo card y con el Ctrl+C de xterm). No-op silencioso si no
/// hay selección o el clipboard no está disponible.
pub(crate) fn copy_surf_selection(s: &State) {
    let Some(sel) = s.surf_selection.as_ref() else {
        return;
    };
    let snap = match s.surf_layout.lock() {
        Ok(g) => g.clone(),
        Err(p) => p.into_inner().clone(),
    };
    let Some(snap) = snap else {
        return;
    };
    let text = sel.slice_text(&snap.store);
    if text.is_empty() {
        return;
    }
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
    }
}

/// Doble-click sobre el cuerpo de output: selecciona la palabra bajo el
/// punto (paridad con xterm/gnome-terminal). Resuelve `(lx, ly)` a `Point`
/// con `point_at_geo`, computa los boundaries de palabra en char-indices y
/// los convierte a offsets de byte UTF-8 para armar el `SelectionRange`.
pub(crate) fn apply_surf_double_click(
    mut s: State,
    lx: f32,
    ly: f32,
    _rect_w: f32,
    _rect_h: f32,
) -> State {
    use llimphi_widget_terminal::{point_at_geo, Point, SelectionRange};
    let snap = match s.surf_layout.lock() {
        Ok(g) => g.clone(),
        Err(p) => p.into_inner().clone(),
    };
    let Some(snap) = snap else {
        return s;
    };
    let Some(hit) = point_at_geo(
        &snap.items_geo,
        snap.scroll_y,
        snap.viewport_h,
        snap.metrics,
        snap.gutter_w,
        &snap.store,
        lx,
        ly,
    ) else {
        return s;
    };
    let Some(text) = snap.store.line(hit.line) else {
        return s;
    };
    // El click se entrega en byte_col; `word_range_at` opera en char-indices.
    // Convertir byte → char.
    let char_col = text[..hit.col.min(text.len())].chars().count();
    let (start_char, end_char) = word_range_at(text, char_col);
    if end_char <= start_char {
        return s;
    }
    // Char-indices → byte offsets.
    let mut chars_seen = 0usize;
    let mut start_byte = text.len();
    let mut end_byte = text.len();
    for (b, _) in text.char_indices() {
        if chars_seen == start_char {
            start_byte = b;
        }
        if chars_seen == end_char {
            end_byte = b;
            break;
        }
        chars_seen += 1;
    }
    s.surf_selection = Some(SelectionRange {
        anchor: Point::new(hit.line, start_byte),
        head: Point::new(hit.line, end_byte),
    });
    s
}

/// Triple-click sobre el cuerpo de output: selecciona la línea entera bajo
/// el punto (paridad con xterm/gnome-terminal). Reusa `point_at_geo` para
/// localizar la línea y arma `SelectionRange` de (line, 0) a (line,
/// text.len()). No-op silencioso si el click cae en chrome o fuera del
/// store.
pub(crate) fn apply_surf_triple_click(mut s: State, lx: f32, ly: f32) -> State {
    use llimphi_widget_terminal::{point_at_geo, Point, SelectionRange};
    let snap = match s.surf_layout.lock() {
        Ok(g) => g.clone(),
        Err(p) => p.into_inner().clone(),
    };
    let Some(snap) = snap else {
        return s;
    };
    let Some(hit) = point_at_geo(
        &snap.items_geo,
        snap.scroll_y,
        snap.viewport_h,
        snap.metrics,
        snap.gutter_w,
        &snap.store,
        lx,
        ly,
    ) else {
        return s;
    };
    let Some(text) = snap.store.line(hit.line) else {
        return s;
    };
    s.surf_selection = Some(SelectionRange {
        anchor: Point::new(hit.line, 0),
        head: Point::new(hit.line, text.len()),
    });
    s
}

/// Aplica el item elegido del menú contextual del surface y lo cierra.
/// 0 = Copiar selección · 1 = Copiar todo el scrollback · 2 = Seleccionar todo.
pub(crate) fn apply_surf_menu_pick(mut s: State, idx: usize) -> State {
    use llimphi_widget_terminal::{Point, SelectionRange};
    let snap = match s.surf_layout.lock() {
        Ok(g) => g.clone(),
        Err(p) => p.into_inner().clone(),
    };
    let Some(snap) = snap else {
        s.surf_menu = None;
        return s;
    };
    match idx {
        0 => copy_surf_selection(&s),
        1 => {
            // Copia todo el scrollback vigente (líneas spilled NO incluidas —
            // serían lookups async; el menú "todo" copia lo en memoria).
            let n = snap.store.len();
            if n > 0 {
                let text = snap.store.slice_text(0, n);
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set_text(text);
                }
            }
        }
        2 => {
            // Selección desde (0,0) hasta el final de la última línea.
            let n = snap.store.len();
            if n > 0 {
                let last = n - 1;
                let last_len = snap.store.line(last).map(|t| t.len()).unwrap_or(0);
                s.surf_selection = Some(SelectionRange {
                    anchor: Point::new(0, 0),
                    head: Point::new(last, last_len),
                });
            }
        }
        _ => {}
    }
    s.surf_menu = None;
    s
}
