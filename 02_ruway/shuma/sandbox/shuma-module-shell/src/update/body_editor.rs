use super::*;

/// Aplica un evento de puntero del cuerpo IDE-text de una card: Click
/// posiciona el caret, Drag extiende la selección (acumulando el delta
/// contra el press, igual que `nada`). Reconstruye el `EditorState` del
/// bloque desde su texto (la fuente de verdad) + el cursor guardado, lo
/// muta, y guarda el cursor de vuelta en `state.body_sel`.
pub(crate) fn apply_body_pointer(
    mut s: State,
    block: u64,
    ev: llimphi_widget_text_editor::PointerEvent,
) -> State {
    use llimphi_widget_text_editor::PointerEvent;
    let metrics = body_editor_metrics();
    let mut ed = body_editor_state(&s, block);
    let scroll = ed.scroll_offset;
    match ev {
        PointerEvent::Click { x, y } => {
            s.body_drag_accum = (0.0, 0.0);
            let (line, col) = metrics.screen_to_pos(x, y, scroll);
            ed.set_caret_at(line, col);
        }
        PointerEvent::Drag {
            initial_x,
            initial_y,
            dx,
            dy,
        } => {
            s.body_drag_accum.0 += dx;
            s.body_drag_accum.1 += dy;
            let cur_x = initial_x + s.body_drag_accum.0;
            let cur_y = initial_y + s.body_drag_accum.1;
            let (line, col) = metrics.screen_to_pos(cur_x, cur_y, scroll);
            ed.extend_selection_to(line, col);
        }
    }
    s.body_sel = Some((block, ed.cursor.clone()));
    s
}

/// Rango `[start, end)` (en columnas/chars) de la palabra en `line_text`
/// que contiene la columna `col` — alfanumérico + `_`, igual que el
/// text-editor. Si `col` cae sobre un no-word-char, devuelve un rango
/// vacío en `col` (no selecciona).
pub(crate) fn word_range_at(line_text: &str, col: usize) -> (usize, usize) {
    let chars: Vec<char> = line_text.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    if col >= chars.len() || !is_word(chars[col]) {
        // Permití también el caso "el cursor quedó justo después de la
        // última letra de la palabra" (col == len o sobre separador): mirá
        // el char anterior.
        if col > 0 && col <= chars.len() && is_word(chars[col - 1]) {
            let mut start = col;
            while start > 0 && is_word(chars[start - 1]) {
                start -= 1;
            }
            return (start, col);
        }
        return (col, col);
    }
    let mut start = col;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    (start, end)
}

/// Doble-click sobre el cuerpo: selecciona la palabra bajo el punto. `x`/`y`
/// son locales al nodo del editor (incluyen el gutter), así que restamos
/// `gutter_width` para pasar a coords del área de texto.
pub(crate) fn apply_body_double_click(mut s: State, block: u64, x: f32, y: f32) -> State {
    let metrics = body_editor_metrics();
    let mut ed = body_editor_state(&s, block);
    let content_x = x - metrics.gutter_width;
    let (line, col) = metrics.screen_to_pos(content_x, y, ed.scroll_offset);
    // Texto de la línea para calcular los límites de la palabra.
    let lines = body_lines_for_block(&s, block);
    let Some(line_text) = lines.get(line) else {
        return s;
    };
    let (start, end) = word_range_at(line_text, col);
    if end > start {
        ed.set_caret_at(line, start);
        ed.extend_selection_to(line, end);
        s.body_drag_accum = (0.0, 0.0);
        s.body_sel = Some((block, ed.cursor.clone()));
    }
    s
}

/// Copia al clipboard la selección viva del cuerpo de `block` (click
/// derecho). No-op si no hay selección en ese bloque.
pub(crate) fn copy_body_selection(s: &State, block: u64) {
    let Some((b, _)) = s.body_sel.as_ref() else {
        return;
    };
    if *b != block {
        return;
    }
    let ed = body_editor_state(s, block);
    if let Some(text) = ed.selected_text() {
        set_clipboard(&text);
    }
}

/// Copia el bloque entero al clipboard: el comando (`$ …`) seguido de su
/// salida completa (stdout y stderr en orden). A diferencia de
/// [`copy_body_selection`], no depende de que haya una selección viva — es el
/// "copiar comando + salida" estilo terminal moderna. No-op si el bloque no
/// tiene ni comando ni cuerpo.
pub(crate) fn copy_command_block(s: &State, block: u64) {
    let mut partes: Vec<String> = Vec::new();
    if let Some(cmd) = s.block_command.get(&block) {
        // `block_command` guarda el texto ya con el prefijo "$ ".
        partes.push(cmd.clone());
    }
    partes.extend(body_lines_for_block(s, block));
    if partes.is_empty() {
        return;
    }
    set_clipboard(&partes.join("\n"));
}

/// Bloque objetivo del menú contextual del output: el que el usuario tiene
/// seleccionado, o el más reciente con cuerpo. `None` si no hay ninguno (no
/// hay nada que copiar → no se abre el menú).
pub(crate) fn menu_target_block(s: &State) -> Option<u64> {
    if let Some((b, _)) = s.body_sel {
        return Some(b);
    }
    s.output
        .iter()
        .rev()
        .find(|l| {
            l.block != 0
                && l.kind != OutputKind::Prompt
                && l.stage.is_none()
                && !is_status_line(&l.text)
        })
        .map(|l| l.block)
}

/// `true` si el bloque objetivo del menú tiene una selección viva (para
/// habilitar/deshabilitar el item "Copiar selección").
pub(crate) fn menu_has_selection(s: &State, block: u64) -> bool {
    matches!(s.body_sel.as_ref(), Some((b, _)) if *b == block)
        && body_editor_state(s, block).selected_text().is_some()
}

/// Aplica el item elegido del menú contextual del output y lo cierra.
/// 0 = Copiar selección · 1 = Copiar todo el bloque · 2 = Seleccionar todo.
pub(crate) fn apply_body_menu_pick(mut s: State, idx: usize) -> State {
    let Some((_, _, block)) = s.body_menu else {
        return s;
    };
    match idx {
        0 => copy_body_selection(&s, block),
        1 => {
            let text = body_lines_for_block(&s, block).join("\n");
            if !text.is_empty() {
                set_clipboard(&text);
            }
        }
        2 => {
            let mut ed = body_editor_state(&s, block);
            ed.select_all();
            s.body_sel = Some((block, ed.cursor.clone()));
        }
        _ => {}
    }
    s.body_menu = None;
    s
}
