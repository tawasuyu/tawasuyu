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
        Msg::BodyPointer { block, ev } => {
            s = apply_body_pointer(s, block, ev);
        }
        Msg::CopyBody(block) => {
            copy_body_selection(&s, block);
        }
        Msg::BodyDoubleClick { block, x, y } => {
            s = apply_body_double_click(s, block, x, y);
        }
        Msg::OpenBodyMenu { x, y } => {
            // Bloque objetivo: el que el usuario seleccionó, o el más reciente
            // con cuerpo. Si no hay ninguno, no abrimos (nada que copiar).
            if let Some(block) = menu_target_block(&s) {
                s.body_menu = Some((x, y, block));
            }
        }
        Msg::BodyMenuDismiss => {
            s.body_menu = None;
        }
        Msg::BodyMenuPick(idx) => {
            s = apply_body_menu_pick(s, idx);
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
    }
    s
}

/// Edita la query y re-busca. Resetea `current` al primer match (lo más
/// natural cuando uno está tipeando — el resaltado salta a la primera
/// ocurrencia conforme se escribe).
fn apply_find_edit(mut s: State, mutate: impl FnOnce(&mut String)) -> State {
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
fn recompute_find(mut s: State) -> State {
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
fn step_find(mut s: State, forward: bool) -> State {
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
fn apply_current_match(mut s: State, snap: &crate::SurfLayout) -> State {
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

/// Actualiza la selección viva del cuerpo de output en modo superficie. El
/// primer Move arranca (`anchor = head = point_at(ax, ay)`); los siguientes
/// extienden (`head = point_at(acc)`); End deja la selección fijada pero
/// `surf_selecting = false` para que un próximo Move arranque limpio.
/// Aplica un delta de scroll a la superficie, manteniendo el invariante de
/// anclaje (Fase 5.0). Devuelve `s` con `scroll_px` / `surf_scroll_anchor`
/// actualizados. NO toca `surf_scroll_velocity` — eso lo hacen los callers
/// (`Msg::Scroll` la captura, `step_scroll_inertia` la decae).
fn apply_scroll_delta(mut s: State, delta: f32) -> State {
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
fn step_scroll_inertia(mut s: State) -> State {
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

fn apply_surf_select_drag(
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
fn copy_surf_selection(s: &State) {
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
                    s.completion_source = crate::completion_source_for(&s.source, &s.cwd);
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

/// `true` si hay un `ActiveRun` con PTY vivo. Las teclas van al stdin del
/// PTY mientras esto sea cierto (el programa es interactivo, esté o no en
/// pantalla completa). El **render** en cambio sigue a [`is_tui_fullscreen`].
///
/// **No-blocking**: usa `try_lock`. Si el lector del PTY tiene el mutex en
/// este instante (drenando una ráfaga grande de output, p. ej. `ls -alR`),
/// volvemos `false` antes que pasmar el thread de pintura: pintar `false`
/// un frame de más es indistinguible de "todavía no llegó el dato", pero
/// bloquear el render durante una ráfaga deja la pantalla negra.
pub(crate) fn is_tui_active(s: &State) -> bool {
    let Some(arc) = s.running.as_ref() else {
        return false;
    };
    let g = match arc.try_lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    g.tui.is_some()
}

/// `true` si el PTY vivo entró a **alternate screen** (`ESC[?1049h`) — la
/// señal dura de una app TUI de pantalla completa (vim, htop, less, man…).
/// Es lo que decide pintar el panel full-screen (grid/vim) en vez de las
/// líneas. Al salir del alt-screen (`ESC[?1049l`) vuelve a modo líneas.
///
/// Misma política `try_lock` que [`is_tui_active`]: ante contienda, `false`
/// — el render cae al pane de cards (que sí usa data ya volcada a
/// `state.output`) y nunca se pasma esperando al lector del PTY.
pub(crate) fn is_tui_fullscreen(s: &State) -> bool {
    let Some(arc) = s.running.as_ref() else {
        return false;
    };
    let g = match arc.try_lock() {
        Ok(g) => g,
        Err(_) => return false,
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

/// Limpia texto pegado al editor de línea. A diferencia del shell GPUI
/// (que colapsaba todo a una línea unida por `; `), este input es
/// **multilínea** —editar construcciones abiertas, pegar scripts—, así que
/// los saltos se **preservan**. Lo que sí hacemos:
///
/// - normalizar `\r\n` y `\r` a `\n` (pastes de Windows / terminales),
/// - tab → espacio (el line editor no tabula columnas),
/// - descartar caracteres de control peligrosos (ESC, BEL, …) que un paste
///   de terminal puede arrastrar y que corromperían el render del input,
/// - recortar **un** salto final, para que pegar `"ls -la\n"` no deje una
///   línea vacía colgando bajo el comando.
pub(crate) fn sanitize_paste(s: &str) -> String {
    let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
    let cleaned: String = normalized
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| *c == '\n' || !c.is_control())
        .collect();
    cleaned
        .strip_suffix('\n')
        .map(str::to_string)
        .unwrap_or(cleaned)
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

/// Convierte un click sobre el panel TUI en bytes xterm-mouse y los manda
/// al PTY del run activo. No-op si el programa no habilitó mouse
/// (`MouseProtocolMode::None`) o no hay TUI. Para modos que reportan
/// release (VT200/ButtonMotion/AnyMotion), encadena Press + Release en una
/// sola escritura — los TUIs (vim/htop/btop) los procesan en ese orden.
pub(crate) fn forward_tui_click_to_pty(
    s: &State,
    button: u8,
    lx: f32,
    ly: f32,
    rect_w: f32,
    rect_h: f32,
) {
    use crate::mouse_xterm::{encode, local_to_cell, XBtn, XPhase};
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
    let mode = screen.mouse_protocol_mode();
    if matches!(mode, vt100::MouseProtocolMode::None) {
        return;
    }
    let encoding = screen.mouse_protocol_encoding();
    let btn = match button {
        0 => XBtn::Left,
        1 => XBtn::Middle,
        2 => XBtn::Right,
        _ => return,
    };
    let (col, row) = local_to_cell(lx, ly, rect_w, rect_h, tui.cols, tui.rows);
    let mut payload: Vec<u8> = Vec::new();
    if let Some(b) = encode(mode, encoding, btn, XPhase::Press, col, row) {
        payload.extend_from_slice(&b);
    }
    if let Some(b) = encode(mode, encoding, btn, XPhase::Release, col, row) {
        payload.extend_from_slice(&b);
    }
    if !payload.is_empty() {
        guard.handle.write_input(payload);
    }
}

/// Convierte un tick de rueda sobre el panel TUI en eventos xterm-mouse
/// (button 4 = arriba, button 5 = abajo) y los manda al PTY. Emite tantos
/// "press" como ticks lógicos (ceil de `|dy|`) — la rueda no tiene release
/// en xterm. No-op si el programa no habilitó mouse o no hay TUI.
pub(crate) fn forward_tui_wheel_to_pty(
    s: &State,
    dy: f32,
    lx: f32,
    ly: f32,
    rect_w: f32,
    rect_h: f32,
) {
    use crate::mouse_xterm::{encode, local_to_cell, XBtn, XPhase};
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
    let mode = screen.mouse_protocol_mode();
    if matches!(mode, vt100::MouseProtocolMode::None) {
        return;
    }
    let encoding = screen.mouse_protocol_encoding();
    let btn = if dy > 0.0 { XBtn::WheelUp } else { XBtn::WheelDown };
    let ticks = dy.abs().ceil() as u32;
    if ticks == 0 {
        return;
    }
    let (col, row) = local_to_cell(lx, ly, rect_w, rect_h, tui.cols, tui.rows);
    let mut payload: Vec<u8> = Vec::new();
    for _ in 0..ticks.min(8) {
        if let Some(b) = encode(mode, encoding, btn, XPhase::Press, col, row) {
            payload.extend_from_slice(&b);
        }
    }
    if !payload.is_empty() {
        guard.handle.write_input(payload);
    }
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
    // Si ya hay un comando vivo y el usuario NO terminó con `&` (que
    // fuerza bg), interpretamos el Enter como respuesta al stdin del
    // running: típico apt Y/n, sudo password, prompts custom. Escribimos
    // `<line>\n` al stdin. El usuario aún puede arrancar un bg paralelo
    // tipeando `cmd &`.
    if s.running.is_some() && !trimmed.ends_with('&') {
        let bytes = {
            let mut v = line.clone().into_bytes();
            v.push(b'\n');
            v
        };
        if let Some(active_arc) = s.running.clone() {
            if let Ok(guard) = active_arc.lock() {
                guard.handle.write_input(bytes);
            }
        }
        // Echo discreto de lo enviado para que el usuario vea qué tipeó.
        s.push_output(OutputLine::notice(format!("← {line}")));
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

    // Expansión de aliases del `.shumarc`: la primera palabra se reemplaza si
    // está declarada. Los meta-comandos del shell (`:save`, `:limit`, …) se
    // dejan SIN expandir para que el rc no pueda secuestrarlos. Lo que se
    // muestra (`$ trimmed`) y se persiste en el historial es lo que el usuario
    // tipeó; lo que se ejecuta es `exec_line` ya resuelto.
    let exec_line = if trimmed.starts_with(':') {
        trimmed.clone()
    } else {
        s.config.expand_aliases(&trimmed).into_owned()
    };

    // Builtins primero — no spawnean proceso, corren aunque haya run vivo.
    if let Some((cmd, rest)) = split_first_word(&exec_line) {
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
            ":kill" => return apply_jobs_signal(s, rest, JobSignal::Kill),
            ":stop" => return apply_jobs_signal(s, rest, JobSignal::Stop),
            ":cont" => return apply_jobs_signal(s, rest, JobSignal::Cont),
            ":limit" => return apply_capture_limit(s, rest),
            ":spill" => return apply_spill(s, rest),
            ":scrollback" => return apply_scrollback(s, rest),
            ":save" => return save_group(s, rest),
            ":groups" => return apply_groups_list(s),
            _ => {}
        }
    }

    // Sufijo `&` (con espacios opcionales antes) → background. El
    // background siempre arranca, sin encolar; no hay límite.
    if let Some(stripped) = exec_line.strip_suffix('&') {
        let cmd = stripped.trim_end().to_string();
        if cmd.is_empty() {
            return s;
        }
        return start_bg(s, cmd);
    }

    // Comando externo foreground. Si ya hay uno corriendo, el nuevo
    // arranca en background paralelo (job nuevo) — no encolar. Esto
    // evita que un comando colgado (fastfetch, ssh, etc.) bloquee el
    // shell. El usuario ve un notice "▶ job N en background" y puede
    // seguir tipeando; `:jobs` los lista, `:kill N` los mata, `:fg N`
    // los traería al foreground (TODO).
    if s.running.is_some() {
        let cmd = exec_line.clone();
        s.push_output(OutputLine::notice(format!(
            "▶ corre en background (hay otro comando vivo) — {cmd}"
        )));
        return start_bg(s, exec_line);
    }
    start_run(s, exec_line)
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum JobSignal {
    /// SIGTERM — pedido cortés de terminar (`:term N`).
    Term,
    /// SIGKILL — fin inmediato e incondicional (`:kill N`).
    Kill,
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

/// Aplica `:term N` / `:kill N` / `:stop N` / `:cont N` al job de índice `N`.
/// Stop/Cont son no-op en jobs sin `Killer` (remotos vía daemon); Term/Kill
/// caen a cerrar el stream del daemon.
pub(crate) fn apply_jobs_signal(mut s: State, rest: &str, sig: JobSignal) -> State {
    let idx: usize = match rest.trim().parse() {
        Ok(n) => n,
        Err(_) => {
            s.push_output(OutputLine::notice("uso: :term N | :kill N | :stop N | :cont N"));
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
        JobSignal::Kill => match guard.killer.as_ref() {
            Some(k) => {
                k.kill();
                true
            }
            None => {
                // Remoto: el daemon no expone SIGKILL fino; cerrar el stream
                // es lo más fuerte que tenemos.
                guard.handle.kill();
                true
            }
        },
        JobSignal::Stop => guard.killer.as_ref().map(|k| k.stop()).unwrap_or(false),
        JobSignal::Cont => guard.killer.as_ref().map(|k| k.cont()).unwrap_or(false),
    };
    let label = match sig {
        JobSignal::Term => "TERM",
        JobSignal::Kill => "KILL",
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

/// `:scrollback` (sin args): muestra el estado del scrollback persistente —
/// líneas en memoria + spilleadas + path del spill file. `:scrollback open`
/// abre el spill file con `$EDITOR` (o cae a `xdg-open`) para que el
/// usuario inspeccione el archive sin salir del shell.
pub(crate) fn apply_scrollback(mut s: State, rest: &str) -> State {
    let arg = rest.trim();
    let snap = match s.surf_history.lock() {
        Ok(g) => (g.len(), g.spilled_count(), g.spill_path()),
        Err(p) => {
            let g = p.into_inner();
            (g.len(), g.spilled_count(), g.spill_path())
        }
    };
    let (in_mem, in_spill, spill_path) = snap;
    match arg {
        "open" => {
            let Some(path) = spill_path else {
                s.push_output(OutputLine::notice(
                    ":scrollback open — el spill no está activo (ver [scrollback].spill = true en shumarc.toml)",
                ));
                return s;
            };
            let path_s = path.display().to_string();
            // Preferimos $EDITOR para inspección textual; si no, xdg-open.
            if let Ok(editor) = std::env::var("EDITOR") {
                spawn_detached(&editor, &[path_s.as_str()]);
            } else {
                spawn_detached("xdg-open", &[path_s.as_str()]);
            }
            s.push_output(OutputLine::notice(format!("abriendo {path_s}…")));
        }
        "" => {
            // Estado: líneas en memoria + en spill + path.
            s.push_output(OutputLine::notice(format!(
                "scrollback: {in_mem} líneas en memoria, {in_spill} archivadas"
            )));
            if let Some(p) = spill_path {
                s.push_output(OutputLine::notice(format!(
                    "spill: {} ({:?})",
                    p.display(),
                    p.metadata().ok().map(|m| m.len()).unwrap_or(0)
                )));
                s.push_output(OutputLine::notice(
                    "abrí con `:scrollback open` o `cat`-éalo desde otra shell",
                ));
            } else {
                s.push_output(OutputLine::notice(
                    "spill no activo — activá con [scrollback].spill = true en shumarc.toml",
                ));
            }
        }
        a if a.starts_with("grep ") => {
            let pattern = a[5..].trim();
            if pattern.is_empty() {
                s.push_output(OutputLine::notice("uso: :scrollback grep <patrón>"));
                return s;
            }
            s = apply_scrollback_grep(s, pattern);
        }
        _ => {
            s.push_output(OutputLine::notice("uso: :scrollback [open | grep <patrón>]"));
        }
    }
    s
}

/// Busca un substring literal en TODO el archive del scrollback — tanto
/// las líneas en memoria como las del spill file. Útil cuando el usuario
/// sabe que algo apareció hace mucho y ya está fuera del cache visible.
/// Reporta los hits como notices con su `global_id` 1-based.
/// Case-sensitive (literal); el usuario usa el `:scrollback open` + el
/// search de `$EDITOR` para casos más complejos.
fn apply_scrollback_grep(mut s: State, pattern: &str) -> State {
    let hist = match s.surf_history.lock() {
        Ok(g) => g.clone(),
        Err(p) => p.into_inner().clone(),
    };
    let mut hits: Vec<(u64, String)> = Vec::new();
    let total_spilled = hist.spilled_count();
    // Spilled: leer una por una. Lentos en archives enormes; el caller
    // que necesite más velocidad usa el editor con su grep.
    for id in 0..total_spilled as u64 {
        if let Ok(Some(text)) = hist.read_spilled(id) {
            if text.contains(pattern) {
                hits.push((id, text));
            }
        }
        // Cap defensivo: nunca más de 1000 hits para no saturar el output.
        if hits.len() >= 1000 {
            break;
        }
    }
    // In-memory: las líneas vigentes (índices 0..len → global ids
    // dropped+0..dropped+len).
    let in_mem = hist.len();
    let dropped = hist.dropped();
    for i in 0..in_mem {
        if hits.len() >= 1000 {
            break;
        }
        if let Some(text) = hist.line(i) {
            if text.contains(pattern) {
                hits.push((dropped + i as u64, text.to_string()));
            }
        }
    }
    if hits.is_empty() {
        s.push_output(OutputLine::notice(format!(
            "grep: sin hits para `{pattern}` ({} líneas revisadas)",
            total_spilled + in_mem
        )));
        return s;
    }
    s.push_output(OutputLine::notice(format!(
        "grep: {} hit{} para `{pattern}`",
        hits.len(),
        if hits.len() == 1 { "" } else { "s" }
    )));
    for (id, text) in hits.iter().take(50) {
        s.push_output(OutputLine::notice(format!("  [{}] {}", id + 1, text)));
    }
    if hits.len() > 50 {
        s.push_output(OutputLine::notice(format!(
            "  … y {} más (cap del builtin a 50 visibles)",
            hits.len() - 50
        )));
    }
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
        Source::Remote { host, user, port, .. } => {
            // Shell SSH (russh vía shuma-remote-exec::run_ssh). v1: cada comando
            // es un `ssh exec` (sin PTY/streaming). La auth sale de hosts.json.
            match resolve_ssh_auth(host, user) {
                Ok(auth) => {
                    let cwd = s.cwd.display().to_string();
                    let handle =
                        shuma_remote_exec::run_ssh(&line, &cwd, host, user, *port, auth);
                    ActiveRun {
                        handle: BackendHandle::Remote(handle),
                        killer: None,
                        command: line,
                        tui: None,
                        block: run_block,
                    }
                }
                Err(e) => {
                    s.push_output(OutputLine::notice(format!("✘ SSH: {e}")));
                    fail_pending_intent(&mut s);
                    return s;
                }
            }
        }
        Source::Container { engine, name, .. } => {
            // Envuelve el spec en `<engine> exec` contra el contenedor. El wrap
            // ya leyó `spec.cwd` como cwd INTERIOR (lo inyecta como `cd` adentro).
            let mut wrapped = wrap_spec_for_container(spec.clone(), engine, name);
            // El cwd del SPAWN en el host debe ser válido y accesible: el cwd
            // interior (p.ej. `/root`) no existe/no es accesible en el host y
            // haría fallar el spawn. `/` siempre sirve; el chroot/bind manda.
            wrapped.cwd = "/".to_string();
            let handle = shuma_exec::run(&wrapped);
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

// --- Auth SSH para `Source::Remote` -----------------------------------------
// Espejo MÍNIMO del schema de `~/.config/shuma/hosts.json` (lo escribe el
// chasis vía `hosts.rs`): sólo los campos que necesita el transporte SSH.

#[derive(serde::Deserialize)]
struct HostEntry {
    host: String,
    #[serde(default)]
    user: String,
    #[serde(default = "host_default_port")]
    port: u16,
    #[serde(default)]
    auth: HostAuthJson,
}

fn host_default_port() -> u16 {
    22
}

#[derive(serde::Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HostAuthJson {
    #[default]
    Password,
    Key {
        path: String,
    },
}

fn hosts_json_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("shuma").join("hosts.json"))
}

/// Corre el binario askpass (`SHUMA_ASKPASS`/`SSH_ASKPASS`) con `prompt` y
/// devuelve lo que imprime en stdout (la contraseña/passphrase). `None` si no
/// hay askpass configurado o el usuario canceló.
fn run_askpass(prompt: &str) -> Option<String> {
    let bin = std::env::var_os("SHUMA_ASKPASS").or_else(|| std::env::var_os("SSH_ASKPASS"))?;
    let out = std::process::Command::new(bin).arg(prompt).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout)
        .trim_end_matches(['\n', '\r'])
        .to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Resuelve el método de auth para `host`/`user` leyendo `hosts.json`. Clave
/// (PEM) → `SshAuth::Key`; contraseña → askpass al conectar.
fn resolve_ssh_auth(
    host: &str,
    user: &str,
) -> Result<shuma_remote_exec::SshAuth, String> {
    let path = hosts_json_path().ok_or("no se pudo ubicar hosts.json")?;
    let txt = std::fs::read_to_string(&path)
        .map_err(|e| format!("no pude leer {}: {e}", path.display()))?;
    let entries: Vec<HostEntry> =
        serde_json::from_str(&txt).map_err(|e| format!("hosts.json inválido: {e}"))?;
    let entry = entries
        .iter()
        .find(|h| h.host == host && (h.user == user || h.user.is_empty()))
        .or_else(|| entries.iter().find(|h| h.host == host))
        .ok_or_else(|| format!("no hay host guardado para {host} — gestioná hosts"))?;
    let _ = entry.port;
    match &entry.auth {
        HostAuthJson::Key { path } => Ok(shuma_remote_exec::SshAuth::Key {
            path: PathBuf::from(path),
            passphrase: None,
        }),
        HostAuthJson::Password => {
            let pw = run_askpass(&format!("Contraseña SSH para {user}@{host}:")).ok_or(
                "auth por contraseña: configurá SHUMA_ASKPASS/SSH_ASKPASS o usá una clave (PEM)",
            )?;
            Ok(shuma_remote_exec::SshAuth::Password(pw))
        }
    }
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
/// Inserta `-A` después de `sudo` cuando el usuario no lo puso, para que
/// sudo dispare `SUDO_ASKPASS` (popup) en vez de quedar colgado leyendo
/// stdin del PTY. Respeta `-A`, `-S`, `--askpass`, `--stdin` ya presentes.
/// Sólo toca la primera ocurrencia al principio del line — pipes / `&&` /
/// `;` van por su cuenta (el shell del PTY los maneja).
fn inject_askpass(line: &str) -> String {
    let trimmed = line.trim_start();
    let lead_len = line.len() - trimmed.len();
    let Some(rest_after_sudo) = trimmed.strip_prefix("sudo") else {
        return line.to_string();
    };
    // Exigir que `sudo` sea palabra completa (siguiente char espacio / EOL).
    let next = rest_after_sudo.chars().next();
    if !matches!(next, None | Some(' ') | Some('\t')) {
        return line.to_string();
    }
    // Heurística simple: si los tokens del comando contienen -A/-S/--askpass/
    // --stdin antes de cualquier `;|&` o salto de pipe, dejarlo como está.
    for tok in rest_after_sudo.split_whitespace() {
        if tok == "-A" || tok == "-S" || tok == "--askpass" || tok == "--stdin" {
            return line.to_string();
        }
        // Llegamos a un argumento que no es flag → dejamos de buscar (es
        // el comando ejecutado por sudo y sus flags son suyos).
        if !tok.starts_with('-') {
            break;
        }
    }
    let lead = &line[..lead_len];
    format!("{lead}sudo -A{rest_after_sudo}")
}

/// Envuelve `spec` en la invocación del **engine de aislamiento** elegido.
///
/// - `engine = "podman"` / `"docker"`: `name` es el nombre del container ya
///   creado; corremos `<engine> exec -i <name> bash -c <line>`.
/// - `engine = "bwrap"`: `name` es el PATH al rootfs en disco
///   (`~/.local/share/shuma/rootfs/<distro>`); corremos `bwrap` con los
///   binds estándar y `bash -c <line>` adentro. No requiere config
///   global — sólo el binario `bwrap` instalado.
///
/// En ambos casos el proceso hijo que ve `shuma-exec` sigue siendo local —
/// reusamos la maquinaria de PTY / capture / kill de `Source::Local`.
fn wrap_spec_for_container(mut spec: CommandSpec, engine: &str, name: &str) -> CommandSpec {
    if engine == "unshare" {
        return wrap_spec_for_unshare(spec, name);
    }
    if engine == "bwrap" {
        return wrap_spec_for_bwrap(spec, name);
    }
    let eng = engine.to_string();
    let nm = name.to_string();
    spec.exec = match spec.exec {
        Exec::Shell { line, program } => {
            // bash local que dispara `engine exec` con `program -c "line"`
            // adentro. Mantenemos Exec::Shell para preservar captura por
            // líneas (no PTY).
            let inner = format!(
                "{eng} exec -i {nm} {prog} -c {q}",
                eng = shell_quote(&eng),
                nm = shell_quote(&nm),
                prog = shell_quote(&program),
                q = shell_quote(&line),
            );
            Exec::Shell {
                line: inner,
                program: "bash".into(),
            }
        }
        Exec::Pty { program, args, cols, rows } => {
            // PTY local que ejecuta `engine exec -it name <program> <args...>`.
            let mut new_args = vec!["exec".to_string(), "-it".into(), nm, program];
            new_args.extend(args);
            Exec::Pty {
                program: eng,
                args: new_args,
                cols,
                rows,
            }
        }
        Exec::Direct { stages } => {
            // Reconstruimos la pipe como una sola line bash y la disparamos
            // dentro del contenedor; perdemos la captura de etapas (tee) —
            // tradeoff aceptable para el MVP del cableo container.
            let mut line = String::new();
            for (i, st) in stages.iter().enumerate() {
                if i > 0 {
                    line.push_str(" | ");
                }
                line.push_str(&shell_quote(&st.program));
                for a in &st.args {
                    line.push(' ');
                    line.push_str(&shell_quote(a));
                }
            }
            let inner = format!(
                "{eng} exec -i {nm} bash -c {q}",
                eng = shell_quote(&eng),
                nm = shell_quote(&nm),
                q = shell_quote(&line),
            );
            Exec::Shell {
                line: inner,
                program: "bash".into(),
            }
        }
    };
    spec
}

/// Script `sh` que recibe `$1 = rootfs_path` y `$2 = línea bash a correr`
/// y ejecuta el comando aislado por `unshare` + `chroot`. Monta `/proc`,
/// `/dev`, `/sys` y bind-mountea `/etc/resolv.conf` del host para que apt
/// y pacman alcancen la red. Sin `--unshare-net` por la misma razón.
///
/// El `|| true` después de cada `mount` evita que el script entero
/// aborte si el rootfs ya tenía algo montado (re-entry tras crash).
const UNSHARE_SCRIPT: &str = "\
mount -t proc proc \"$1/proc\" 2>/dev/null || true; \
mount --bind /dev \"$1/dev\" 2>/dev/null || true; \
mount --bind /sys \"$1/sys\" 2>/dev/null || true; \
mount --bind /etc/resolv.conf \"$1/etc/resolv.conf\" 2>/dev/null || true; \
exec chroot \"$1\" /bin/bash -c \"$2\"";

/// Variante de [`wrap_spec_for_container`] para `engine = "unshare"`. El
/// `rootfs_path` es un filesystem extraído en disco local; `unshare -r`
/// + `chroot` lo activan sin necesidad de root ni bwrap ni podman — sólo
/// requiere `util-linux` + `coreutils` (instalados en todo Linux moderno).
///
/// Funciona en distros con `kernel.unprivileged_userns_clone = 1` (default
/// en kernels >= 5.10 mayoritarios). Si está deshabilitado, el `unshare -r`
/// falla con "Operation not permitted" y el caller verá el stderr en el
/// notice.
fn wrap_spec_for_unshare(mut spec: CommandSpec, rootfs_path: &str) -> CommandSpec {
    fn base_args(rootfs: &str, inner_line: &str) -> Vec<String> {
        vec![
            "-r".into(),       // map root in user ns
            "-m".into(),       // mount ns (para mount -t proc etc.)
            "-u".into(),       // uts ns
            "-i".into(),       // ipc ns
            "-p".into(),       // pid ns
            "-f".into(),       // fork (necesario con -p)
            "--kill-child".into(), // los hijos mueren con el padre
            "--".into(),
            "/bin/sh".into(), "-c".into(), UNSHARE_SCRIPT.into(),
            "_".into(),                  // $0
            rootfs.to_string(),          // $1
            inner_line.to_string(),      // $2
        ]
    }
    let rootfs = rootfs_path.to_string();
    // Prefijo común para TODO comando dentro del contenedor: HOME del root y
    // `cd` al cwd interior que trackea shuma (`spec.cwd`). Sin esto el comando
    // corría en `/` con PWD heredado del host → `pwd`/`ls`/el prompt se
    // contradecían. `|| true` para no abortar si el dir no existe (el comando
    // igual reporta su propio error).
    let prelude = format!(
        "export HOME=/root; cd {} 2>/dev/null || true; ",
        shell_quote(&spec.cwd)
    );
    spec.exec = match spec.exec {
        Exec::Shell { line, program: _ } => {
            // No-TUI: corremos `unshare` como UNA etapa `Exec::Direct` para
            // capturar stdout/stderr por líneas y renderizarlas como bloques,
            // igual que un comando local. (Antes se forzaba `Exec::Pty`, pero
            // sin `TuiSession` el drenado descartaba los `Bytes` del PTY → el
            // comando corría sin mostrar NADA, con la card en verde/✘ sin
            // motivo. Los TUI fullscreen sí van por la rama `Exec::Pty` de
            // abajo, que sí trae su emulador.)
            let inner = format!("{prelude}{line}");
            let args = base_args(&rootfs, &inner);
            Exec::Direct { stages: vec![StageSpec { program: "unshare".into(), args }] }
        }
        Exec::Pty { program, args, cols, rows } => {
            // Para Exec::Pty (TUI fullscreen tipo vim) armamos el `bash -c`
            // con el program + args ya quoteados.
            let mut inner = prelude.clone();
            inner.push_str(&shell_quote(&program));
            for a in &args {
                inner.push(' ');
                inner.push_str(&shell_quote(a));
            }
            let args = base_args(&rootfs, &inner);
            Exec::Pty { program: "unshare".into(), args, cols, rows }
        }
        Exec::Direct { stages } => {
            let mut line = String::new();
            for (i, st) in stages.iter().enumerate() {
                if i > 0 {
                    line.push_str(" | ");
                }
                line.push_str(&shell_quote(&st.program));
                for a in &st.args {
                    line.push(' ');
                    line.push_str(&shell_quote(a));
                }
            }
            // Pipe simple → también por `Exec::Direct` (captura por líneas).
            let inner = format!("{prelude}{line}");
            let args = base_args(&rootfs, &inner);
            Exec::Direct { stages: vec![StageSpec { program: "unshare".into(), args }] }
        }
    };
    spec
}

/// Args base de bwrap para correr un comando dentro de `rootfs_path`. La
/// idea: aislar mount/pid/uts/ipc pero **compartir net** del host (para
/// que `apt update`, `pacman -Sy`, etc. lleguen al mundo). El `/work`
/// queda como bind del cwd del host cuando aplica.
fn bwrap_args(rootfs_path: &str) -> Vec<String> {
    let mut a: Vec<String> = vec![
        // Root del container.
        "--bind".into(), rootfs_path.into(), "/".into(),
        // Filesystems internos.
        "--proc".into(), "/proc".into(),
        "--dev".into(), "/dev".into(),
        "--tmpfs".into(), "/tmp".into(),
        // DNS funcional: copia el resolv.conf del host (ro).
        "--ro-bind-try".into(), "/etc/resolv.conf".into(), "/etc/resolv.conf".into(),
        // Aislamiento: namespaces propios menos net (compartido).
        "--unshare-pid".into(),
        "--unshare-uts".into(),
        "--unshare-ipc".into(),
        // El process tree muere si el padre muere — no quedan zombies.
        "--die-with-parent".into(),
        // Env mínimo razonable para un shell vacío.
        "--setenv".into(), "HOME".into(), "/root".into(),
        "--setenv".into(), "USER".into(), "root".into(),
        "--setenv".into(), "PATH".into(),
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
        "--setenv".into(), "TERM".into(), "xterm-256color".into(),
    ];
    // Si existe ~/work en el rootfs, lo usamos como cwd; sino /.
    a.push("--chdir".into());
    a.push("/".into());
    a
}

/// Variante de [`wrap_spec_for_container`] para `engine = "bwrap"`. El
/// `rootfs_path` es el filesystem extraído (LXC image) en disco local.
fn wrap_spec_for_bwrap(mut spec: CommandSpec, rootfs_path: &str) -> CommandSpec {
    let base = bwrap_args(rootfs_path);
    // Igual que unshare: HOME del root + `cd` al cwd interior trackeado.
    let prelude = format!(
        "export HOME=/root; cd {} 2>/dev/null || true; ",
        shell_quote(&spec.cwd)
    );
    spec.exec = match spec.exec {
        Exec::Shell { line, program: _ } => {
            // No-TUI → `Exec::Direct` (una etapa bwrap) para capturar
            // stdout/stderr por líneas y renderizar como bloques. Forzar PTY
            // sin TuiSession descartaba el output (ver wrap_spec_for_unshare).
            // Los TUI fullscreen van por la rama `Exec::Pty` de abajo.
            let mut args = base;
            args.push("--".into());
            args.push("bash".into());
            args.push("-c".into());
            args.push(format!("{prelude}{line}"));
            Exec::Direct { stages: vec![StageSpec { program: "bwrap".into(), args }] }
        }
        Exec::Pty { program, args, cols, rows } => {
            // TUI: envolvemos en `bash -c` para poder hacer el `cd` interior.
            let mut inner = prelude.clone();
            inner.push_str("exec ");
            inner.push_str(&shell_quote(&program));
            for a in &args {
                inner.push(' ');
                inner.push_str(&shell_quote(a));
            }
            let mut new_args = base;
            new_args.push("--".into());
            new_args.push("bash".into());
            new_args.push("-c".into());
            new_args.push(inner);
            Exec::Pty {
                program: "bwrap".into(),
                args: new_args,
                cols,
                rows,
            }
        }
        Exec::Direct { stages } => {
            // Serialize pipe as a single bash line (mismo tradeoff que podman).
            let mut line = String::new();
            for (i, st) in stages.iter().enumerate() {
                if i > 0 {
                    line.push_str(" | ");
                }
                line.push_str(&shell_quote(&st.program));
                for a in &st.args {
                    line.push(' ');
                    line.push_str(&shell_quote(a));
                }
            }
            let mut args = base;
            args.push("--".into());
            args.push("bash".into());
            args.push("-c".into());
            args.push(format!("{prelude}{line}"));
            Exec::Direct { stages: vec![StageSpec { program: "bwrap".into(), args }] }
        }
    };
    spec
}

/// Quote básico estilo Bourne para envolver en `'…'`. Sustituye `'` por
/// `'\''`. Suficiente para inyectar paths/comandos del usuario al wrap del
/// container; no pretende ser un parser POSIX completo.
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

pub(crate) fn build_spec(line: &str, cwd: &str) -> (CommandSpec, Option<TuiSession>) {
    // sudo sin `-A`/`-S` quedaría colgado pidiendo pass en stdin del PTY —
    // inyectamos `-A` para que use `SUDO_ASKPASS` (popup Llimphi).
    let line_owned = inject_askpass(line);
    let line = line_owned.as_str();
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
        // Limitamos los eventos por tick. Un `ls -alR /` puede escupir miles
        // de líneas en un solo flush; procesar todo dentro del lock de
        // `active_arc` pasma la pantalla porque el render llama `try_lock`
        // en cada frame. Con un tope, el lock se libera entre Ticks y la UI
        // se actualiza sin esperar al final del comando. Los restantes
        // QUEDAN EN LA COLA del backend para el próximo Tick (no se pierden).
        const DRAIN_BUDGET: usize = 512;
        let events = guard.handle.try_events_limit(DRAIN_BUDGET);
        for ev in events.into_iter() {
            match ev {
                RunEvent::Stdout(line) => {
                    // +1 por el `\n` implícito de cada línea drenada.
                    s.current_run_bytes = s.current_run_bytes.saturating_add(line.len() as u64 + 1);
                    // Strip ANSI escapes: comandos como `fastfetch`, `ls
                    // --color`, `git --color=always` emiten SGR + `\r` para
                    // sobrescribir. `strip_ansi` colapsa `\r` y descarta
                    // códigos sin perder el contenido visible. El coloreo
                    // real (style runs) queda para una iteración futura.
                    let clean = shuma_line::ansi::strip_ansi(&line);
                    s.push_in_block(run_block, OutputLine::stdout(clean));
                }
                RunEvent::StageStdout { stage, line } => {
                    // Salida de una etapa intermedia (tee). NO suma a
                    // `current_run_bytes` (el grafo cuenta la salida final);
                    // queda guardada para el desplegable de su etapa.
                    let clean = shuma_line::ansi::strip_ansi(&line);
                    s.push_in_block(run_block, OutputLine::stage_stdout(stage, clean));
                }
                RunEvent::Stderr(line) => {
                    s.current_run_bytes = s.current_run_bytes.saturating_add(line.len() as u64 + 1);
                    let clean = shuma_line::ansi::strip_ansi(&line);
                    s.push_in_block(run_block, OutputLine::stderr(clean));
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
            // Mismo límite que `drain_run`: ráfagas grandes de un job background
            // no pasman la pantalla; los eventos restantes se procesan en el
            // próximo Tick.
            for ev in guard.handle.try_events_limit(512) {
                match ev {
                    RunEvent::Stdout(line) => s.push_in_block(
                        job_block,
                        OutputLine::stdout(shuma_line::ansi::strip_ansi(&line)),
                    ),
                    RunEvent::StageStdout { stage, line } => s.push_in_block(
                        job_block,
                        OutputLine::stage_stdout(stage, shuma_line::ansi::strip_ansi(&line)),
                    ),
                    RunEvent::Stderr(line) => s.push_in_block(
                        job_block,
                        OutputLine::stderr(shuma_line::ansi::strip_ansi(&line)),
                    ),
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

/// Resuelve `.`/`..` de forma puramente léxica (sin tocar el FS ni seguir
/// symlinks). Para el `cd` dentro de un contenedor, donde el path es del FS
/// de adentro y `canonicalize` (host) no aplica.
fn normalize_lexical(p: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let mut out: Vec<std::ffi::OsString> = Vec::new();
    for comp in p.components() {
        match comp {
            Component::RootDir | Component::Prefix(_) => {}
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(c) => out.push(c.to_os_string()),
        }
    }
    let mut res = PathBuf::from("/");
    for c in out {
        res.push(c);
    }
    res
}

pub(crate) fn apply_cd(mut s: State, rest: &str) -> State {
    // En un contenedor el `cd` es contra el FS de ADENTRO, no el del host:
    // resolvemos el path de forma léxica (sin `canonicalize`, que miraría el
    // host) y actualizamos el cwd interior. Sin verificación de existencia —
    // el siguiente comando (que corre con `cd <cwd>` adentro) reporta el error
    // si el dir no existe.
    if matches!(s.source, Source::Container { .. }) {
        let trimmed = rest.trim();
        let base = if trimmed.is_empty() {
            PathBuf::from("/root") // HOME del root dentro del contenedor
        } else if trimmed.starts_with('/') {
            PathBuf::from(trimmed)
        } else {
            s.cwd.join(trimmed)
        };
        s.cwd = normalize_lexical(&base);
        s.completion_source = crate::completion_source_for(&s.source, &s.cwd);
        return s;
    }
    // Remoto (SSH): cada comando es un `ssh exec` (shell nuevo en $HOME). v1:
    // sólo persistimos `cd` a rutas ABSOLUTAS (un `cd` relativo no tiene contra
    // qué resolver sin un round-trip). El cwd se antepone como `cd` en run_ssh.
    if matches!(s.source, Source::Remote { .. }) {
        let trimmed = rest.trim();
        if trimmed.is_empty() {
            s.cwd = PathBuf::from("~");
        } else if trimmed.starts_with('/') {
            s.cwd = normalize_lexical(&PathBuf::from(trimmed));
        } else {
            s.push_output(OutputLine::notice(
                "cd remoto (v1): usá una ruta absoluta (p. ej. cd /var/log)",
            ));
        }
        return s;
    }
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
