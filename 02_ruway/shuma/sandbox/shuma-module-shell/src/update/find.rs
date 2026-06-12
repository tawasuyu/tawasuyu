use super::*;

/// Edita la query y re-busca. Resetea `current` al primer match (lo más
/// natural cuando uno está tipeando — el resaltado salta a la primera
/// ocurrencia conforme se escribe).
pub(crate) fn apply_find_edit(mut s: State, mutate: impl FnOnce(&mut String)) -> State {
    if let Some(f) = s.find.as_mut() {
        mutate(&mut f.query);
    } else {
        return s;
    }
    recompute_find(s)
}

/// Re-corre `find_matches` con la query/política vigentes y arma
/// `surf_selection` con el match `current` (o el primero si recién hubo
/// edición). Si la nueva query no matchea nada, `current = None` y la
/// selección se limpia.
pub(crate) fn recompute_find(mut s: State) -> State {
    use llimphi_widget_terminal::{find_matches, FindOpts};
    let Some(f) = s.find.as_mut() else {
        return s;
    };
    let snap = match s.surf_layout.lock() {
        Ok(g) => g.clone(),
        Err(p) => p.into_inner().clone(),
    };
    let Some(snap) = snap else {
        // Sin layout publicado, no hay nada que buscar. Mantenemos la query
        // pero matches vacíos; al primer render volvemos a entrar.
        f.matches.clear();
        f.current = None;
        s.surf_selection = None;
        return s;
    };
    f.matches = find_matches(
        &snap.store,
        &f.query,
        FindOpts { case_insensitive: f.case_insensitive },
    );
    if f.matches.is_empty() {
        f.current = None;
        s.surf_selection = None;
        s
    } else {
        f.current = Some(0);
        apply_current_match(s, &snap)
    }
}

/// Avanza/retrocede el match actual (cíclico) y refleja como selección.
pub(crate) fn step_find(mut s: State, forward: bool) -> State {
    use llimphi_widget_terminal::{next_match, prev_match};
    let snap = match s.surf_layout.lock() {
        Ok(g) => g.clone(),
        Err(p) => p.into_inner().clone(),
    };
    let Some(snap) = snap else {
        return s;
    };
    let Some(f) = s.find.as_mut() else {
        return s;
    };
    if f.matches.is_empty() {
        return s;
    }
    f.current = if forward {
        next_match(&f.matches, f.current)
    } else {
        prev_match(&f.matches, f.current)
    };
    apply_current_match(s, &snap)
}

/// Refleja el match `current` de `find` como `surf_selection` y ajusta
/// `scroll_px` para traerlo a la vista (centrado en el viewport, clampeado
/// al overflow). Toma `snap` aparte para no doble-lockear `surf_layout`.
pub(crate) fn apply_current_match(mut s: State, snap: &crate::SurfLayout) -> State {
    use llimphi_widget_terminal::{line_top_in_content, Point, SelectionRange};
    let Some(f) = s.find.as_ref() else {
        return s;
    };
    let Some(i) = f.current else {
        return s;
    };
    let Some(m) = f.matches.get(i).copied() else {
        return s;
    };
    // Selección = el span del match (mismo painter del overlay; ya
    // copiable con SurfCopySelection).
    s.surf_selection = Some(SelectionRange {
        anchor: Point::new(m.line, m.start),
        head: Point::new(m.line, m.end),
    });
    // Auto-scroll: lleva la línea del match a la mitad del viewport.
    if let Some(line_top) = line_top_in_content(&snap.items_geo, snap.metrics.line_height, m.line) {
        let centered = (line_top - snap.viewport_h * 0.5).max(0.0);
        // Convertir scroll_y (desde arriba) a scroll_px (desde abajo) — el
        // modelo del shell usa esta convención para anclar al fondo en
        // ausencia de scroll manual.
        let overflow = s.out_overflow.lock().map(|g| *g).unwrap_or(0.0);
        s.scroll_px = (overflow - centered).clamp(0.0, overflow);
        // Anchor del scroll para que el find sobreviva appends sucesivos
        // (Fase 5: anclaje estable). Si quedó pinned al fondo, anchor=0.
        s.surf_scroll_anchor = if s.scroll_px > 0.5 { overflow } else { 0.0 };
    }
    s
}
