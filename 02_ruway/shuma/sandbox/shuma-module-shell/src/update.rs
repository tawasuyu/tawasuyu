use super::*;

/// Mapea `action_id` de `ShortcutAction::ModuleAction` al `Msg`.
pub fn dispatch(action_id: &str) -> Option<Msg> {
    match action_id {
        "shell.clear" => Some(Msg::Clear),
        "shell.cancel" => Some(Msg::Cancel),
        _ => None,
    }
}

/// Traduce un `KeyEvent` a una llamada sobre `LineState`. Devuelve
/// `true` si tocó el state. No maneja Enter, Tab, Up/Down ni Ctrl-C
/// (esos los intercepta el `update` del módulo).
pub(crate) fn apply_key_to_line(line: &mut LineState, ev: &KeyEvent) -> bool {
    match &ev.key {
        Key::Named(NamedKey::Backspace) => {
            line.backspace();
            true
        }
        Key::Named(NamedKey::Delete) => {
            line.delete();
            true
        }
        Key::Named(NamedKey::ArrowLeft) => {
            if ev.modifiers.ctrl {
                line.move_word_left();
            } else {
                line.move_left();
            }
            true
        }
        Key::Named(NamedKey::ArrowRight) => {
            if ev.modifiers.ctrl {
                line.move_word_right();
            } else {
                line.move_right();
            }
            true
        }
        Key::Named(NamedKey::Home) => {
            line.move_home();
            true
        }
        Key::Named(NamedKey::End) => {
            line.move_end();
            true
        }
        Key::Named(NamedKey::Space) => {
            line.insert(" ");
            true
        }
        _ => {
            if let Some(text) = &ev.text {
                if !text.is_empty() && !text.chars().any(|c| c.is_control()) {
                    line.insert(text);
                    return true;
                }
            }
            false
        }
    }
}

pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Key(ev) => {
            if ev.state != KeyState::Pressed {
                return s;
            }
            // Si hay un TUI activo, las teclas van al stdin del PTY
            // (no al input). El usuario sale tipeando dentro del TUI
            // (`:q` en vim, `q` en less, etc.).
            if is_tui_active(&s) {
                // Shift+Insert siempre pega. Ctrl-V también — en TUIs
                // tipo less/vim no suele ser un binding (vim usa Ctrl-V
                // para visual-block en normal mode; al editar dentro
                // de insert mode tampoco). Si choca con un usuario
                // específico, en el futuro lo gateamos por allowlist.
                let paste = (ev.modifiers.ctrl
                    && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("v")))
                    || (ev.modifiers.shift && matches!(&ev.key, Key::Named(NamedKey::Insert)));
                if paste {
                    forward_paste_to_pty(&s);
                    return s;
                }
                forward_key_to_pty(&s, &ev);
                return s;
            }
            // Cualquier tecla del input reancla el parpadeo del caret (queda
            // sólido un instante y luego titila) — el input se siente vivo.
            s.input_edit_at_ms = now_unix_millis();
            // Si el overlay de búsqueda está abierto, las teclas van ahí.
            if s.history_search.is_some() {
                return handle_search_key(s, &ev);
            }
            // Ctrl-C: si hay run vivo, mandarle SIGTERM y comer la tecla.
            if ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("c"))
            {
                if s.running.is_some() {
                    return cancel_running(s);
                }
            }
            // Ctrl-V (o Shift+Insert): pega del clipboard al input.
            // (Si hay TUI, lo intercepta `is_tui_active` arriba; ese
            // camino tiene su propio paste.)
            let is_paste = (ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("v")))
                || (ev.modifiers.shift && matches!(&ev.key, Key::Named(NamedKey::Insert)));
            if is_paste {
                if let Some(text) = read_clipboard() {
                    s.input.insert(&text);
                    refresh_completion(&mut s);
                }
                return s;
            }
            // Ctrl-R: abrir overlay de búsqueda de historial.
            if ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("r"))
            {
                s.history_search = Some(HistorySearch::default());
                return s;
            }
            // Popup de completado abierto (vivo, mientras tipeás): las teclas
            // lo navegan. Tab/→ aceptan el resaltado; las flechas ↑↓ y
            // Shift+Tab ciclan; Enter NO acepta — ejecuta el comando como
            // está (el popup es una sugerencia, no un modal).
            if s.completion.is_some() {
                match &ev.key {
                    Key::Named(NamedKey::Tab) if ev.modifiers.shift => {
                        return cycle_completion(s, -1);
                    }
                    Key::Named(NamedKey::Tab) | Key::Named(NamedKey::ArrowRight) => {
                        return accept_completion(s);
                    }
                    Key::Named(NamedKey::ArrowDown) => return cycle_completion(s, 1),
                    Key::Named(NamedKey::ArrowUp) => return cycle_completion(s, -1),
                    Key::Named(NamedKey::Escape) => {
                        close_completion(&mut s);
                        return s;
                    }
                    // Enter cae al manejador de abajo (ejecuta); el resto de
                    // teclas cierra el popup y se procesa normal (lo
                    // reabriremos vivo tras la edición).
                    Key::Named(NamedKey::Enter) => close_completion(&mut s),
                    _ => close_completion(&mut s),
                }
            }
            // F1..F8: ejecuta el grupo guardado de esa posición (`:save`).
            // (F12 lo reserva el chasis para cerrar.)
            if let Some(idx) = fkey_index(&ev.key) {
                return run_group(s, idx);
            }
            // Enter: ejecuta — pero si el texto deja una construcción
            // abierta (quote, paren, heredoc, `\` final, pipe pendiente),
            // insertamos un salto de línea y seguimos editando.
            // Shift+Enter fuerza salto de línea siempre.
            if let Key::Named(NamedKey::Enter) = ev.key {
                let pending = shuma_line::needs_continuation(s.input.text());
                if pending || ev.modifiers.shift {
                    s.input.insert("\n");
                    s.history_cursor = None;
                    return s;
                }
                s.history_cursor = None;
                s = run_submitted(s);
                return s;
            }
            // Tab: completion.
            if let Key::Named(NamedKey::Tab) = ev.key {
                return apply_completion_msg(s);
            }
            // Up/Down: navegación de historial.
            if let Key::Named(NamedKey::ArrowUp) = ev.key {
                return navigate_history(s, shuma_history::Nav::Older);
            }
            if let Key::Named(NamedKey::ArrowDown) = ev.key {
                return navigate_history(s, shuma_history::Nav::Newer);
            }
            // Flecha derecha al final de línea con ghost visible: acepta ghost.
            if let Key::Named(NamedKey::ArrowRight) = ev.key {
                if !ev.modifiers.ctrl && s.input.cursor() == s.input.text().len() {
                    if let Some(suffix) = current_ghost(&s) {
                        if !suffix.is_empty() {
                            s.input.insert(&suffix);
                            return s;
                        }
                    }
                }
            }
            apply_key_to_line(&mut s.input, &ev);
            // Cualquier edición rompe el cursor de navegación de historial.
            s.history_cursor = None;
            // Refresco vivo del popup de completado (as-you-type, estilo el
            // shuma viejo): aparece solo mientras hay un prefijo a completar.
            refresh_completion(&mut s);
        }
        Msg::FocusInput => {
            s.focused = true;
        }
        Msg::Clear => {
            s.clear_output();
        }
        Msg::ToggleBlock(id) => {
            if !s.collapsed.remove(&id) {
                s.collapsed.insert(id);
            }
        }
        Msg::Scroll(delta) => {
            // `out_overflow` lo publicó la última `view`; clampa sin que
            // el handler tenga que recomputar la geometría.
            let overflow = s.out_overflow.lock().map(|g| *g).unwrap_or(0.0);
            s.scroll_px = (s.scroll_px + delta).clamp(0.0, overflow);
        }
        Msg::RunLine(line) => {
            s.input.set_text(line);
            s = run_submitted(s);
        }
        Msg::ToggleStage { block, stage } => {
            let key = (block, stage);
            if !s.expanded_stages.remove(&key) {
                s.expanded_stages.insert(key);
            }
        }
        Msg::SetReprocess(block) => {
            // Toggle: re-armar el mismo bloque lo desarma.
            if s.reprocess_source == Some(block) {
                s.reprocess_source = None;
            } else {
                s.reprocess_source = Some(block);
                s.focused = true;
            }
        }
        Msg::RunGroup(idx) => {
            s = run_group(s, idx);
        }
        Msg::BodyPointer { block, ev } => {
            s = apply_body_pointer(s, block, ev);
        }
        Msg::CopyBody(block) => {
            copy_body_selection(&s, block);
        }
        Msg::BodyDoubleClick { block, x, y } => {
            s = apply_body_double_click(s, block, x, y);
        }
        Msg::Tick => {
            s = drain_run(s);
        }
        Msg::Cancel => {
            if s.running.is_some() {
                s = cancel_running(s);
            }
        }
        Msg::OpenDecoration(kind) => {
            s = open_decoration(s, kind);
        }
        Msg::InsertAtCursor(text) => {
            // Cerramos cualquier overlay activo para que el texto
            // pegado quede visible sin tener que cerrar el Ctrl-R a mano.
            s.history_search = None;
            s.history_cursor = None;
            s.input.insert(&text);
            s.focused = true;
        }
        Msg::VimPaste => {
            // Sólo aplica si hay un TUI vivo; `forward_paste_to_pty` es
            // no-op silencioso si no.
            forward_paste_to_pty(&s);
        }
        Msg::VimDrag {
            end,
            dx,
            dy,
            ax,
            ay,
        } => {
            let fresh = s.vim_sel.map_or(true, |v| !v.active);
            if fresh {
                s.vim_sel = Some(VimSel {
                    ax,
                    ay,
                    hx: ax + dx,
                    hy: ay + dy,
                    active: !end,
                });
            } else if let Some(v) = s.vim_sel.as_mut() {
                v.hx += dx;
                v.hy += dy;
                if end {
                    v.active = false;
                }
            }
            if end {
                // Umbral mínimo de drag: un click (o jitter sub-celda) no
                // selecciona ni copia. Exige cruzar ~una celda para contar.
                let dragged = s.vim_sel.is_some_and(|v| {
                    let (dx, dy) = (v.hx - v.ax, v.hy - v.ay);
                    (dx * dx + dy * dy).sqrt() >= crate::view::VIM_CHAR_W as f32
                });
                if dragged {
                    copy_vim_selection(&s);
                } else {
                    s.vim_sel = None;
                }
            }
        }
    }
    s
}

/// Acciona el click sobre una decoración del output. Ninguna acción
/// bloquea la UI: `xdg-open` se forkea detached, y los cambios al
/// state (cwd, input) son in-memory.
pub(crate) fn open_decoration(mut s: State, kind: shuma_line::DecorationKind) -> State {
    use shuma_line::DecorationKind as Dk;
    match kind {
        Dk::Path {
            abs,
            is_dir,
            is_executable,
            ..
        } => {
            if is_dir {
                // Directorios → cd. Cambia el cwd y lo refleja en el
                // header sin "ejecutar" un comando.
                if abs.is_dir() {
                    s.cwd = abs;
                    s.completion_source = Arc::new(ShellSource::new(&s.cwd));
                }
            } else if is_executable {
                // Binarios → pre-llenar el input con el path; el
                // usuario decide los args y Enter.
                s.input.set_text(abs.display().to_string());
            } else {
                // Archivos regulares → xdg-open detached.
                spawn_detached("xdg-open", &[abs.display().to_string().as_str()]);
            }
        }
        Dk::Url(url) => {
            spawn_detached("xdg-open", &[&url]);
        }
        Dk::GrepRef { abs, line_no, col } => {
            // `$EDITOR +line file` para vim/neovim/helix; si no hay
            // EDITOR, xdg-open al archivo y listo.
            if let Ok(editor) = std::env::var("EDITOR") {
                let line_flag = format!("+{line_no}");
                let path = abs.display().to_string();
                let args: Vec<&str> = match col {
                    Some(_) => vec![&line_flag, &path],
                    None => vec![&line_flag, &path],
                };
                spawn_detached(&editor, &args);
            } else {
                spawn_detached("xdg-open", &[abs.display().to_string().as_str()]);
            }
        }
        Dk::GitSha(sha) => {
            // Pre-llenar `git show <sha>` — la acción más útil 99% del tiempo.
            s.input.set_text(format!("git show {sha}"));
        }
        Dk::IssueRef(_) | Dk::BoxDraw => {
            // Sin acción asociada.
        }
    }
    s
}

/// Lanza un proceso "detached" — no esperamos, no leemos su output,
/// y el padre puede morir sin matarlo (`process_group(0)` para
/// despegarlo de la sesión de shuma). Usado para `xdg-open` y `$EDITOR`
/// disparados desde clicks.
pub(crate) fn spawn_detached(program: &str, args: &[&str]) {
    use std::os::unix::process::CommandExt;
    let _ = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
}

/// Aplica un Tab:
/// - popup abierto: cicla al siguiente candidato (no toca el texto, así el
///   rango de reemplazo del `Completion` guardado sigue válido).
/// - popup cerrado: 0 candidatos → nada; 1 → lo inserta directo; ≥2 → abre
///   el popup con el primero resaltado (sin tocar el texto todavía).
pub(crate) fn apply_completion_msg(mut s: State) -> State {
    if let Some(comp) = &s.completion {
        let n = comp.candidates.len();
        if n > 0 {
            s.completion_index = (s.completion_index + 1) % n;
        }
        return s;
    }
    let comp = s.input.complete(s.completion_source.as_ref());
    if comp.is_empty() {
        return s;
    }
    if comp.candidates.len() == 1 {
        let candidate = comp.candidates[0].clone();
        s.input.apply_completion(&comp, &candidate);
        return s;
    }
    s.completion = Some(comp);
    s.completion_index = 0;
    s
}

/// Cierra el popup de completado sin aplicar nada.
pub(crate) fn close_completion(s: &mut State) {
    s.completion = None;
    s.completion_index = 0;
}

/// Refresca el popup de completado **en vivo** (as-you-type): lo abre cuando
/// hay un prefijo a completar y candidatos, lo cierra si no. Rankea los
/// comandos por uso (frecuencia en el historial) — los más usados primero.
pub(crate) fn refresh_completion(s: &mut State) {
    let mut comp = s.input.complete(s.completion_source.as_ref());
    if comp.candidates.is_empty() || comp.replace_end <= comp.replace_start {
        s.completion = None;
        s.completion_index = 0;
        return;
    }
    rank_completion_by_usage(s, &mut comp);
    s.completion = Some(comp);
    s.completion_index = 0;
}

/// Reordena los candidatos de comando por frecuencia de uso en el historial
/// (desc), con desempate alfabético — "ordenado por prioridad y uso". Sólo
/// aplica a completados de comando; paths/flags quedan como vienen.
pub(crate) fn rank_completion_by_usage(s: &State, comp: &mut shuma_line::Completion) {
    if comp.kind != shuma_line::CompletionKind::Command {
        return;
    }
    let mut freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    if let Ok(h) = s.history.lock() {
        for e in h.entries() {
            if let Some(w) = e.line.split_whitespace().next() {
                *freq.entry(w.to_string()).or_insert(0) += 1;
            }
        }
    }
    comp.candidates.sort_by(|a, b| {
        let fa = freq.get(a).copied().unwrap_or(0);
        let fb = freq.get(b).copied().unwrap_or(0);
        fb.cmp(&fa).then_with(|| a.cmp(b))
    });
}

/// Cicla el candidato resaltado del popup (`delta` ±1, con wrap). No-op si
/// el popup está cerrado.
pub(crate) fn cycle_completion(mut s: State, delta: i32) -> State {
    if let Some(comp) = &s.completion {
        let n = comp.candidates.len() as i32;
        if n > 0 {
            s.completion_index = (s.completion_index as i32 + delta).rem_euclid(n) as usize;
        }
    }
    s
}

/// Acepta el candidato resaltado del popup, lo inserta y cierra el popup.
pub(crate) fn accept_completion(mut s: State) -> State {
    if let Some(comp) = s.completion.take() {
        if let Some(candidate) = comp.candidates.get(s.completion_index) {
            s.input.apply_completion(&comp, candidate);
        }
    }
    s.completion_index = 0;
    s
}

/// Prefijo común más largo de un slice de strings — usado en completion
/// cuando hay múltiples candidatos.
pub(crate) fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else {
        return String::new();
    };
    let mut end = first.len();
    for s in &items[1..] {
        let bytes = s.as_bytes();
        let fbytes = first.as_bytes();
        let mut i = 0;
        while i < end && i < bytes.len() && bytes[i] == fbytes[i] {
            i += 1;
        }
        end = i;
        if end == 0 {
            break;
        }
    }
    // Asegurarse de cortar en límite de carácter UTF-8.
    while end > 0 && !first.is_char_boundary(end) {
        end -= 1;
    }
    first[..end].to_string()
}

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

/// `true` si hay un `ActiveRun` con PTY vivo. Las teclas van al stdin del
/// PTY mientras esto sea cierto (el programa es interactivo, esté o no en
/// pantalla completa). El **render** en cambio sigue a [`is_tui_fullscreen`].
pub(crate) fn is_tui_active(s: &State) -> bool {
    let Some(arc) = s.running.as_ref() else {
        return false;
    };
    let g = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    g.tui.is_some()
}

/// `true` si el PTY vivo entró a **alternate screen** (`ESC[?1049h`) — la
/// señal dura de una app TUI de pantalla completa (vim, htop, less, man…).
/// Es lo que decide pintar el panel full-screen (grid/vim) en vez de las
/// líneas. Al salir del alt-screen (`ESC[?1049l`) vuelve a modo líneas.
pub(crate) fn is_tui_fullscreen(s: &State) -> bool {
    let Some(arc) = s.running.as_ref() else {
        return false;
    };
    let g = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    g.tui
        .as_ref()
        .map(|t| t.parser.screen().alternate_screen())
        .unwrap_or(false)
}

/// Contenido de la pantalla del PTY vivo cuando está en **modo líneas**
/// (PTY presente, sin alt-screen). Devuelve las filas como texto (sin
/// formato), recortando las filas vacías del final. `None` si no hay PTY
/// o está en pantalla completa (ese caso lo pinta el panel full-screen).
/// Las salidas de programas que no toman la pantalla (p. ej. `watch`) se
/// leen así como texto normal en vez de una grilla apretada.
pub(crate) fn pty_line_text(s: &State) -> Option<Vec<String>> {
    let arc = s.running.as_ref()?;
    let g = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let tui = g.tui.as_ref()?;
    let screen = tui.parser.screen();
    if screen.alternate_screen() {
        return None;
    }
    Some(screen_to_lines(screen))
}

/// Filas de un `vt100::Screen` como texto sin formato, recortando las
/// filas vacías del final. Pura (sin State) para poder testearla con un
/// parser construido a mano.
pub(crate) fn screen_to_lines(screen: &vt100::Screen) -> Vec<String> {
    let (_rows, cols) = screen.size();
    let mut lines: Vec<String> = screen
        .rows(0, cols)
        .map(|r| r.trim_end().to_string())
        .collect();
    while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines
}

/// Traduce una tecla a su secuencia de bytes para el PTY (xterm-compat).
/// Las TUIs esperan estos códigos.
pub(crate) fn key_to_pty_bytes(ev: &KeyEvent) -> Vec<u8> {
    match &ev.key {
        Key::Named(NamedKey::Enter) => b"\r".to_vec(),
        Key::Named(NamedKey::Tab) => b"\t".to_vec(),
        Key::Named(NamedKey::Backspace) => b"\x7f".to_vec(),
        Key::Named(NamedKey::Escape) => b"\x1b".to_vec(),
        Key::Named(NamedKey::ArrowUp) => b"\x1b[A".to_vec(),
        Key::Named(NamedKey::ArrowDown) => b"\x1b[B".to_vec(),
        Key::Named(NamedKey::ArrowRight) => b"\x1b[C".to_vec(),
        Key::Named(NamedKey::ArrowLeft) => b"\x1b[D".to_vec(),
        Key::Named(NamedKey::Home) => b"\x1b[H".to_vec(),
        Key::Named(NamedKey::End) => b"\x1b[F".to_vec(),
        Key::Named(NamedKey::PageUp) => b"\x1b[5~".to_vec(),
        Key::Named(NamedKey::PageDown) => b"\x1b[6~".to_vec(),
        Key::Named(NamedKey::Delete) => b"\x1b[3~".to_vec(),
        Key::Named(NamedKey::Space) => b" ".to_vec(),
        _ => {
            // Ctrl-<x>: codifica el byte 0x01..0x1a para letras.
            if ev.modifiers.ctrl {
                if let Key::Character(c) = &ev.key {
                    if let Some(ch) = c.chars().next() {
                        let lo = ch.to_ascii_lowercase();
                        if ('a'..='z').contains(&lo) {
                            return vec![(lo as u8) - b'a' + 1];
                        }
                    }
                }
            }
            ev.text.as_deref().unwrap_or("").as_bytes().to_vec()
        }
    }
}

/// Lee el clipboard del SO (vía `arboard`). Devuelve `None` si no hay
/// display server, está vacío, o el contenido no es texto. No cachea —
/// el sistema tiene su propio TTL.
pub(crate) fn read_clipboard() -> Option<String> {
    let mut clip = arboard::Clipboard::new().ok()?;
    clip.get_text().ok()
}

/// Escribe texto al clipboard del SO. No-op silencioso sin display server.
pub(crate) fn set_clipboard(text: &str) {
    if let Ok(mut clip) = arboard::Clipboard::new() {
        let _ = clip.set_text(text.to_string());
    }
}

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

/// Extrae el texto de la selección del card de vim sobre el screen
/// actual del PTY y lo copia al clipboard. Selección lineal por filas
/// (estilo terminal), cada fila recortada de espacios al final.
pub(crate) fn copy_vim_selection(s: &State) {
    let Some(vs) = s.vim_sel else { return };
    let Some(arc) = s.running.as_ref() else {
        return;
    };
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let Some(tui) = guard.tui.as_ref() else {
        return;
    };
    let screen = tui.parser.screen();
    let (rows, cols) = screen.size();
    let mut grid: Vec<Vec<char>> = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut line: Vec<char> = Vec::with_capacity(cols as usize);
        for c in 0..cols {
            let ch = match screen.cell(r, c) {
                Some(cell) if cell.has_contents() => cell.contents().chars().next().unwrap_or(' '),
                _ => ' ',
            };
            line.push(ch);
        }
        grid.push(line);
    }
    let (cw, lh) = match s.vim_metrics.lock() {
        Ok(g) if g.0 > 1.0 && g.1 > 1.0 => (g.0 as f64, g.1 as f64),
        _ => (crate::view::VIM_CHAR_W, crate::view::VIM_LINE_H),
    };
    let (r0, c0) = crate::view::vim_px_to_cell(vs.ax as f64, vs.ay as f64, cw, lh);
    let (r1, c1) = crate::view::vim_px_to_cell(vs.hx as f64, vs.hy as f64, cw, lh);
    let (sr, sc, er, ec) = if (r0, c0) <= (r1, c1) {
        (r0, c0, r1, c1)
    } else {
        (r1, c1, r0, c0)
    };
    if sr >= grid.len() {
        return;
    }
    let er = er.min(grid.len() - 1);
    let mut out = String::new();
    for r in sr..=er {
        let line = &grid[r];
        let lo = if r == sr { sc.min(line.len()) } else { 0 };
        let hi = if r == er {
            (ec + 1).min(line.len())
        } else {
            line.len()
        };
        if hi > lo {
            let seg: String = line[lo..hi].iter().collect();
            out.push_str(seg.trim_end());
        }
        if r != er {
            out.push('\n');
        }
    }
    if !out.trim().is_empty() {
        set_clipboard(&out);
    }
}

/// Pega el contenido del clipboard en el PTY del run activo. Si el TUI
/// hijo está en bracketed-paste mode (DECSET 2004), envuelve la
/// secuencia en `\x1b[200~...\x1b[201~` para que vim, less y emacs
/// distingan "tipeé esto" de "pegué esto" (auto-indent, paste-mode,
/// etc.). No-op silencioso si no hay TUI o el clipboard está vacío.
pub(crate) fn forward_paste_to_pty(s: &State) {
    let Some(arc) = s.running.as_ref() else {
        return;
    };
    let Some(text) = read_clipboard() else {
        return;
    };
    if text.is_empty() {
        return;
    }
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let bracketed = guard
        .tui
        .as_ref()
        .map(|t| t.parser.screen().bracketed_paste())
        .unwrap_or(false);
    let payload: Vec<u8> = if bracketed {
        let mut buf: Vec<u8> = b"\x1b[200~".to_vec();
        buf.extend_from_slice(text.as_bytes());
        buf.extend_from_slice(b"\x1b[201~");
        buf
    } else {
        text.into_bytes()
    };
    guard.handle.write_input(payload);
}

/// Manda los bytes de la tecla al PTY del run activo. No-op si no hay
/// tui activo.
pub(crate) fn forward_key_to_pty(s: &State, ev: &KeyEvent) {
    let Some(arc) = s.running.as_ref() else {
        return;
    };
    let bytes = key_to_pty_bytes(ev);
    if bytes.is_empty() {
        return;
    }
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.handle.write_input(bytes);
}

/// Rama de git activa para `cwd` — `None` si no estamos en un repo (o si
/// HEAD está detached). Implementación minimalista por archivo: sube por
/// los padres buscando `.git`, lee `HEAD` y extrae `refs/heads/<rama>`. No
/// usa libgit2 ni lanza procesos (barato de llamar por frame).
pub(crate) fn git_branch(cwd: &std::path::Path) -> Option<String> {
    let mut dir = cwd.to_path_buf();
    let git_dir = loop {
        let candidate = dir.join(".git");
        if candidate.exists() {
            break candidate;
        }
        if !dir.pop() {
            return None;
        }
    };
    // `.git` puede ser un archivo (worktrees/submódulos) con `gitdir: …`,
    // o un directorio con `HEAD` dentro.
    let head_path = if git_dir.is_file() {
        let s = std::fs::read_to_string(&git_dir).ok()?;
        let target = s.strip_prefix("gitdir:")?.trim();
        std::path::PathBuf::from(target).join("HEAD")
    } else {
        git_dir.join("HEAD")
    };
    let head = std::fs::read_to_string(head_path).ok()?;
    head.trim()
        .strip_prefix("ref: refs/heads/")
        .map(|b| b.to_string())
}

/// Marcadores de proyecto: archivos/dirs que identifican la "forma" de un
/// directorio. Gatean la predicción por estructura (no sugerir `cargo` sin
/// `Cargo.toml`).
const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "go.mod",
    "Makefile",
    "pyproject.toml",
    "pom.xml",
    "build.gradle",
];

/// Marcadores de proyecto presentes en `dir`.
fn markers_in(dir: &str) -> Vec<String> {
    let base = std::path::Path::new(dir);
    PROJECT_MARKERS
        .iter()
        .filter(|m| base.join(m).exists())
        .map(|m| m.to_string())
        .collect()
}

/// Construye los `CommandRecord` de `shuma-infer` a partir del historial
/// (éxito = exit 0).
fn infer_records(s: &State) -> Vec<shuma_infer::CommandRecord> {
    let Ok(history) = s.history.lock() else {
        return Vec::new();
    };
    history
        .entries()
        .iter()
        // El historial Llimphi aún no graba el exit (siempre `None`):
        // tratamos lo desconocido como éxito para no descartar todo el
        // corpus. Si más adelante se registra el exit, los fallos
        // (`Some(c!=0)`) quedan excluidos automáticamente.
        .map(|e| {
            let ok = e.exit.map_or(true, |c| c == 0);
            shuma_infer::CommandRecord::parse(&e.line, e.cwd.clone(), ok)
        })
        .collect()
}

/// Recalcula los patrones emergentes del historial y los cachea en el
/// state. Se llama al cerrar cada comando (cuando el historial creció).
pub(crate) fn refresh_patterns(s: &mut State) {
    let records = infer_records(s);
    s.patterns = shuma_infer::detect_patterns(&records, &shuma_infer::InferConfig::default());
}

/// Condición de disparo de un patrón: los marcadores de proyecto comunes a
/// todos los directorios donde corrió.
fn pattern_trigger(p: &shuma_infer::EmergingPattern) -> Vec<String> {
    let mut dirs = p.directories.iter();
    let Some(first) = dirs.next() else {
        return Vec::new();
    };
    let mut common = markers_in(first);
    for d in dirs {
        let here = markers_in(d);
        common.retain(|m| here.contains(m));
    }
    common
}

/// La secuencia que el motor predice como continuación de la sesión, si la
/// hay y el cwd comparte la forma del patrón.
pub(crate) fn predicted_sequence(s: &State) -> Option<String> {
    if s.patterns.is_empty() {
        return None;
    }
    let records = infer_records(s);
    let tail = &records[records.len().saturating_sub(6)..];
    let (pi, next) = shuma_infer::predict_next(tail, &s.patterns)?;
    if next.is_empty() {
        return None;
    }
    // Disparo por estructura: no anticipar un patrón en un directorio que
    // no comparte su forma (no sugerir `cargo` sin `Cargo.toml`).
    let trigger = pattern_trigger(&s.patterns[pi]);
    if !trigger.is_empty() {
        let here = markers_in(&s.cwd.to_string_lossy());
        if !trigger.iter().all(|m| here.contains(m)) {
            return None;
        }
    }
    Some(next.join(" && "))
}

/// Sugerencia "ghost" para la línea actual — la secuencia predicha por el
/// motor de patrones (si aplica) y, tras ella, el prefijo histórico más
/// reciente que extiende el texto que ya está tipeado.
pub(crate) fn current_ghost(s: &State) -> Option<String> {
    let text = s.input.text();
    if text.is_empty() || s.input.cursor() != text.len() {
        return None;
    }
    // Corpus por prioridad: secuencia predicha primero, luego historial.
    let mut corpus: Vec<String> = Vec::new();
    if let Some(seq) = predicted_sequence(s) {
        corpus.push(seq);
    }
    if let Ok(history) = s.history.lock() {
        corpus.extend(history.entries().iter().rev().map(|e| e.line.clone()));
    }
    shuma_line::ghost_suggestion(text, &corpus)
}

pub(crate) fn run_submitted(mut s: State) -> State {
    let line = s.input.text().to_string();
    let trimmed = line.trim().to_string();
    s.input.clear();
    if trimmed.is_empty() {
        return s;
    }
    // El comando que estaba en foco recede al historial: se pliega para que
    // el nuevo nazca expandido y la vista no sea un volcado plano. Sólo los
    // que tienen cuerpo (los sin salida no se pliegan; se ven distinto).
    let prev = s.current_block;
    if prev != 0 && !body_lines_for_block(&s, prev).is_empty() {
        s.collapsed.insert(prev);
    }
    s.push_output(OutputLine::prompt(format!("$ {trimmed}")));

    // Append al historial — todo lo que el usuario Enter-eó queda
    // registrado, builtins incluidos (para que `cd ../foo` reaparezca
    // por Up). `IgnoreConsecutive` evita ráfagas iguales.
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = shuma_history::Entry::new(trimmed.clone(), s.cwd.display().to_string(), now);
        if let Ok(mut h) = s.history.lock() {
            let _ = h.append(entry);
        }
    }
    // Recalcula los patrones emergentes con el historial ya actualizado —
    // alimentan la predicción del ghost para el próximo comando.
    refresh_patterns(&mut s);

    // Builtins primero — no spawnean proceso, corren aunque haya run vivo.
    if let Some((cmd, rest)) = split_first_word(&trimmed) {
        match cmd {
            "cd" => {
                return apply_cd(s, rest);
            }
            "pwd" => {
                let cwd_str = s.cwd.display().to_string();
                s.push_output(OutputLine::stdout(cwd_str));
                return s;
            }
            "clear" => {
                s.clear_output();
                return s;
            }
            "exit" => {
                s.push_output(OutputLine::notice(
                    "exit: el chasis maneja la salida (F12 para cerrar)",
                ));
                return s;
            }
            ":jobs" => return apply_jobs_list(s),
            ":term" => return apply_jobs_signal(s, rest, JobSignal::Term),
            ":stop" => return apply_jobs_signal(s, rest, JobSignal::Stop),
            ":cont" => return apply_jobs_signal(s, rest, JobSignal::Cont),
            ":limit" => return apply_capture_limit(s, rest),
            ":spill" => return apply_spill(s, rest),
            ":save" => return save_group(s, rest),
            ":groups" => return apply_groups_list(s),
            _ => {}
        }
    }

    // Sufijo `&` (con espacios opcionales antes) → background. El
    // background siempre arranca, sin encolar; no hay límite.
    if let Some(stripped) = trimmed.strip_suffix('&') {
        let cmd = stripped.trim_end().to_string();
        if cmd.is_empty() {
            return s;
        }
        return start_bg(s, cmd);
    }

    // Comando externo foreground. Si ya hay uno corriendo, lo encolamos;
    // si no, arrancamos ahora mismo.
    if s.running.is_some() {
        s.queue.push_back(trimmed);
        s.push_output(OutputLine::notice(
            "⌛ en cola — esperando a que el comando actual termine",
        ));
        return s;
    }
    start_run(s, trimmed)
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum JobSignal {
    Term,
    Stop,
    Cont,
}

/// Lista los bg_jobs con su índice y comando. Marca finalizados.
pub(crate) fn apply_jobs_list(mut s: State) -> State {
    if s.bg_jobs.is_empty() {
        s.push_output(OutputLine::notice("(sin jobs en background)"));
        return s;
    }
    // Snapshot de los Arc para no retener el borrow de `s.bg_jobs`
    // mientras `push_output` toma `&mut s`.
    let jobs = s.bg_jobs.clone();
    for (i, arc) in jobs.iter().enumerate() {
        let (cmd, status) = match arc.lock() {
            Ok(g) => (
                g.command.clone(),
                if g.handle.is_finished() {
                    "done"
                } else {
                    "running"
                },
            ),
            Err(p) => {
                let g = p.into_inner();
                (
                    g.command.clone(),
                    if g.handle.is_finished() {
                        "done"
                    } else {
                        "running"
                    },
                )
            }
        };
        s.push_output(OutputLine::notice(format!("[{i}] {status}  {cmd}")));
    }
    s
}

/// Aplica `:term N` / `:stop N` / `:cont N` al job de índice `N`.
/// Stop/Cont son no-op en jobs sin `Killer` (remotos vía daemon).
pub(crate) fn apply_jobs_signal(mut s: State, rest: &str, sig: JobSignal) -> State {
    let idx: usize = match rest.trim().parse() {
        Ok(n) => n,
        Err(_) => {
            s.push_output(OutputLine::notice("uso: :term N | :stop N | :cont N"));
            return s;
        }
    };
    let Some(arc) = s.bg_jobs.get(idx).cloned() else {
        s.push_output(OutputLine::notice(format!("no hay job [{idx}]")));
        return s;
    };
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let acted = match sig {
        JobSignal::Term => match guard.killer.as_ref() {
            Some(k) => {
                k.term();
                true
            }
            None => {
                // Remoto: cancel via stream close.
                guard.handle.kill();
                true
            }
        },
        JobSignal::Stop => guard.killer.as_ref().map(|k| k.stop()).unwrap_or(false),
        JobSignal::Cont => guard.killer.as_ref().map(|k| k.cont()).unwrap_or(false),
    };
    let label = match sig {
        JobSignal::Term => "TERM",
        JobSignal::Stop => "STOP",
        JobSignal::Cont => "CONT",
    };
    drop(guard);
    s.push_output(OutputLine::notice(if acted {
        format!("[{idx}] SIG{label} enviado")
    } else {
        format!("[{idx}] no se pudo enviar SIG{label}")
    }));
    s
}

/// `:limit <MB>` — tope de captura de stdout por run. `0` = sin tope.
pub(crate) fn apply_capture_limit(mut s: State, rest: &str) -> State {
    match rest.trim().parse::<usize>() {
        Ok(mb) => {
            s.capture_limit_bytes = mb.saturating_mul(1024 * 1024);
            let msg = if mb == 0 {
                "captura sin tope".to_string()
            } else {
                format!("captura limitada a {mb} MB por comando")
            };
            s.push_output(OutputLine::notice(msg));
        }
        Err(_) => s.push_output(OutputLine::notice("uso: :limit <MB>  (0 = sin tope)")),
    }
    s
}

/// `:spill on|off` — volcar a disco la salida que excede el `:limit`.
pub(crate) fn apply_spill(mut s: State, rest: &str) -> State {
    let arg = rest.trim();
    let on = matches!(arg, "on" | "si" | "sí" | "1" | "true");
    let off = matches!(arg, "off" | "no" | "0" | "false");
    if !on && !off {
        s.push_output(OutputLine::notice("uso: :spill on|off"));
        return s;
    }
    s.spill = on;
    let note = match (on, s.capture_limit_bytes) {
        (true, 0) => "spill activado — pero sin `:limit <MB>` no tiene efecto",
        (true, _) => "spill activado — la salida excedente se vuelca a disco",
        (false, _) => "spill desactivado",
    };
    s.push_output(OutputLine::notice(note));
    s
}

/// `:save <nombre>` — guarda como grupo los comandos del historial desde el
/// último `:save` (excluyendo los meta-comandos `:`). Ejecutables por F1..F8.
pub(crate) fn save_group(mut s: State, rest: &str) -> State {
    let name = rest.trim().to_string();
    if name.is_empty() {
        s.push_output(OutputLine::notice(
            "uso: :save <nombre>  (agrupa los comandos desde el último :save)",
        ));
        return s;
    }
    let (lines, hist_len) = {
        let Ok(h) = s.history.lock() else {
            return s;
        };
        let entries = h.entries();
        // El propio `:save` ya entró al historial: lo excluimos junto con el
        // resto de meta-comandos `:`.
        let upto = entries.len().saturating_sub(1);
        let lines: Vec<String> = entries
            .get(s.group_anchor..upto)
            .unwrap_or(&[])
            .iter()
            .map(|e| e.line.clone())
            .filter(|l| !l.trim_start().starts_with(':'))
            .collect();
        (lines, entries.len())
    };
    if lines.is_empty() {
        s.push_output(OutputLine::notice(
            "nada que guardar — corré algún comando antes de `:save`",
        ));
        return s;
    }
    // El próximo grupo arranca desde acá.
    s.group_anchor = hist_len;
    // Reemplaza un grupo homónimo, si existe.
    let n = lines.len();
    if let Some(g) = s.groups.iter_mut().find(|g| g.name == name) {
        g.lines = lines;
    } else {
        s.groups.push(CommandGroup { name: name.clone(), lines });
    }
    let fkey = s
        .groups
        .iter()
        .position(|g| g.name == name)
        .map(|i| i + 1)
        .unwrap_or(0);
    s.push_output(OutputLine::notice(format!(
        "grupo «{name}» guardado ({n} comandos) — F{fkey} lo ejecuta"
    )));
    s
}

/// `:groups` — lista los grupos guardados con su tecla de función.
pub(crate) fn apply_groups_list(mut s: State) -> State {
    if s.groups.is_empty() {
        s.push_output(OutputLine::notice(
            "(sin grupos — `:save <nombre>` guarda los últimos comandos)",
        ));
        return s;
    }
    let rows: Vec<String> = s
        .groups
        .iter()
        .enumerate()
        .map(|(i, g)| format!("F{}  {}  ({} cmds)", i + 1, g.name, g.lines.len()))
        .collect();
    for r in rows {
        s.push_output(OutputLine::notice(r));
    }
    s
}

/// Reconstruye el stdout de un bloque (su card) uniendo las líneas
/// `Stdout` sin etapa — para alimentarlo como stdin de un reprocess.
pub(crate) fn gather_block_stdout(s: &State, block: u64) -> String {
    let mut out = String::new();
    for l in &s.output {
        if l.block == block && l.kind == OutputKind::Stdout && l.stage.is_none() {
            out.push_str(&l.text);
            out.push('\n');
        }
    }
    out
}

/// Índice de grupo (0-based) para F1..F8; `None` para cualquier otra tecla.
pub(crate) fn fkey_index(key: &Key) -> Option<usize> {
    match key {
        Key::Named(NamedKey::F1) => Some(0),
        Key::Named(NamedKey::F2) => Some(1),
        Key::Named(NamedKey::F3) => Some(2),
        Key::Named(NamedKey::F4) => Some(3),
        Key::Named(NamedKey::F5) => Some(4),
        Key::Named(NamedKey::F6) => Some(5),
        Key::Named(NamedKey::F7) => Some(6),
        Key::Named(NamedKey::F8) => Some(7),
        _ => None,
    }
}

/// Ejecuta el grupo de índice `idx` (0-based) como una sola línea
/// (`l1 && l2 && …`). No-op si no existe ese grupo.
pub(crate) fn run_group(s: State, idx: usize) -> State {
    let Some(joined) = s
        .groups
        .get(idx)
        .map(|g| g.lines.join(" && "))
        .filter(|j| !j.is_empty())
    else {
        return s;
    };
    let mut s = s;
    s.input.set_text(joined);
    run_submitted(s)
}

/// Variante de `start_run` que arranca como job background. La salida
/// se mergea al output buffer prefijada por `[N]`. Devuelve `s` con el
/// nuevo job en `bg_jobs`.
pub(crate) fn start_bg(mut s: State, line: String) -> State {
    let cwd_str = s.cwd.display().to_string();
    let (spec, _tui) = build_spec(&line, &cwd_str);
    // Background no soporta TUI (no le pintamos el grid; el panel
    // sería robado al foreground). Si la línea era TUI, la corremos
    // sin PTY igual — el binario podrá quejarse, pero al menos no
    // tira la UI.
    let bg_spec = if matches!(spec.exec, Exec::Pty { .. }) {
        let mut s2 = spec.clone();
        s2.exec = Exec::Shell {
            line: line.clone(),
            program: "bash".into(),
        };
        s2
    } else {
        spec
    };
    let handle = shuma_exec::run(&bg_spec);
    let killer = handle.killer();
    let idx = s.bg_jobs.len();
    // Cada job de fondo vive en SU propia card (bloque propio). Sin esto
    // su salida se intercalaba en la card del comando de foreground.
    let bg_block = s.open_block();
    s.push_in_block(bg_block, OutputLine::prompt(format!("[{idx}] $ {line} &")));
    let active = ActiveRun {
        handle: BackendHandle::Local(handle),
        killer: Some(killer),
        command: line,
        tui: None,
        block: bg_block,
    };
    s.bg_jobs.push(Arc::new(Mutex::new(active)));
    s
}

pub(crate) fn start_run(mut s: State, line: String) -> State {
    let cwd_str = s.cwd.display().to_string();
    let (mut spec, tui) = build_spec(&line, &cwd_str);
    // Config de captura y reprocess sólo aplican a runs no-PTY (los TUI
    // capturan a vt100, no a buffer, y no consumen stdin reprocesado).
    if tui.is_none() {
        spec.capture_limit = s.capture_limit_bytes;
        spec.spill_path = (s.spill && s.capture_limit_bytes > 0).then(|| {
            std::env::temp_dir().join(format!(
                "shuma-spill-{}-{}.log",
                std::process::id(),
                s.current_block
            ))
        });
        // Reprocess armado: el stdout del bloque fuente alimenta el stdin.
        if let Some(src) = s.reprocess_source.take() {
            let data = gather_block_stdout(&s, src);
            if !data.is_empty() {
                spec.stdin_data = Some(data);
            }
        }
    } else {
        // Un run TUI desarma cualquier reprocess pendiente (no aplica).
        s.reprocess_source = None;
    }
    // Registramos la intención antes de hacer spawn — si el spawn
    // remoto falla, igual queda el nodo `%cN` con status `Failed`
    // marcado más abajo (vía el RunEvent::Failed que retorna el
    // backend). El lienzo refleja el intento.
    s.current_run_node = Some(s.intent_graph.record(line.clone()));
    s.current_run_bytes = 0;
    // El prompt de este run ya abrió su bloque (current_block); fijamos
    // que TODA su salida —drenada en ticks futuros— vaya a esa card.
    let run_block = s.current_block;
    let active = match &s.source {
        Source::Local => {
            // Camino histórico — exec directo sobre esta máquina.
            let handle = shuma_exec::run(&spec);
            let killer = handle.killer();
            ActiveRun {
                handle: BackendHandle::Local(handle),
                killer: Some(killer),
                command: line,
                tui,
                block: run_block,
            }
        }
        Source::Daemon { socket, .. } => {
            let sock = socket
                .clone()
                .unwrap_or_else(shuma_protocol::default_socket_path);
            // PTY remoto full-duplex: conservamos la `TuiSession` para
            // pintar el terminal localmente; las teclas/resize viajan al
            // daemon por el asa remota.
            if tui.is_some() {
                match shuma_remote_exec::run_pty(&spec, &sock) {
                    Ok(h) => ActiveRun {
                        handle: BackendHandle::Remote(h),
                        killer: None,
                        command: line,
                        tui,
                        block: run_block,
                    },
                    Err(e) => {
                        s.push_output(OutputLine::notice(format!("✘ daemon pty: {e}")));
                        fail_pending_intent(&mut s);
                        return s;
                    }
                }
            } else {
                match shuma_remote_exec::run(&spec, &sock) {
                    Ok(h) => ActiveRun {
                        handle: BackendHandle::Remote(h),
                        killer: None,
                        command: line,
                        tui: None,
                        block: run_block,
                    },
                    Err(e) => {
                        s.push_output(OutputLine::notice(format!("✘ daemon: {e}")));
                        fail_pending_intent(&mut s);
                        return s;
                    }
                }
            }
        }
        Source::DaemonTcp {
            addr,
            server_pub_hex,
            ..
        } => {
            // Identidad y pubkey del server hacen falta en ambos caminos
            // (PTY y no-PTY); las resolvemos una vez antes de ramificar.
            let kp = match load_or_create_identity() {
                Ok(kp) => kp,
                Err(e) => {
                    s.push_output(OutputLine::notice(format!("✘ identity: {e}")));
                    fail_pending_intent(&mut s);
                    return s;
                }
            };
            let server_pub = match parse_pub_hex(server_pub_hex) {
                Ok(p) => p,
                Err(e) => {
                    s.push_output(OutputLine::notice(format!("✘ server_pub_hex: {e}")));
                    fail_pending_intent(&mut s);
                    return s;
                }
            };
            if tui.is_some() {
                match shuma_remote_exec::run_pty_tcp(&spec, addr, kp, server_pub) {
                    Ok(h) => ActiveRun {
                        handle: BackendHandle::Remote(h),
                        killer: None,
                        command: line,
                        tui,
                        block: run_block,
                    },
                    Err(e) => {
                        s.push_output(OutputLine::notice(format!("✘ daemon tcp pty: {e}")));
                        fail_pending_intent(&mut s);
                        return s;
                    }
                }
            } else {
                match shuma_remote_exec::run_tcp(&spec, addr, kp, server_pub) {
                    Ok(h) => ActiveRun {
                        handle: BackendHandle::Remote(h),
                        killer: None,
                        command: line,
                        tui: None,
                        block: run_block,
                    },
                    Err(e) => {
                        s.push_output(OutputLine::notice(format!("✘ daemon tcp: {e}")));
                        fail_pending_intent(&mut s);
                        return s;
                    }
                }
            }
        }
        Source::Remote { .. } => {
            // SSH (matilda usa esta variante para otra cosa). El shell
            // no tiene un transporte SSH para comandos arbitrarios aún;
            // fallback a local con notice claro.
            s.push_output(OutputLine::notice(
                "shell vía SSH no implementado todavía — corro local",
            ));
            let handle = shuma_exec::run(&spec);
            let killer = handle.killer();
            ActiveRun {
                handle: BackendHandle::Local(handle),
                killer: Some(killer),
                command: line,
                tui,
                block: run_block,
            }
        }
    };
    s.running = Some(Arc::new(Mutex::new(active)));
    s
}

/// Cierra el nodo `%cN` registrado por `start_run` como fallido cuando
/// el spawn no llega a colocar el `RunHandle` (errores de socket/identity/
/// pub-hex/tcp). Sin esto el lienzo mostraría el comando como "running"
/// para siempre. Limpiá también el contador de bytes.
pub(crate) fn fail_pending_intent(s: &mut State) {
    if let Some(id) = s.current_run_node.take() {
        s.intent_graph.complete(id, false, 0);
    }
    s.current_run_bytes = 0;
}

/// Carga el `Keypair` del shell desde el archivo de identidad,
/// creando uno nuevo si no existe. Usa el path por defecto de
/// `shuma-link::Keypair::default_path()` (`~/.config/shuma/keys/identity`).
pub(crate) fn load_or_create_identity() -> Result<shuma_link::Keypair, String> {
    let path = shuma_link::Keypair::default_path()
        .ok_or_else(|| "no se pudo derivar el path de identidad".to_string())?;
    shuma_link::Keypair::load_or_generate(&path).map_err(|e| e.to_string())
}

pub(crate) fn parse_pub_hex(hex_str: &str) -> Result<shuma_link::PublicKey, String> {
    shuma_link::PublicKey::from_hex(hex_str).map_err(|e| e.to_string())
}

/// Si `line` es un pipe «simple» de ≥2 etapas —sólo `Command`/`Argument`/
/// `Flag`/`Pipe`/espacio, sin comillas, variables, redirecciones,
/// operadores, globs (`* ? [ ] { }`) ni `~`— devuelve sus etapas como
/// [`StageSpec`] para correrlo por `Exec::Direct`. Si no, `None` (cae a
/// `sh -c`, que sí absorbe esa sintaxis). Un único comando también cae a
/// `sh -c`: el modo directo sólo aporta cuando hay tubería que interceptar.
///
/// Conservador a propósito: `shuma_line::Stage` no recoge los `StringLit`
/// en `args`, así que un pipe con comillas debe ir al shell o perdería el
/// argumento citado.
pub(crate) fn simple_pipe_stages(line: &str) -> Option<Vec<StageSpec>> {
    use shuma_line::TokenKind::*;
    let tokens = shuma_line::tokenize(line, shuma_line::Dialect::Bash);
    let simple = !tokens.is_empty()
        && tokens.iter().all(|t| {
            matches!(t.kind, Command | Argument | Flag | Pipe | Whitespace)
                && !t.text.contains(['*', '?', '[', ']', '{', '}'])
                && !t.text.starts_with('~')
        });
    if !simple {
        return None;
    }
    let pipeline = shuma_line::split_pipeline(&tokens);
    if pipeline.stages.len() < 2 {
        return None;
    }
    let mut stages = Vec::with_capacity(pipeline.stages.len());
    for st in &pipeline.stages {
        // Una etapa sin comando (línea incompleta, p. ej. termina en `|`)
        // → al shell, que reporta el error de sintaxis como toca.
        let program = st.command.clone()?;
        stages.push(StageSpec {
            program,
            args: st.args.clone(),
        });
    }
    Some(stages)
}

/// Decide cómo lanzar `line`: si el primer token está en la allowlist
/// TUI (o el usuario lo prefijó con `:tui`), abre un PTY; si es un pipe
/// simple, lo corre directo con captura por etapa; si no, va por el shell
/// normal (streaming Stdout/Stderr).
pub(crate) fn build_spec(line: &str, cwd: &str) -> (CommandSpec, Option<TuiSession>) {
    // Prefijo explícito `:tui <comando>`.
    let (cmd_line, force_tui) = match line.strip_prefix(":tui ") {
        Some(rest) => (rest.trim(), true),
        None => (line, false),
    };
    let first_word = cmd_line.split_whitespace().next().unwrap_or("");
    let is_tui = force_tui || TUI_ALLOWLIST.contains(&first_word);
    if !is_tui {
        // Pipe «simple» (sólo comandos/args/flags y `|`, sin comillas,
        // variables, redirecciones, globs ni `~`): lo corremos directo
        // —conectando los procesos nosotros— y activamos la captura por
        // etapa (tee) para inspeccionar los intermedios en vivo. Cualquier
        // sintaxis que el modo directo no absorbe cae a `sh -c`.
        if let Some(stages) = simple_pipe_stages(line) {
            return (
                CommandSpec {
                    exec: Exec::Direct { stages },
                    cwd: cwd.to_string(),
                    capture_limit: 0,
                    spill_path: None,
                    stdin_data: None,
                    capture_stages: true,
                },
                None,
            );
        }
        return (CommandSpec::shell(line, cwd), None);
    }
    // Bajo PTY: parseamos en stages básicos por whitespace. No soporta
    // pipes ni redirecciones — un TUI fullscreen no los usa.
    let parts: Vec<String> = cmd_line.split_whitespace().map(String::from).collect();
    if parts.is_empty() {
        return (CommandSpec::shell(line, cwd), None);
    }
    let program = parts[0].clone();
    let args = parts[1..].to_vec();
    let spec = CommandSpec {
        exec: Exec::Pty {
            program,
            args,
            cols: PTY_COLS,
            rows: PTY_ROWS,
        },
        cwd: cwd.to_string(),
        capture_limit: 0,
        spill_path: None,
        stdin_data: None,
        capture_stages: false,
    };
    // Stage marker — usamos `parts` para sintaxis, no para ejecutar; el
    // Exec::Pty arma el spawn directo. La conversión a `StageSpec`
    // queda como guía visual del tooltip si después la queremos
    // exponer (hoy `Exec::Pty` no usa stages).
    let _ = StageSpec {
        program: parts[0].clone(),
        args: parts[1..].to_vec(),
    };
    // `program` ya se movió al `Exec::Pty`; usamos `parts[0]` (sigue vivo).
    (spec, Some(TuiSession::new(&parts[0], PTY_ROWS, PTY_COLS)))
}

pub(crate) fn drain_run(mut s: State) -> State {
    let Some(active_arc) = s.running.clone() else {
        return s;
    };
    let mut finished_with: Option<RunEvent> = None;
    // Bloque de ESTE run — toda su salida va a su card, aunque el usuario
    // haya tipeado otros comandos (que movieron `current_block`) mientras
    // corría.
    let run_block;
    {
        let mut guard = match active_arc.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        run_block = guard.block;
        // Resize del PTY si el rect del panel cambió desde el último
        // tick. Cell size aproximado: 7.5 px ancho × 16 px alto (12 pt
        // monoespacio en Llimphi default). Si el panel se redimensiona
        // el TUI hace SIGWINCH al child.
        let want_resize: Option<(u16, u16)> = if let Some(tui) = guard.tui.as_ref() {
            let (w, h) = match s.last_tui_rect.lock() {
                Ok(g) => *g,
                Err(p) => *p.into_inner(),
            };
            if w > 1.0 && h > 1.0 {
                let cols = ((w / 7.5).floor() as i32).clamp(20, 400) as u16;
                let rows = ((h / 16.0).floor() as i32).clamp(5, 200) as u16;
                if rows != tui.rows || cols != tui.cols {
                    Some((rows, cols))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some((rows, cols)) = want_resize {
            guard.handle.resize(rows, cols);
            if let Some(tui) = guard.tui.as_mut() {
                tui.set_size(rows, cols);
            }
        }
        let events = guard.handle.try_events();
        for ev in events {
            match ev {
                RunEvent::Stdout(line) => {
                    // +1 por el `\n` implícito de cada línea drenada.
                    s.current_run_bytes = s.current_run_bytes.saturating_add(line.len() as u64 + 1);
                    s.push_in_block(run_block, OutputLine::stdout(line));
                }
                RunEvent::StageStdout { stage, line } => {
                    // Salida de una etapa intermedia (tee). NO suma a
                    // `current_run_bytes` (el grafo cuenta la salida final);
                    // queda guardada para el desplegable de su etapa.
                    s.push_in_block(run_block, OutputLine::stage_stdout(stage, line));
                }
                RunEvent::Stderr(line) => {
                    s.current_run_bytes = s.current_run_bytes.saturating_add(line.len() as u64 + 1);
                    s.push_in_block(run_block, OutputLine::stderr(line));
                }
                RunEvent::Truncated => s.push_in_block(
                    run_block,
                    OutputLine::notice("… (salida truncada por límite de captura)"),
                ),
                RunEvent::Spilled(path) => s.push_in_block(
                    run_block,
                    OutputLine::notice(format!("… (resto volcado a {path})")),
                ),
                RunEvent::Bytes(bytes) => {
                    s.current_run_bytes = s.current_run_bytes.saturating_add(bytes.len() as u64);
                    if let Some(tui) = guard.tui.as_mut() {
                        tui.parser.process(&bytes);
                    }
                }
                ev @ (RunEvent::Exited(_) | RunEvent::Failed(_)) => {
                    finished_with = Some(ev);
                }
            }
        }
    }
    if let Some(ev) = finished_with {
        let ok = matches!(ev, RunEvent::Exited(0));
        let notice = match ev {
            RunEvent::Exited(0) => "✔ exit 0".to_string(),
            RunEvent::Exited(code) => format!("✘ exit {code}"),
            RunEvent::Failed(e) => format!("✘ no se pudo spawnear: {e}"),
            _ => unreachable!(),
        };
        s.push_in_block(run_block, OutputLine::notice(notice));
        // El comando terminado queda EXPANDIDO; sólo recede (se pliega) al
        // perderse en el historial cuando arranca uno nuevo (ver
        // `recede_previous_blocks` en `run_submitted`).
        // Cerrá el nodo del grafo de intenciones — el lienzo lo refleja
        // como verde/rojo en el próximo render.
        if let Some(id) = s.current_run_node.take() {
            s.intent_graph.complete(id, ok, s.current_run_bytes);
        }
        s.current_run_bytes = 0;
        s.running = None;
        // Si quedó algo en cola, arrancarlo ya — sin esperar otro Tick.
        if let Some(next) = s.queue.pop_front() {
            s = start_run(s, next);
        }
    }
    // Drenado de jobs background — cada uno aporta sus líneas
    // prefijadas por `[N]`. Los terminados se eliminan del Vec.
    s = drain_bg_jobs(s);
    s
}

/// Drena los `bg_jobs` y los limpia. Las líneas se prefijan `[N]`
/// para distinguir su origen.
pub(crate) fn drain_bg_jobs(mut s: State) -> State {
    let mut next_jobs: Vec<Arc<Mutex<ActiveRun>>> = Vec::with_capacity(s.bg_jobs.len());
    // Snapshot de los Arc: `push_output` toma `&mut s`, incompatible con
    // retener el borrow de `s.bg_jobs` durante el loop.
    let jobs = s.bg_jobs.clone();
    for arc in jobs.iter() {
        let mut keep = true;
        let mut finished: Option<RunEvent> = None;
        // Bloque propio del job — su salida vive en SU card, nunca en la
        // del foreground (era el bug del "output mezclado").
        let job_block;
        {
            let mut guard = match arc.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            job_block = guard.block;
            for ev in guard.handle.try_events() {
                match ev {
                    RunEvent::Stdout(line) => s.push_in_block(job_block, OutputLine::stdout(line)),
                    RunEvent::StageStdout { stage, line } => {
                        s.push_in_block(job_block, OutputLine::stage_stdout(stage, line))
                    }
                    RunEvent::Stderr(line) => s.push_in_block(job_block, OutputLine::stderr(line)),
                    RunEvent::Truncated => {
                        s.push_in_block(job_block, OutputLine::notice("… (truncada)"))
                    }
                    RunEvent::Spilled(path) => s.push_in_block(
                        job_block,
                        OutputLine::notice(format!("… (volcado a {path})")),
                    ),
                    RunEvent::Bytes(_) => {
                        // Background sin PTY — no debería emitir Bytes.
                    }
                    ev @ (RunEvent::Exited(_) | RunEvent::Failed(_)) => {
                        finished = Some(ev);
                    }
                }
            }
        }
        if let Some(ev) = finished {
            let notice = match ev {
                RunEvent::Exited(0) => "✔ exit 0".to_string(),
                RunEvent::Exited(code) => format!("✘ exit {code}"),
                RunEvent::Failed(e) => format!("✘ failed: {e}"),
                _ => unreachable!(),
            };
            s.push_in_block(job_block, OutputLine::notice(notice));
            keep = false;
        }
        if keep {
            next_jobs.push(arc.clone());
        }
    }
    s.bg_jobs = next_jobs;
    s
}

pub(crate) fn cancel_running(mut s: State) -> State {
    let mut run_block = s.current_block;
    if let Some(arc) = s.running.as_ref() {
        let guard = match arc.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        run_block = guard.block;
        // Local: SIGKILL al grupo entero — Ctrl-C debe doler en una UI.
        // Remoto: cerrar el stream — el daemon detecta EOF y mata al
        // hijo. La forma del notice no cambia.
        if let Some(killer) = guard.killer.as_ref() {
            killer.kill();
        } else {
            guard.handle.kill();
        }
        // El próximo Tick observará `RunEvent::Exited` y limpiará el handle.
    }
    s.push_in_block(run_block, OutputLine::notice("⏹ cancel (SIGKILL enviado)"));
    s
}

pub(crate) fn apply_cd(mut s: State, rest: &str) -> State {
    let target = if rest.trim().is_empty() {
        // `cd` sin args → HOME (convención bash/zsh).
        match std::env::var("HOME") {
            Ok(h) => PathBuf::from(h),
            Err(_) => {
                s.push_output(OutputLine::notice("cd: HOME no está definido"));
                return s;
            }
        }
    } else {
        let trimmed = rest.trim();
        let p = PathBuf::from(trimmed);
        if p.is_absolute() {
            p
        } else {
            s.cwd.join(p)
        }
    };
    match std::fs::canonicalize(&target) {
        Ok(canonical) => {
            if canonical.is_dir() {
                s.cwd = canonical;
            } else {
                s.push_output(OutputLine::notice(format!(
                    "cd: no es un directorio: {}",
                    target.display()
                )));
            }
        }
        Err(e) => {
            s.push_output(OutputLine::notice(format!("cd: {}: {e}", target.display())));
        }
    }
    s
}

pub(crate) fn split_first_word(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    if line.is_empty() {
        return None;
    }
    match line.find(char::is_whitespace) {
        Some(i) => Some((&line[..i], &line[i + 1..])),
        None => Some((line, "")),
    }
}

pub(crate) fn push_line(buf: &mut Vec<OutputLine>, line: OutputLine) {
    buf.push(line);
    let len = buf.len();
    if len > MAX_OUTPUT_LINES {
        buf.drain(0..len - MAX_OUTPUT_LINES);
    }
}
