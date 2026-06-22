use super::*;

mod find;
mod scroll;
mod surface;
mod completion;
mod history;
pub(crate) mod pty;
mod clipboard;
mod body_editor;
mod patterns;
mod run_exec;
mod builtins;
mod ssh_auth;
mod containers;
mod spec_builder;
mod utils;

pub(crate) use find::*;
pub(crate) use scroll::*;
pub(crate) use surface::*;
pub(crate) use completion::*;
pub(crate) use history::*;
pub(crate) use pty::*;
pub(crate) use clipboard::*;
pub(crate) use body_editor::*;
pub(crate) use patterns::*;
pub(crate) use run_exec::*;
pub(crate) use builtins::*;
pub(crate) use ssh_auth::*;
pub(crate) use containers::*;
pub(crate) use spec_builder::*;
pub(crate) use utils::*;

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
    // Movimientos con `shift` extienden la selección; sin `shift` la limpian.
    let movement = matches!(
        &ev.key,
        Key::Named(
            NamedKey::ArrowLeft | NamedKey::ArrowRight | NamedKey::Home | NamedKey::End
        )
    );
    if movement {
        if ev.modifiers.shift {
            line.begin_or_extend_selection();
        } else {
            line.clear_selection();
        }
    }
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
            // Si la barra de find del cuerpo de output está abierta, las
            // teclas van ahí (focus-grabbing). Esc cierra, Enter avanza,
            // Shift+Enter retrocede, Backspace borra, chars editan la query.
            if s.find.is_some() {
                return handle_find_key(s, &ev);
            }
            // Si el overlay de búsqueda está abierto, las teclas van ahí.
            if s.history_search.is_some() {
                return handle_search_key(s, &ev);
            }
            // Ctrl+F: abre la barra de find del cuerpo de output (sólo en
            // modo superficie; la barra se ignora en el camino viejo).
            if ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("f"))
            {
                s.find = Some(FindState::default());
                return s;
            }
            // Ctrl-A: seleccionar toda la línea del input.
            if ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("a"))
                && !s.input.is_empty()
            {
                s.input.select_all();
                s.input_edit_at_ms = now_unix_millis();
                return s;
            }
            // Ctrl-C: si hay selección en el input, copiarla (no cancela).
            // Si no, y hay run vivo, SIGTERM. Si no hay nada, no-op.
            if ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("c"))
            {
                if let Some(sel) = s.input.selected_text() {
                    set_clipboard(&sel);
                    return s;
                }
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
                    s.input.insert(&sanitize_paste(&text));
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
            // (Con shift extiende selección, así que no acepta.)
            if let Key::Named(NamedKey::ArrowRight) = ev.key {
                if !ev.modifiers.ctrl
                    && !ev.modifiers.shift
                    && s.input.cursor() == s.input.text().len()
                {
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
            // Volver a la "línea": el Enter arranca comandos nuevos, ya no
            // alimenta el stdin de un job.
            s.input_focus = None;
        }
        Msg::FocusJob(block) => {
            s.focused = true;
            // Sólo dirigimos el input a un comando que siga vivo; si ya
            // cerró, el foco se queda en la línea (no apuntamos a un muerto).
            if s.block_has_live_job(block) {
                s.input_focus = Some(block);
            } else if s.input_focus == Some(block) {
                s.input_focus = None;
            }
        }
        Msg::Clear => {
            s.clear_output();
        }
        Msg::ToggleBlock(id) => {
            if !s.collapsed.remove(&id) {
                s.collapsed.insert(id);
            }
        }
        Msg::PushNotice(text) => {
            s.push_output(OutputLine::notice(text));
        }
        Msg::ZoomBy(factor) => {
            if factor > 0.0 && factor.is_finite() {
                let old = s.font_zoom;
                s.font_zoom = (s.font_zoom * factor).clamp(0.5, 3.0);
                if (s.font_zoom - old).abs() > f32::EPSILON {
                    // Notice visible para que el usuario confirme que el
                    // atajo le llegó. Sin esto, si el render no respeta
                    // el cambio (path TUI fullscreen) parece no funcionar.
                    s.push_output(OutputLine::notice(format!(
                        "🔍 zoom {:.0}% → {:.0}%",
                        old * 100.0,
                        s.font_zoom * 100.0
                    )));
                }
            }
        }
        Msg::ZoomReset => {
            let old = s.font_zoom;
            s.font_zoom = 1.0;
            s.surf_scroll_x = 0.0;
            if (old - 1.0).abs() > f32::EPSILON {
                s.push_output(OutputLine::notice(format!(
                    "🔍 zoom {:.0}% → 100%",
                    old * 100.0
                )));
            }
        }
        Msg::ScrollHoriz(dx) => {
            s.surf_scroll_x = (s.surf_scroll_x + dx).max(0.0);
        }
        Msg::ToggleSection { block, idx } => {
            let key = (block, idx);
            if !s.section_collapsed.remove(&key) {
                s.section_collapsed.insert(key);
            }
        }
        Msg::SortSectionColumn { block, section, col } => {
            let key = (block, section);
            // Cicla: ninguno → asc(col) → desc(col) → ninguno;
            // si se clickeó otra columna, arranca asc en esa.
            let next = match s.section_sort.get(&key).copied() {
                None => Some((col, true)),
                Some((prev_col, asc)) if prev_col == col => {
                    if asc {
                        Some((col, false))
                    } else {
                        None
                    }
                }
                Some(_) => Some((col, true)),
            };
            match next {
                Some(v) => {
                    s.section_sort.insert(key, v);
                }
                None => {
                    s.section_sort.remove(&key);
                }
            }
        }
        Msg::Scroll(delta) => {
            s = apply_scroll_delta(s, delta);
            // Captura la última velocidad para el scroll inercial: el Tick
            // sigue aplicando el delta con decay hasta epsilon (Fase 5.2).
            s.surf_scroll_velocity = delta;
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
        Msg::AcceptChoreography(signature) => {
            s = accept_choreography(s, &signature);
        }
        Msg::DismissChoreography(signature) => {
            s.dismissed_choreo.insert(signature);
        }
        Msg::AcceptAlias(line) => {
            s = accept_alias(s, &line);
        }
        Msg::DismissAlias(line) => {
            s.dismissed_alias.insert(line);
        }
        Msg::AcceptDidYouMean(block) => {
            if let Some(corregida) = s.did_you_mean.remove(&block) {
                s.input.set_text(&corregida);
            }
        }
        Msg::InsertBlockRef(block) => {
            // Apila la ref al final de lo ya tipeado: `grep error ` + `%c12`.
            let actual = s.input.text().to_string();
            let sep = if actual.is_empty() || actual.ends_with(' ') {
                ""
            } else {
                " "
            };
            s.input.set_text(&format!("{actual}{sep}%c{block} "));
        }
        Msg::CopyCommandBlock(block) => {
            copy_command_block(&s, block);
        }
        Msg::Tick => {
            s = drain_run(s);
            // Scroll inercial: si quedó velocidad de la última entrada del
            // usuario, aplicar un paso y decaer por fricción (Fase 5.2 del
            // SDD-TERMINAL). Hitting bottom (re-pin) detiene la inercia.
            s = step_scroll_inertia(s);
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
        Msg::TuiMouseClick { button, lx, ly, rect_w, rect_h } => {
            forward_tui_click_to_pty(&s, button, lx, ly, rect_w, rect_h);
        }
        Msg::TuiMouseWheel { dy, lx, ly, rect_w, rect_h } => {
            forward_tui_wheel_to_pty(&s, dy, lx, ly, rect_w, rect_h);
        }
        Msg::SurfSelectDrag { phase, dx, dy, ax, ay } => {
            s = apply_surf_select_drag(s, phase, dx, dy, ax, ay);
        }
        Msg::SurfClearSelection => {
            s.surf_selection = None;
            s.surf_selecting = false;
        }
        Msg::SurfCopySelection => {
            copy_surf_selection(&s);
        }
        Msg::SurfDoubleClick { lx, ly, rect_w, rect_h } => {
            // Auto-detect de triple-click: si llega otro double-click
            // dentro de ~350 ms del previo, lo tratamos como triple
            // (select-line). Paridad con xterm: tap-tap = word,
            // tap-tap-tap-tap = line. La ventana de 350 ms cubre clicks
            // humanos sin pisar interacciones reales.
            const TRIPLE_WINDOW_MS: u64 = 350;
            let now = now_unix_millis();
            let recent = now.saturating_sub(s.surf_last_dblclick_ms) < TRIPLE_WINDOW_MS;
            s.surf_last_dblclick_ms = now;
            if recent {
                s = apply_surf_triple_click(s, lx, ly);
            } else {
                s = apply_surf_double_click(s, lx, ly, rect_w, rect_h);
            }
        }
        Msg::SurfOpenMenu { x, y } => {
            s.surf_menu = Some((x, y));
        }
        Msg::SurfMenuDismiss => {
            s.surf_menu = None;
        }
        Msg::SurfMenuPick(idx) => {
            s = apply_surf_menu_pick(s, idx);
        }
        Msg::FindOpen => {
            s.find = Some(FindState::default());
        }
        Msg::FindClose => {
            if s.find.as_ref().and_then(|f| f.current).is_some() {
                // Si la selección era el match resaltado, la limpiamos al
                // cerrar — un Esc no debería dejar selección residual.
                s.surf_selection = None;
            }
            s.find = None;
        }
        Msg::FindChar(c) => {
            s = apply_find_edit(s, |q| q.push(c));
        }
        Msg::FindBackspace => {
            s = apply_find_edit(s, |q| {
                q.pop();
            });
        }
        Msg::FindNext => {
            s = step_find(s, true);
        }
        Msg::FindPrev => {
            s = step_find(s, false);
        }
        Msg::FindToggleCase => {
            if let Some(f) = s.find.as_mut() {
                f.case_insensitive = !f.case_insensitive;
            }
            s = recompute_find(s);
        }
        Msg::LlmResult { kind, ok, text } => {
            s.llm_inflight = false;
            if !ok {
                s.push_output(OutputLine::notice(format!("🜲 llm · {text}")));
            } else {
                match kind {
                    LlmKind::Command => {
                        // Una sola línea, sin backticks/markdown que el modelo
                        // pueda colar. Va al input — el usuario revisa y Enter.
                        let line = text
                            .lines()
                            .map(|l| l.trim().trim_matches('`'))
                            .find(|l| !l.is_empty())
                            .unwrap_or("")
                            .to_string();
                        if line.is_empty() {
                            s.push_output(OutputLine::notice("🜲 llm · sin propuesta"));
                        } else {
                            s.input.set_text(&line);
                            s.focused = true;
                            s.push_output(OutputLine::notice(
                                "🜲 llm · propuesta en el input — revisá y Enter (no se ejecutó)",
                            ));
                        }
                    }
                    LlmKind::Text => {
                        for l in text.lines() {
                            s.push_output(OutputLine::notice(format!("🜲 {l}")));
                        }
                    }
                }
            }
        }
    }
    s
}

pub(crate) fn push_line(buf: &mut Vec<OutputLine>, line: OutputLine) {
    buf.push(line);
    let len = buf.len();
    if len > MAX_OUTPUT_LINES {
        buf.drain(0..len - MAX_OUTPUT_LINES);
    }
}
