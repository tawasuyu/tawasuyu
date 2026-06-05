use super::*;

pub(crate) fn dispatch(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::ToggleNode(i) => toggle_node(model, i),
            Msg::SelectNode(i) => select_node(model, i),
            Msg::MenuOpen(idx) => {
                let mut m = model;
                m.menu_open = idx;
                m.edit_menu = None;
                m.menu_active = usize::MAX;
                // Animación de aparición/swap: cada vez que se abre (o se
                // cambia de) menú, el dropdown se funde+desliza de nuevo.
                if idx.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
                m
            }
            Msg::MenuNav(dir) => {
                let mut m = model;
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
                m
            }
            Msg::MenuActivate => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    if let Some(cmd) = menubar_command_at(&menu, mi, model.menu_active) {
                        return handle_menu_command(model, cmd, handle);
                    }
                }
                model
            }
            Msg::EditNav(dir) => {
                let mut m = model;
                let (items, _) = build_edit_menu(&m);
                m.edit_active = step_active(&items, m.edit_active, dir);
                m
            }
            Msg::EditActivate => {
                let (_, picks) = build_edit_menu(&model);
                match picks.get(model.edit_active).copied() {
                    Some(CtxPick::Edit(a)) => apply_edit_menu_action(model, a),
                    Some(CtxPick::OpenSub(p)) => {
                        let mut m = model;
                        m.edit_sub = Some(p);
                        m
                    }
                    _ => model,
                }
            }
            Msg::MenuCommand(cmd) => handle_menu_command(model, cmd, handle),
            Msg::EditMenuOpen(x, y) => {
                let mut m = model;
                m.edit_menu = Some((x, y));
                m.menu_open = None;
                m.edit_sub = None;
                m.edit_active = usize::MAX;
                // Animación de aparición: 0→1 con ease-out, autodirigida
                // por ticks de llimphi-motion hasta terminar.
                m.edit_menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(handle, motion::FAST, || Msg::MenuTick);
                m
            }
            Msg::MenuTick => model,
            Msg::EditMenuAction(action) => apply_edit_menu_action(model, action),
            Msg::EditSubHover(opt) => {
                let mut m = model;
                m.edit_sub = opt;
                m
            }
            Msg::EditSubPick(_parent, child) => {
                // El submenú "Buscar": child → el Msg real de búsqueda.
                let mut m = model;
                m.edit_menu = None;
                m.edit_sub = None;
                let target = match child {
                    0 => Msg::FindOpen,
                    1 => Msg::Fif(FifMsg::Open),
                    2 => Msg::Outline(OutlineMsg::Open),
                    3 => Msg::GotoDefinitionRequest,
                    _ => return m,
                };
                return dispatch(m, target, handle);
            }
            Msg::CloseMenus => {
                let mut m = model;
                m.menu_open = None;
                m.edit_menu = None;
                m.edit_sub = None;
                m.menu_active = usize::MAX;
                m.edit_active = usize::MAX;
                m
            }
            Msg::EditKey(ev) => apply_editor_key(model, ev),
            Msg::EditorPointer(ev) => apply_editor_pointer(model, ev),
            Msg::Save => {
                // Si tenemos format-on-save activo y un LSP real,
                // primero format, después save: marcamos `pending_save`
                // y esperamos `TextEditsApply` para guardar de verdad.
                let mut m = model;
                if m.format_on_save && m.pending_save_after_format.is_none() {
                    if let Some(idx) = m.active {
                        let path = m.tabs[idx].path.clone();
                        m.lsp.clear_text_edits();
                        m.lsp.request_formatting(&path, 4, true);
                        m.pending_save_after_format = Some(idx);
                        m.status = "formateando antes de guardar…".to_string();
                        return m;
                    }
                }
                save_open_file(m, handle)
            }
            Msg::SaveAsOpen => {
                let mut m = model;
                let Some(tab) = m.active_tab() else { return m };
                let mut input = TextInputState::new();
                input.set_text(&tab.path.display().to_string());
                m.save_as = Some(SaveAsBar { input });
                m.status = "save as · editá la ruta + Enter · Esc cancela".to_string();
                m
            }
            Msg::SaveAsClose => {
                let mut m = model;
                m.save_as = None;
                m
            }
            Msg::SaveAsKey(ev) => {
                let mut m = model;
                if let Some(bar) = m.save_as.as_mut() {
                    let _ = bar.input.apply_key(&ev);
                }
                m
            }
            Msg::SaveAsSubmit => {
                let mut m = model;
                let Some(bar) = m.save_as.take() else { return m };
                let new_path = PathBuf::from(bar.input.text());
                let Some(idx) = m.active else { return m };
                if new_path.as_os_str().is_empty() {
                    m.status = "save as · ruta vacía, ignorado".to_string();
                    return m;
                }
                // Si ya hay otro tab con ese path, sobrescribir sería
                // confuso — abortamos y dejamos que el user resuelva.
                if m.tabs
                    .iter()
                    .enumerate()
                    .any(|(i, t)| i != idx && t.path == new_path)
                {
                    m.status = format!(
                        "save as · ya hay un tab con {} — cerralo primero",
                        new_path.display(),
                    );
                    return m;
                }
                if let Some(parent) = new_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        let _ = fs::create_dir_all(parent);
                    }
                }
                let content = m.tabs[idx].editor.text();
                match fs::write(&new_path, &content) {
                    Ok(()) => {
                        let old_path = m.tabs[idx].path.clone();
                        m.lsp.did_close(&old_path);
                        let ext = new_path
                            .extension()
                            .and_then(|s| s.to_str())
                            .unwrap_or("");
                        m.lsp.did_open(&new_path, ext, &content);
                        m.tabs[idx].path = new_path.clone();
                        m.tabs[idx].dirty = false;
                        m.tabs[idx].last_mtime = file_mtime(&new_path);
                        m.tabs[idx].external_warned = false;
                        if !m.all_files.contains(&new_path) {
                            m.all_files.push(new_path.clone());
                        }
                        m.status = format!("save as · {}", new_path.display());
                    }
                    Err(e) => {
                        // Restauramos el prompt para que el user corrija.
                        let mut input = TextInputState::new();
                        input.set_text(&new_path.display().to_string());
                        m.save_as = Some(SaveAsBar { input });
                        m.status = format!("save as · error: {e}");
                    }
                }
                m
            }
            Msg::Scroll(delta) => {
                let mut m = model;
                if let Some(tab) = m.active_tab_mut() {
                    tab.editor.scroll_by(delta);
                }
                m
            }
            Msg::WinResized(_w, h) => {
                let mut m = model;
                m.win_h = h as f32;
                m.win_w = _w as f32;
                // Re-clampea el scroll del árbol: si la ventana creció, el
                // contenido puede haber dejado de desbordar.
                m.tree_scroll = llimphi_widget_scroll::clamp_offset(
                    m.tree_scroll,
                    m.tree_content_h(),
                    m.tree_viewport_h(),
                );
                m
            }
            Msg::TreeScroll(delta) => {
                let mut m = model;
                m.tree_scroll = llimphi_widget_scroll::clamp_offset(
                    m.tree_scroll + delta,
                    m.tree_content_h(),
                    m.tree_viewport_h(),
                );
                m
            }
            Msg::ActivateTab(i) => activate_tab(model, i),
            Msg::CloseTab(i) => close_tab(model, i),
            Msg::NextTab => {
                let mut m = model;
                if !m.tabs.is_empty() {
                    let n = m.tabs.len();
                    let cur = m.active.unwrap_or(0);
                    m = activate_tab(m, (cur + 1) % n);
                }
                m
            }
            Msg::PrevTab => {
                let mut m = model;
                if !m.tabs.is_empty() {
                    let n = m.tabs.len();
                    let cur = m.active.unwrap_or(0);
                    m = activate_tab(m, (cur + n - 1) % n);
                }
                m
            }
            Msg::Picker(pm) => apply_picker(model, pm),
            Msg::Fif(fmsg) => apply_fif(model, fmsg),
            Msg::Term(tm) => apply_term(model, tm),
            Msg::Palette(pm) => apply_palette(model, pm, handle),
            Msg::Outline(om) => apply_outline(model, om),
            Msg::OutlineRefresh(items) => {
                let mut m = model;
                m.outline_symbols = items;
                // Si el panel está abierto, refrescamos su filtro.
                if let Some(state) = m.outline.as_mut() {
                    outline::refilter(state, &m.outline_symbols);
                }
                m
            }
            Msg::MiniMap(mm) => apply_minimap(model, mm),
            Msg::Bookmarks(bm) => apply_bookmarks(model, bm),
            Msg::SaveSession => {
                save_session(&model);
                model
            }
            Msg::GitStatusChanged(map) => {
                let mut m = model;
                if map != m.git_status {
                    m.git_status = map;
                }
                m
            }
            Msg::CycleTheme => {
                let mut m = model;
                m.theme = Theme::next_after(m.theme.name);
                m.status = format!("✓ tema: {}", m.theme.name);
                m
            }
            Msg::WawaConfigChanged(cfg) => {
                let mut m = model;
                let mut changes = Vec::new();
                let next_theme = theme_from_wawa(&cfg, &m.theme);
                // Comparamos por color de accent (cubre acent override)
                // y por nombre del preset base. Cambio en cualquiera = re-aplicar.
                if next_theme.name != m.theme.name {
                    changes.push("tema");
                }
                if next_theme.accent != m.theme.accent {
                    changes.push("acento");
                }
                m.theme = next_theme;
                if cfg.lang != rimay_localize::current_locale() {
                    let _ = rimay_localize::set_locale(&cfg.lang);
                    changes.push("idioma");
                }
                if !changes.is_empty() {
                    m.status = format!("↻ wawa-config · {}", changes.join(" + "));
                }
                m
            }
            Msg::Diff(dm) => apply_diff(model, dm),
            Msg::FindOpen => {
                let mut m = model;
                if m.find.is_none() {
                    m.find = Some(FindBarState::new());
                    m.status = rimay_localize::t("edit-status-find");
                }
                m
            }
            Msg::FindClose => Model { find: None, ..model },
            Msg::FindKey(ev) => {
                let mut m = model;
                if let Some(f) = m.find.as_mut() {
                    f.input.apply_key(&ev);
                    f.sync();
                }
                m
            }
            Msg::FindNext => find_step(model, true),
            Msg::FindPrev => find_step(model, false),
            Msg::PollLsp => {
                let mut m = model;
                if let (Some(idx), Some(path)) = (m.active, m.active_path()) {
                    let diags = m.lsp.diagnostics(&path);
                    if diags != m.tabs[idx].editor.diagnostics {
                        m.tabs[idx].editor.set_diagnostics(diags);
                    }
                }
                // Si hay request de completions pendiente (popup abierto
                // sin items todavía), pollamos.
                if let Some(bar) = m.completions.as_mut() {
                    let latest = m.lsp.latest_completions();
                    if !latest.is_empty() && latest != bar.items {
                        bar.items = latest;
                        bar.selected = 0;
                    }
                }
                if let Some(popup) = m.hover.as_mut() {
                    let latest = m.lsp.latest_hover();
                    if latest.is_some() && latest != popup.info {
                        popup.info = latest;
                    }
                }
                if let Some(bar) = m.sig_help.as_mut() {
                    let latest = m.lsp.latest_signature_help();
                    if latest.is_some() && latest != bar.info {
                        bar.info = latest;
                    }
                }
                if let Some(bar) = m.references.as_mut() {
                    let latest = m.lsp.latest_references();
                    if !latest.is_empty() && latest != bar.items {
                        bar.items = latest;
                        bar.selected = 0;
                        bar.timeout_warned = false;
                    } else if bar.items.is_empty()
                        && !bar.timeout_warned
                        && bar.requested_at.elapsed() >= std::time::Duration::from_secs(3)
                    {
                        bar.timeout_warned = true;
                        m.status = "references · LSP timeout · sin respuesta en 3 s".to_string();
                    }
                }
                // Goto-def: si llegó una definition, dispara apply en
                // el próximo tick para no anidar update.
                if let Some(loc) = m.lsp.latest_definition() {
                    m.lsp.clear_definition();
                    handle.dispatch(Msg::GotoDefinitionApply(loc));
                }
                // Text edits (formatting): aplicar al recibir.
                let edits = m.lsp.latest_text_edits();
                if !edits.is_empty() {
                    m.lsp.clear_text_edits();
                    handle.dispatch(Msg::TextEditsApply(edits));
                }
                // WorkspaceEdit (rename): aplicar al recibir.
                let we = m.lsp.latest_workspace_edit();
                if !we.is_empty() {
                    m.lsp.clear_workspace_edit();
                    handle.dispatch(Msg::RenameApply(we));
                }
                // Timeout del rename: si waiting + 3 s sin WorkspaceEdit
                // → avisamos una vez para no dejar al user esperando.
                if let Some(r) = m.rename.as_mut() {
                    if r.waiting && !r.timeout_warned {
                        if let Some(t) = r.submitted_at {
                            if t.elapsed() >= std::time::Duration::from_secs(3) {
                                r.timeout_warned = true;
                                m.status = "rename · LSP timeout · Esc cancela".to_string();
                            }
                        }
                    }
                }
                // Document symbols: si llegaron y son distintos a lo que
                // tenemos, refresca el outline state.
                let syms = m.lsp.latest_document_symbols();
                if !syms.is_empty() {
                    let items = symbols_lsp_to_module(syms);
                    if items != m.outline_symbols {
                        m.lsp.clear_document_symbols();
                        handle.dispatch(Msg::OutlineRefresh(items));
                    }
                }
                // Watcher liviano: comparamos mtimes en disco vs los que
                // teníamos al abrir o al último save. Si difiere y el
                // tab está limpio, recargamos; si está sucio, alertamos
                // una sola vez para evitar spam.
                detect_external_changes(&mut m);
                m
            }
            Msg::CompletionsRequest => {
                let mut m = model;
                let Some(idx) = m.active else { return m };
                let path = m.tabs[idx].path.clone();
                let line = m.tabs[idx].editor.cursor.caret.line;
                let col = m.tabs[idx].editor.cursor.caret.col;
                m.lsp.clear_completions();
                m.lsp.request_completions(&path, line, col);
                let (_, prefix) = m.tabs[idx].editor.buffer.current_word_prefix(line, col);
                m.completions = Some(CompletionsBar {
                    items: Vec::new(),
                    selected: 0,
                    anchor: (line, col),
                    filter: prefix,
                });
                m
            }
            Msg::CompletionsNav { delta } => {
                let mut m = model;
                if let Some(bar) = m.completions.as_mut() {
                    let n = bar.filtered_indices().len() as i32;
                    if n > 0 {
                        let sel = (bar.selected as i32 + delta).rem_euclid(n);
                        bar.selected = sel as usize;
                    }
                }
                m
            }
            Msg::CompletionsApply => {
                let mut m = model;
                let Some(bar) = m.completions.take() else { return m };
                m.lsp.clear_completions();
                let Some(idx) = m.active else { return m };
                // Resolvemos el item seleccionado en el filtered set.
                let filtered = bar.filtered_indices();
                let Some(&item_idx) = filtered.get(bar.selected) else { return m };
                let item = match bar.items.get(item_idx) {
                    Some(it) => it.clone(),
                    None => return m,
                };
                let text = item.text_to_insert().to_string();
                // Smart-replace: seleccionamos [word_start_col..caret_col]
                // de la línea actual y reemplazamos por `text`. Si no hay
                // prefijo, queda como simple insert.
                let line = m.tabs[idx].editor.cursor.caret.line;
                let caret_col = m.tabs[idx].editor.cursor.caret.col;
                let (word_start, _) =
                    m.tabs[idx].editor.buffer.current_word_prefix(line, caret_col);
                if word_start < caret_col {
                    m.tabs[idx].editor.cursor.anchor =
                        Some(llimphi_widget_text_editor::Pos::new(line, word_start));
                    m.tabs[idx].editor.cursor.caret =
                        llimphi_widget_text_editor::Pos::new(line, caret_col);
                }
                let tab = &mut m.tabs[idx];
                let _ = llimphi_widget_text_editor::ops::replace_selection(
                    &mut tab.editor.buffer,
                    &mut tab.editor.cursor,
                    &text,
                );
                tab.editor.bump_edit_seq();
                tab.dirty = true;
                let path = tab.path.clone();
                let new_text = tab.editor.text();
                m.lsp.did_change(&path, &new_text);
                m
            }
            Msg::CompletionsClose => {
                let mut m = model;
                m.completions = None;
                m.lsp.clear_completions();
                m
            }
            Msg::HoverRequest => {
                let mut m = model;
                let Some(idx) = m.active else { return m };
                let path = m.tabs[idx].path.clone();
                let line = m.tabs[idx].editor.cursor.caret.line;
                let col = m.tabs[idx].editor.cursor.caret.col;
                m.lsp.clear_hover();
                m.lsp.request_hover(&path, line, col);
                m.hover = Some(HoverPopup { info: None, anchor: (line, col) });
                m
            }
            Msg::HoverClose => {
                let mut m = model;
                m.hover = None;
                m.lsp.clear_hover();
                m
            }
            Msg::GotoDefinitionRequest => {
                let mut m = model;
                let Some(idx) = m.active else { return m };
                let path = m.tabs[idx].path.clone();
                let line = m.tabs[idx].editor.cursor.caret.line;
                let col = m.tabs[idx].editor.cursor.caret.col;
                m.lsp.clear_definition();
                m.lsp.request_definition(&path, line, col);
                m.status = rimay_localize::t("edit-status-goto-def-waiting");
                m
            }
            Msg::ReferencesRequest => {
                let mut m = model;
                let Some(idx) = m.active else { return m };
                let path = m.tabs[idx].path.clone();
                let line = m.tabs[idx].editor.cursor.caret.line;
                let col = m.tabs[idx].editor.cursor.caret.col;
                m.lsp.clear_references();
                m.lsp.request_references(&path, line, col, true);
                m.references = Some(ReferencesBar {
                    items: Vec::new(),
                    selected: 0,
                    anchor: (line, col),
                    requested_at: std::time::Instant::now(),
                    timeout_warned: false,
                });
                m.status = rimay_localize::t("edit-status-references-waiting");
                m
            }
            Msg::ReferencesNav { delta } => {
                let mut m = model;
                if let Some(bar) = m.references.as_mut() {
                    let n = bar.items.len() as i32;
                    if n > 0 {
                        bar.selected = ((bar.selected as i32 + delta).rem_euclid(n)) as usize;
                    }
                }
                m
            }
            Msg::ReferencesApply => {
                let m = model;
                if let Some(bar) = m.references.as_ref() {
                    if let Some(loc) = bar.items.get(bar.selected).cloned() {
                        let mut m2 = m;
                        m2.references = None;
                        m2.lsp.clear_references();
                        return dispatch(m2, Msg::GotoDefinitionApply(loc), handle);
                    }
                }
                m
            }
            Msg::ReferencesClose => {
                let mut m = model;
                m.references = None;
                m.lsp.clear_references();
                m
            }
            Msg::RenameOpen => {
                let mut m = model;
                let Some(idx) = m.active else { return m };
                let line = m.tabs[idx].editor.cursor.caret.line;
                let col = m.tabs[idx].editor.cursor.caret.col;
                let (start, word) = m.tabs[idx].editor.buffer.current_word_prefix(line, col);
                let _ = start;
                let mut input = TextInputState::new();
                input.set_text(&word);
                m.rename = Some(RenameBar {
                    input,
                    anchor: (line, col),
                    waiting: false,
                    submitted_at: None,
                    timeout_warned: false,
                });
                m.status = rimay_localize::t("edit-status-rename-input");
                m
            }
            Msg::RenameKey(ev) => {
                let mut m = model;
                if let Some(r) = m.rename.as_mut() {
                    r.input.apply_key(&ev);
                }
                m
            }
            Msg::RenameSubmit => {
                let mut m = model;
                let Some(path) = m.active_path() else { return m };
                let Some(r) = m.rename.as_mut() else { return m };
                let new_name = r.input.text();
                if new_name.is_empty() {
                    return m;
                }
                m.lsp.clear_workspace_edit();
                m.lsp.request_rename(&path, r.anchor.0, r.anchor.1, &new_name);
                r.waiting = true;
                r.submitted_at = Some(std::time::Instant::now());
                r.timeout_warned = false;
                m.status = rimay_localize::t_args(
                    "edit-status-rename-waiting",
                    &[("name", new_name.as_str().into())],
                );
                m
            }
            Msg::RenameClose => {
                let mut m = model;
                m.rename = None;
                m.lsp.clear_workspace_edit();
                m
            }
            Msg::RenameApply(we) => {
                let mut m = model;
                m.rename = None;
                let mut files_changed = 0;
                let mut bytes_written = 0usize;
                for (path, edits) in we {
                    // ¿Tenemos un tab abierto sobre este path? Si sí, lo
                    // editamos en memoria y notificamos al LSP.
                    if let Some(tab_idx) = m.tab_idx_for(&path) {
                        let tab = &mut m.tabs[tab_idx];
                        apply_text_edits_in_place(&mut tab.editor, edits);
                        tab.dirty = true;
                        let new_text = tab.editor.text();
                        m.lsp.did_change(&path, &new_text);
                        files_changed += 1;
                    } else {
                        match apply_text_edits_to_file(&path, &edits) {
                            Ok(n) => {
                                files_changed += 1;
                                bytes_written += n;
                            }
                            Err(e) => {
                                m.status = rimay_localize::t_args(
                                    "edit-status-rename-error",
                                    &[
                                        ("path", path.display().to_string().into()),
                                        ("err", e.to_string().into()),
                                    ],
                                );
                                return m;
                            }
                        }
                    }
                }
                m.status = rimay_localize::t_args(
                    "edit-status-rename-done",
                    &[
                        ("files", files_changed.to_string().into()),
                        ("bytes", bytes_written.to_string().into()),
                    ],
                );
                m
            }
            Msg::SignatureHelpRequest => {
                let mut m = model;
                let Some(idx) = m.active else { return m };
                let path = m.tabs[idx].path.clone();
                let line = m.tabs[idx].editor.cursor.caret.line;
                let col = m.tabs[idx].editor.cursor.caret.col;
                m.lsp.clear_signature_help();
                m.lsp.request_signature_help(&path, line, col);
                m.sig_help = Some(SignatureHelpBar { info: None, anchor: (line, col) });
                m
            }
            Msg::SignatureHelpClose => {
                let mut m = model;
                m.sig_help = None;
                m.lsp.clear_signature_help();
                m
            }
            Msg::FormatRequest => {
                let mut m = model;
                let Some(path) = m.active_path() else { return m };
                m.lsp.clear_text_edits();
                m.lsp.request_formatting(&path, 4, true);
                m.status = rimay_localize::t("edit-status-formatting-waiting");
                m
            }
            Msg::TextEditsApply(edits) => {
                let mut m = model;
                let Some(idx) = m.active else { return m };
                let tab = &mut m.tabs[idx];
                apply_text_edits_in_place(&mut tab.editor, edits);
                tab.dirty = true;
                let path = tab.path.clone();
                let new_text = tab.editor.text();
                m.lsp.did_change(&path, &new_text);
                m.status = rimay_localize::t("edit-status-formatting-done");
                // Si veníamos de un Save con format-on-save, escribimos
                // ahora. Limpia el pending para evitar el loop.
                if m.pending_save_after_format == Some(idx) {
                    m.pending_save_after_format = None;
                    return save_open_file(m, handle);
                }
                m
            }
            Msg::GotoDefinitionApply(loc) => {
                let mut m = model;
                m.lsp.clear_definition();
                // ¿Ya hay tab con este path? Si sí, lo activamos y movemos
                // el caret. Si no, leemos del disco y abrimos un tab nuevo.
                if let Some(idx) = m.tab_idx_for(&loc.path) {
                    m.active = Some(idx);
                    let tab = &mut m.tabs[idx];
                    tab.editor.set_caret_at(loc.line, loc.col);
                    tab.editor.ensure_caret_visible(EDITOR_VISIBLE_LINES);
                    m.status = rimay_localize::t_args(
                        "edit-status-goto-def-at",
                        &[
                            ("path", loc.path.display().to_string().into()),
                            ("line", (loc.line + 1).to_string().into()),
                        ],
                    );
                    return m;
                }
                match fs::read_to_string(&loc.path) {
                    Ok(content) => {
                        let mut editor = EditorState::new();
                        editor.set_text(&content);
                        editor.set_caret_at(loc.line, loc.col);
                        editor.ensure_caret_visible(EDITOR_VISIBLE_LINES);
                        let ext = loc.path.extension().and_then(|s| s.to_str()).unwrap_or("");
                        m.lsp.did_open(&loc.path, ext, &content);
                        let mtime = file_mtime(&loc.path);
                        m.tabs.push(Tab {
                            path: loc.path.clone(),
                            editor,
                            dirty: false,
                            last_mtime: mtime,
                            external_warned: false,
                        });
                        m.active = Some(m.tabs.len() - 1);
                        m.status = rimay_localize::t_args(
                        "edit-status-goto-def-at",
                        &[
                            ("path", loc.path.display().to_string().into()),
                            ("line", (loc.line + 1).to_string().into()),
                        ],
                    );
                    }
                    Err(e) => {
                        m.status = rimay_localize::t_args(
                            "edit-status-goto-def-error",
                            &[
                                ("path", loc.path.display().to_string().into()),
                                ("err", e.to_string().into()),
                            ],
                        );
                    }
                }
                m
            }
            Msg::SaveResult(r) => {
                let mut m = model;
                m.status = match r {
                    Ok(()) => {
                        let path_disp = m
                            .active_tab()
                            .map(|t| t.path.display().to_string())
                            .unwrap_or_default();
                        if let Some(tab) = m.active_tab_mut() {
                            tab.dirty = false;
                            tab.last_mtime = file_mtime(&tab.path);
                            tab.external_warned = false;
                        }
                        rimay_localize::t_args(
                            "edit-status-saved",
                            &[("path", path_disp.into())],
                        )
                    }
                    Err(e) => rimay_localize::t_args(
                        "edit-status-save-error",
                        &[("err", e.to_string().into())],
                    ),
                };
                m
            }
            Msg::SettingsToggle => {
                let mut m = model;
                m.settings = if m.settings.is_some() {
                    None
                } else {
                    Some(AllichayState::new())
                };
                m
            }
            Msg::SettingsClose => {
                let mut m = model;
                m.settings = None;
                m
            }
            Msg::Settings(am) => {
                let mut m = model;
                match am {
                    // Un cambio de campo se aplica al `Model` (y al status).
                    AllichayMsg::Change(path, value) => {
                        crate::settings::apply_settings_change(&mut m, &path, value);
                    }
                    // El resto sólo muta el estado del panel (diente activo,
                    // scroll, foco). Los `Focus*` no se disparan hoy porque
                    // ningún campo es de texto/celda/hex; quedan como no-op.
                    other => {
                        if let Some(st) = m.settings.as_mut() {
                            match other {
                                AllichayMsg::SelectSection(i) => st.select(i),
                                AllichayMsg::ScrollTo(o) => st.set_scroll(o),
                                _ => {}
                            }
                        }
                    }
                }
                m
            }
            Msg::SettingsKey(ev) => {
                let mut m = model;
                // Enruta la tecla al campo de texto focado del panel; si editó,
                // devuelve el cambio entero para aplicarlo.
                let change = m.settings.as_mut().and_then(|st| st.apply_key(&ev));
                if let Some((path, value)) = change {
                    crate::settings::apply_settings_change(&mut m, &path, value);
                }
                m
            }
        }
}
