use super::*;

pub(crate) fn handle_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }

        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra. Tiene prioridad sobre todo lo demás.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            match &event.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => {
                    return Some(Msg::MenuOpen(Some((mi + n - 1) % n)));
                }
                Key::Named(NamedKey::ArrowRight) => {
                    return Some(Msg::MenuOpen(Some((mi + 1) % n)));
                }
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::MenuActivate),
                _ => return None,
            }
        }

        // Menú de edición (right-click) abierto: ↑/↓ navegan, → abre el
        // submenú de la fila activa, ← lo cierra, Enter ejecuta, Esc cierra.
        if model.edit_menu.is_some() {
            match &event.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::EditNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::EditNav(-1)),
                Key::Named(NamedKey::ArrowRight) => return Some(Msg::EditActivate),
                Key::Named(NamedKey::ArrowLeft) => return Some(Msg::EditSubHover(None)),
                Key::Named(NamedKey::Enter) => return Some(Msg::EditActivate),
                _ => return None,
            }
        }

        // Si el popup de completions está abierto, intercepta nav.
        if model.completions.is_some() {
            match &event.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CompletionsClose),
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::CompletionsNav { delta: 1 }),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::CompletionsNav { delta: -1 }),
                Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Tab) => {
                    return Some(Msg::CompletionsApply);
                }
                _ => {}
            }
        }

        // Command palette abierto: el módulo se lleva todas las teclas
        // (filtro, ↓↑, Enter, Esc).
        if let Some(state) = model.palette.as_ref() {
            if let Some(pm) = palette::on_key(state, event) {
                return Some(Msg::Palette(pm));
            }
        }
        // Symbol outline abierto: idem.
        if let Some(state) = model.outline.as_ref() {
            if let Some(om) = outline::on_key(state, event) {
                return Some(Msg::Outline(om));
            }
        }
        if let Some(bm) = bookmarks::on_key(&model.bookmarks, event) {
            return Some(Msg::Bookmarks(bm));
        }
        // Diff viewer abierto: idem.
        if let Some(state) = model.diff.as_ref() {
            if let Some(dm) = diff::on_key(state, event) {
                return Some(Msg::Diff(dm));
            }
        }

        // Terminal abierto: traga TODAS las teclas (salvo el toggle de
        // apertura, que se reusa para cerrar abajo). El módulo internamente
        // intercepta Ctrl+Shift+W → Close.
        if let Some(state) = model.term.as_ref() {
            // Re-presionar el atajo de apertura cierra el panel y devuelve
            // el foco al editor.
            if term::open_shortcut(event) {
                return Some(Msg::Term(ShumaTermMsg::Close));
            }
            if let Some(tm) = term::on_key(state, event) {
                return Some(Msg::Term(tm));
            }
        }

        // Picker abierto: el módulo decide qué hacer con cada tecla.
        if let Some(state) = model.picker.as_ref() {
            if let Some(pm) = picker::on_key(state, event) {
                return Some(Msg::Picker(pm));
            }
        }
        // Find-in-files abierto: el módulo decide qué hacer con cada tecla.
        if let Some(state) = model.fif.as_ref() {
            if let Some(fm) = fif::on_key(state, event) {
                return Some(Msg::Fif(fm));
            }
        }

        // Save-As (Ctrl+Shift+S): prompt-input con el path actual
        // prepopulado. Si el prompt ya está abierto, las teclas las
        // tragamos abajo (en su rama dedicada).
        if let Some(state) = model.save_as.as_ref() {
            if event.state == KeyState::Pressed {
                return Some(match &event.key {
                    Key::Named(NamedKey::Escape) => Msg::SaveAsClose,
                    Key::Named(NamedKey::Enter) => Msg::SaveAsSubmit,
                    _ => Msg::SaveAsKey(event.clone()),
                });
            }
            let _ = state;
        }

        // Atajos globales
        if event.modifiers.ctrl {
            if event.modifiers.shift
                && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("s"))
            {
                return Some(Msg::SaveAsOpen);
            }
            if matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("s")) {
                return Some(Msg::Save);
            }
            // Ctrl+P abre el fuzzy file picker (helper del módulo).
            if picker::open_shortcut(event) {
                return Some(Msg::Picker(PickerMsg::Open));
            }
            // Ctrl+W cierra el tab activo.
            if matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("w")) {
                if let Some(idx) = model.active {
                    return Some(Msg::CloseTab(idx));
                }
            }
            // Ctrl+Tab / Ctrl+Shift+Tab ciclan entre tabs.
            if matches!(&event.key, Key::Named(NamedKey::Tab)) && model.tabs.len() > 1 {
                return Some(if event.modifiers.shift { Msg::PrevTab } else { Msg::NextTab });
            }
            if !event.modifiers.shift
                && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("f"))
                && model.active_tab().is_some()
            {
                return Some(Msg::FindOpen);
            }
            if matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("g"))
                && model.find.is_some()
            {
                return Some(if event.modifiers.shift { Msg::FindPrev } else { Msg::FindNext });
            }
            // Ctrl+Space pide completions al LSP.
            if matches!(&event.key, Key::Named(NamedKey::Space))
                && model.active_tab().is_some()
            {
                return Some(Msg::CompletionsRequest);
            }
            // Ctrl+K pide hover en la pos del caret.
            if matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("k"))
                && model.active_tab().is_some()
            {
                return Some(Msg::HoverRequest);
            }
            // Ctrl+Shift+F = find-in-files (helper del módulo).
            if fif::open_shortcut(event) {
                return Some(Msg::Fif(FifMsg::Open));
            }
            // Ctrl+` = abre el terminal integrado.
            if term::open_shortcut(event) {
                return Some(Msg::Term(ShumaTermMsg::Open));
            }
            // Ctrl+Shift+P = abre el command palette.
            if palette::open_shortcut(event) {
                return Some(Msg::Palette(PaletteMsg::Open));
            }
            // Ctrl+Shift+O = abre el symbol outline.
            if outline::open_shortcut(event) {
                return Some(Msg::Outline(OutlineMsg::Open));
            }
            // Ctrl+Shift+D = abre el diff viewer (disco vs buffer).
            if diff::open_shortcut(event) {
                return Some(Msg::Diff(DiffMsg::Open));
            }
            if minimap::open_shortcut(event) {
                let already_open = model.minimap.is_some();
                return Some(Msg::MiniMap(if already_open { MiniMapMsg::Close } else { MiniMapMsg::Open }));
            }
            // Ctrl+Alt+T = ciclar tema.
            if event.modifiers.alt
                && !event.modifiers.shift
                && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("t"))
            {
                return Some(Msg::CycleTheme);
            }
            if bookmarks::open_shortcut(event) {
                let already_open = model.bookmarks.overlay.is_some();
                return Some(Msg::Bookmarks(if already_open { BookmarksMsg::CloseList } else { BookmarksMsg::OpenList }));
            }
            if bookmarks::toggle_shortcut(event) {
                if let (Some(idx), Some(path)) = (model.active, model.active_path()) {
                    let line = model.tabs[idx].editor.cursor.caret.line;
                    return Some(Msg::Bookmarks(BookmarksMsg::ToggleAt { path, line }));
                }
            }
            if bookmarks::next_shortcut(event) {
                if let (Some(idx), Some(path)) = (model.active, model.active_path()) {
                    let line = model.tabs[idx].editor.cursor.caret.line;
                    return Some(Msg::Bookmarks(BookmarksMsg::JumpNext { current_path: path, current_line: line }));
                }
            }
            if bookmarks::prev_shortcut(event) {
                if let (Some(idx), Some(path)) = (model.active, model.active_path()) {
                    let line = model.tabs[idx].editor.cursor.caret.line;
                    return Some(Msg::Bookmarks(BookmarksMsg::JumpPrev { current_path: path, current_line: line }));
                }
            }
            // Ctrl+Alt+L = format (estilo JetBrains; antes era Ctrl+Shift+F).
            if event.modifiers.alt
                && !event.modifiers.shift
                && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("l"))
                && model.active_tab().is_some()
            {
                return Some(Msg::FormatRequest);
            }
            // Ctrl+Shift+Space = signatureHelp.
            if event.modifiers.shift
                && matches!(&event.key, Key::Named(NamedKey::Space))
                && model.active_tab().is_some()
            {
                return Some(Msg::SignatureHelpRequest);
            }
        }
        // Esc cierra sig_help antes que cualquier otra cosa.
        if model.sig_help.is_some()
            && matches!(&event.key, Key::Named(NamedKey::Escape))
        {
            return Some(Msg::SignatureHelpClose);
        }
        // Rename prompt abierto: las teclas van al input, Enter submit, Esc cierra.
        if model.rename.is_some() {
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::RenameClose),
                Key::Named(NamedKey::Enter) => Some(Msg::RenameSubmit),
                _ => Some(Msg::RenameKey(event.clone())),
            };
        }
        // References abierto: Up/Down navega, Enter aplica, Esc cierra.
        if model.references.is_some() {
            match &event.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::ReferencesClose),
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::ReferencesNav { delta: 1 }),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::ReferencesNav { delta: -1 }),
                Key::Named(NamedKey::Enter) => return Some(Msg::ReferencesApply),
                _ => {}
            }
        }
        // F12 = goto-definition; Shift+F12 = references.
        if matches!(&event.key, Key::Named(NamedKey::F12))
            && model.active_tab().is_some()
        {
            return Some(if event.modifiers.shift {
                Msg::ReferencesRequest
            } else {
                Msg::GotoDefinitionRequest
            });
        }
        // F2 = rename.
        if matches!(&event.key, Key::Named(NamedKey::F2))
            && model.active_tab().is_some()
        {
            return Some(Msg::RenameOpen);
        }
        // Hover popup abierto + Esc → cerrar.
        if model.hover.is_some()
            && matches!(&event.key, Key::Named(NamedKey::Escape))
        {
            return Some(Msg::HoverClose);
        }

        // Esc colapsa multi-cursor antes de cerrar find/etc.
        if matches!(&event.key, Key::Named(NamedKey::Escape))
            && model.active_tab().is_some_and(|t| t.editor.has_multi_cursor())
        {
            return Some(Msg::EditKey(event.clone())); // lo ruteamos al editor
        }

        // Modo find abierto: el input se queda con todo menos Esc/Enter/F3.
        if model.find.is_some() {
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::FindClose),
                Key::Named(NamedKey::Enter) => Some(if event.modifiers.shift {
                    Msg::FindPrev
                } else {
                    Msg::FindNext
                }),
                Key::Named(NamedKey::F3) => Some(if event.modifiers.shift {
                    Msg::FindPrev
                } else {
                    Msg::FindNext
                }),
                _ => Some(Msg::FindKey(event.clone())),
            };
        }

        if model.active_tab().is_none() {
            return None;
        }
        Some(Msg::EditKey(event.clone()))
}
