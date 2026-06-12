use super::*;

/// Navega el historial por Up/Down.
pub(crate) fn navigate_history(mut s: State, dir: shuma_history::Nav) -> State {
    let next = {
        let history = s.history.lock().unwrap();
        history
            .navigate(s.history_cursor, dir)
            .map(|(i, e)| (i, e.line.clone()))
    };
    if let Some((i, line)) = next {
        s.history_cursor = Some(i);
        s.input.set_text(line);
    } else if matches!(dir, shuma_history::Nav::Newer) {
        // Salir del historial al final: línea vacía.
        s.history_cursor = None;
        s.input.clear();
    }
    s
}

/// Maneja teclas mientras el overlay Ctrl-R está abierto.
pub(crate) fn handle_search_key(mut s: State, ev: &KeyEvent) -> State {
    let Some(mut search) = s.history_search.take() else {
        return s;
    };
    match &ev.key {
        Key::Named(NamedKey::Escape) => {
            // Salida sin aceptar.
            return s;
        }
        Key::Named(NamedKey::Enter) => {
            // Acepta el seleccionado: pasa a la línea (sin ejecutar).
            let pick = {
                let history = s.history.lock().unwrap();
                history
                    .fuzzy_search(&search.query, 50)
                    .get(search.selected)
                    .map(|e| e.line.clone())
            };
            if let Some(line) = pick {
                s.input.set_text(line);
            }
            return s;
        }
        Key::Named(NamedKey::Backspace) => {
            search.query.pop();
            search.selected = 0;
        }
        Key::Named(NamedKey::ArrowDown) => {
            let history = s.history.lock().unwrap();
            let max = history.fuzzy_search(&search.query, 50).len();
            if max > 0 && search.selected + 1 < max {
                search.selected += 1;
            }
        }
        Key::Named(NamedKey::ArrowUp) => {
            search.selected = search.selected.saturating_sub(1);
        }
        _ => {
            if let Some(text) = &ev.text {
                if !text.is_empty() && !text.chars().any(|c| c.is_control()) {
                    search.query.push_str(text);
                    search.selected = 0;
                }
            }
        }
    }
    s.history_search = Some(search);
    s
}

/// Maneja teclas mientras la barra de find del cuerpo de output está
/// abierta (Ctrl+F). Esc cierra; Enter avanza (Shift+Enter retrocede);
/// Backspace borra; cualquier char visible se concatena a la query y
/// re-busca. F3/Shift+F3 son atajos alternativos para next/prev.
pub(crate) fn handle_find_key(s: State, ev: &KeyEvent) -> State {
    if s.find.is_none() {
        return s;
    }
    match &ev.key {
        Key::Named(NamedKey::Escape) => update(s, Msg::FindClose),
        Key::Named(NamedKey::Enter) | Key::Named(NamedKey::F3) => {
            let msg = if ev.modifiers.shift { Msg::FindPrev } else { Msg::FindNext };
            update(s, msg)
        }
        Key::Named(NamedKey::Backspace) => update(s, Msg::FindBackspace),
        _ => {
            if let Some(text) = &ev.text {
                let mut s = s;
                for c in text.chars() {
                    if !c.is_control() {
                        s = update(s, Msg::FindChar(c));
                    }
                }
                s
            } else {
                s
            }
        }
    }
}
