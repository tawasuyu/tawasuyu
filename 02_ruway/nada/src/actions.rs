//! Controlador: apply_* de cada overlay/acción + helpers de tabs/find.
#![allow(unused_imports)]
use crate::prelude::*;
use crate::*;
use crate::actions::*;
use crate::fsutil::*;
use crate::view::*;
use crate::session::*;
use crate::clipboard::*;
use crate::keys::*;
use crate::update::*;
pub(crate) fn toggle_node(mut model: Model, i: usize) -> Model {
    let Some(node) = model.nodes.get(i).cloned() else {
        return model;
    };
    if !node.is_dir {
        return model;
    }
    let new_expanded = !node.expanded;
    model.nodes[i].expanded = new_expanded;
    if new_expanded {
        // Insertamos children justo después de `i`.
        let mut children: Vec<TreeNode> = Vec::new();
        visit_dir(&node.path, node.depth + 1, true, &mut children);
        // Splice
        for (offset, child) in children.into_iter().enumerate() {
            model.nodes.insert(i + 1 + offset, child);
        }
    } else {
        // Quitamos descendants (deeper depth) hasta el primer hermano.
        let mut j = i + 1;
        while j < model.nodes.len() && model.nodes[j].depth > node.depth {
            j += 1;
        }
        model.nodes.drain((i + 1)..j);
    }
    model
}

pub(crate) fn select_node(mut model: Model, i: usize) -> Model {
    let Some(node) = model.nodes.get(i).cloned() else {
        return model;
    };
    model.selected = Some(i);
    if node.is_dir {
        // Click en directorio = toggle también, así no necesita el chevron.
        return toggle_node(model, i);
    }
    open_path(model, node.path)
}

/// Abre un archivo: si ya hay un tab con ese path lo activa; si no, lee
/// del disco, crea EditorState nuevo, notifica `did_open` al LSP y empuja
/// un tab nuevo. Mensaje de status según el resultado.
pub(crate) fn open_path(mut model: Model, path: PathBuf) -> Model {
    push_recent(&mut model.recent_files, &path);
    if let Some(tab_idx) = model.tab_idx_for(&path) {
        model.active = Some(tab_idx);
        model.status = format!("activo · {}", relative_to(&model.root, &path));
        return model;
    }
    match fs::read_to_string(&path) {
        Ok(content) => {
            let mut editor = EditorState::new();
            editor.set_text(&content);
            if model.demo_lsp {
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                if ext == "rs" || ext == "py" {
                    editor.set_diagnostics(demo_diagnostics(&content));
                }
            }
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            model.lsp.did_open(&path, ext, &content);
            let mtime = file_mtime(&path);
            model.tabs.push(Tab {
                path: path.clone(),
                editor,
                dirty: false,
                last_mtime: mtime,
                external_warned: false,
            });
            model.active = Some(model.tabs.len() - 1);
            model.status = format!("abierto · {} bytes", content.len());
        }
        Err(e) => {
            model.status = format!("error abriendo {}: {e}", path.display());
        }
    }
    model
}

/// Routea un PickerMsg al módulo y traduce el `PickerAction` resultante.
pub(crate) fn apply_picker(model: Model, pm: PickerMsg) -> Model {
    let mut m = model;
    if matches!(pm, PickerMsg::Open) && m.picker.is_none() {
        let ordered = files_with_recents_first(&m.recent_files, &m.all_files);
        m.picker = Some(PickerState::new(&ordered, &m.root));
        m.status = format!(
            "picker · {} archivos · ↓↑ Enter abre · Esc cierra",
            m.all_files.len(),
        );
        return m;
    }
    let ordered = files_with_recents_first(&m.recent_files, &m.all_files);
    let action = match m.picker.as_mut() {
        Some(state) => picker::apply(state, pm, &ordered, &m.root),
        None => return m,
    };
    match action {
        PickerAction::None => {}
        PickerAction::Close => m.picker = None,
        PickerAction::Open(path) => {
            m.picker = None;
            m = open_path(m, path);
        }
    }
    m
}

/// Routea un FifMsg a `llimphi_module_fif::apply` y traduce el `FifAction`
/// resultante a la mutación apropiada del Model. Único lugar de nada
/// que conoce los detalles del módulo.
pub(crate) fn apply_fif(model: Model, fmsg: FifMsg) -> Model {
    let mut m = model;
    // Lazy-init en Open: si no había state, lo creamos (dialog_open=true
    // por default). Si ya existía, reabrimos el dialog conservando los
    // resultados previos — pasamos `FifMsg::Open` adelante a `apply`.
    if matches!(fmsg, FifMsg::Open) && m.fif.is_none() {
        m.fif = Some(FifState::new());
        m.status = format!(
            "find-in-files · escribí + Enter para buscar en {} archivos · Esc cierra",
            m.all_files.len(),
        );
        return m;
    }
    let action = match m.fif.as_mut() {
        Some(state) => fif::apply(state, fmsg, &m.all_files),
        None => return m,
    };
    match action {
        FifAction::None => {}
        FifAction::CloseDialog => {
            if let Some(state) = m.fif.as_mut() {
                state.dialog_open = false;
            }
        }
        FifAction::CloseAll => {
            m.fif = None;
        }
        FifAction::Searched { matches, elapsed, query } => {
            m.status = format!(
                "find-in-files · «{query}» · {matches} matches · {:.0} ms",
                elapsed.as_secs_f64() * 1000.0,
            );
        }
        FifAction::Replaced {
            files_changed,
            replacements,
            failures,
            query,
            replacement,
        } => {
            // Recargar tabs limpios cuyo path haya cambiado en disco.
            // Para tabs sucios el watcher externo se encarga del aviso.
            let touched_paths: std::collections::BTreeSet<PathBuf> =
                m.tabs.iter().map(|t| t.path.clone()).collect();
            for path in touched_paths {
                if let Some(idx) = m.tabs.iter().position(|t| t.path == path) {
                    if !m.tabs[idx].dirty {
                        if let Ok(content) = fs::read_to_string(&path) {
                            m.tabs[idx].editor.set_text(&content);
                            m.tabs[idx].last_mtime = file_mtime(&path);
                        }
                    }
                }
            }
            let fail_note = if failures > 0 {
                format!(" · {failures} archivos fallaron")
            } else {
                String::new()
            };
            m.status = format!(
                "reemplazo · «{query}» → «{replacement}» · {replacements} matches en {files_changed} archivos{fail_note}",
            );
        }
        FifAction::OpenAt { path, line, col } => {
            // Cerramos el dialog pero dejamos la barra de resultados viva
            // — el user puede saltar entre matches sin reescribir la query.
            if let Some(state) = m.fif.as_mut() {
                state.dialog_open = false;
            }
            m = open_path(m, path);
            if let Some(tab) = m.active_tab_mut() {
                tab.editor.set_caret_at(line, col);
                tab.editor.ensure_caret_visible(EDITOR_VISIBLE_LINES);
            }
        }
    }
    m
}

/// Cuántas filas del diff caben en su panel. El módulo necesita esto
/// para clampear el scroll; lo derivamos de [`DIFF_PANEL_H`] y la
/// altura de fila del módulo (15 px) — aproximación constante para
/// evitar tener que medir layout en el host.
pub(crate) const DIFF_VISIBLE_ROWS: usize = ((DIFF_PANEL_H - 18.0) / 15.0) as usize;

/// Routea un DiffMsg al módulo diff. Lazy-init: en `Open`, lee el
/// archivo de disco y compara contra el buffer actual. Snapshot
/// congelado — cambios subsecuentes del buffer no recomputan.
pub(crate) fn apply_diff(model: Model, dm: DiffMsg) -> Model {
    let mut m = model;
    if matches!(dm, DiffMsg::Open) && m.diff.is_none() {
        let Some(tab) = m.active_tab() else {
            m.status = "diff · ningún tab activo".into();
            return m;
        };
        let path = tab.path.clone();
        let after = tab.editor.text();
        let before = std::fs::read_to_string(&path).unwrap_or_default();
        let label_left = format!("disco · {}", path.file_name().and_then(|s| s.to_str()).unwrap_or("?"));
        let label_right = if tab.dirty { "buffer (●)" } else { "buffer" }.to_string();
        let state = DiffState::new(label_left, label_right, &before, &after);
        m.status = format!(
            "diff · +{} -{} ={} · ↑↓ scroll · n/N hunk · Esc cierra",
            state.stats.inserts, state.stats.deletes, state.stats.equals,
        );
        m.diff = Some(state);
        return m;
    }
    let action = match m.diff.as_mut() {
        Some(state) => diff::apply(state, dm, DIFF_VISIBLE_ROWS),
        None => return m,
    };
    if matches!(action, DiffAction::Close) {
        m.diff = None;
    }
    m
}

/// Convierte la lista de symbols que devuelve el LSP al tipo que el
/// módulo outline conoce. La estructura es 1:1; este shim sólo evita
/// que el módulo dependa del crate del LSP.
pub(crate) fn symbols_lsp_to_module(lsp: Vec<DocumentSymbolEntry>) -> Vec<SymbolItem> {
    lsp.into_iter()
        .map(|e| SymbolItem {
            name: e.name,
            kind: e.kind,
            line: e.line,
            col: e.col,
            container: e.container,
            depth: e.depth,
        })
        .collect()
}

/// Routea un OutlineMsg al módulo outline. Lazy-init en `Open`: si no
/// hay tab activo es no-op; si lo hay y todavía no llegaron symbols,
/// dispara `documentSymbol` en background — el PollLsp tick poblará
/// la lista cuando la respuesta llegue.
pub(crate) fn apply_outline(model: Model, om: OutlineMsg) -> Model {
    let mut m = model;
    if matches!(om, OutlineMsg::Open) && m.outline.is_none() {
        if m.active.is_none() {
            m.status = "outline · ningún tab activo".into();
            return m;
        }
        if let Some(path) = m.active_path() {
            m.lsp.request_document_symbols(&path);
        }
        m.outline = Some(OutlineState::new(&m.outline_symbols));
        m.status = if m.outline_symbols.is_empty() {
            "outline · pidiendo symbols al LSP… (sin LSP, queda vacío)".into()
        } else {
            format!("outline · {} símbolos", m.outline_symbols.len())
        };
        return m;
    }
    let action = match m.outline.as_mut() {
        Some(state) => outline::apply(state, om, &m.outline_symbols),
        None => return m,
    };
    match action {
        OutlineAction::None => {}
        OutlineAction::Close => m.outline = None,
        OutlineAction::GoTo { line, col } => {
            m.outline = None;
            if let Some(tab) = m.active_tab_mut() {
                tab.editor.set_caret_at(line, col);
                tab.editor.ensure_caret_visible(EDITOR_VISIBLE_LINES);
            }
        }
    }
    m
}

/// Catálogo de comandos que el palette muestra. Estático: lo construimos
/// una sola vez en `init` y vive en `Model.palette_commands`. Cada `id`
/// debe estar mapeado en [`palette_id_to_msg`] para que el invoke pueda
/// dispatchearse.

/// Routea un BookmarksMsg al modulo bookmarks. El state es
/// always-on (no Option), pero el overlay es opcional: OpenList
/// lo crea, CloseList lo cierra.
pub(crate) fn apply_bookmarks(model: Model, bm: BookmarksMsg) -> Model {
    let mut m = model;
    if matches!(bm, BookmarksMsg::OpenList) && m.bookmarks.overlay.is_none() {
        m.bookmarks.overlay = Some(BookmarksOverlay::new());
        bookmarks::refilter_overlay(&mut m.bookmarks);
        let n = m.bookmarks.marks.len();
        m.status = format!("bookmarks abierto - {} marks - Enter salta - Esc cierra", n);
        return m;
    }
    let action = bookmarks::apply(&mut m.bookmarks, bm);
    match action {
        BookmarksAction::None => {}
        BookmarksAction::Close => m.bookmarks.overlay = None,
        BookmarksAction::SetStatus(s) => m.status = s,
        BookmarksAction::JumpTo { path, line } => {
            m.bookmarks.overlay = None;
            m = open_path(m, path);
            if let Some(tab) = m.active_tab_mut() {
                let max_line = tab.editor.buffer.len_lines().saturating_sub(1);
                let target = line.min(max_line);
                tab.editor.set_caret_at(target, 0);
                tab.editor.ensure_caret_visible(EDITOR_VISIBLE_LINES);
            }
        }
    }
    m
}
pub(crate) fn apply_minimap(model: Model, mm: MiniMapMsg) -> Model {
    let mut m = model;
    if matches!(mm, MiniMapMsg::Open) && m.minimap.is_none() {
        if m.active.is_none() {
            m.status = "minimap: ningun tab activo".into();
            return m;
        }
        m.minimap = Some(MiniMapState::new());
        m.status = "minimap abierto - Ctrl+Shift+M cierra".into();
        return m;
    }
    let action = match m.minimap.as_mut() {
        Some(state) => minimap::apply(state, mm),
        None => return m,
    };
    match action {
        MiniMapAction::None => {}
        MiniMapAction::Close => m.minimap = None,
        MiniMapAction::JumpTo(line) => {
            if let Some(tab) = m.active_tab_mut() {
                let max_line = tab.editor.buffer.len_lines().saturating_sub(1);
                let target = line.min(max_line);
                tab.editor.set_caret_at(target, 0);
                tab.editor.ensure_caret_visible(EDITOR_VISIBLE_LINES);
            }
        }
    }
    m
}


/// Construye un vec de chars-per-line + viewport + caret_line para
/// el minimap del tab activo. Si no hay tab, todo vacio. O(lineas).
pub(crate) fn minimap_snapshot_data(model: &Model) -> (Vec<usize>, usize, usize, usize) {
    let Some(tab) = model.active_tab() else {
        return (Vec::new(), 0, 0, 0);
    };
    let total = tab.editor.buffer.len_lines();
    let lines: Vec<usize> = (0..total)
        .map(|i| tab.editor.buffer.line_len_chars(i))
        .collect();
    let scroll = tab.editor.scroll_offset;
    let caret = tab.editor.cursor.caret.line;
    (lines, scroll, scroll + EDITOR_VISIBLE_LINES, caret)
}

pub(crate) fn build_command_catalog() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand::new("editor.save", "Save File", "Editor").with_shortcut("Ctrl+S"),
        PaletteCommand::new("editor.saveAs", "Save File As…", "Editor")
            .with_shortcut("Ctrl+Shift+S"),
        PaletteCommand::new("editor.openFile", "Open File…", "Editor")
            .with_shortcut("Ctrl+P"),
        PaletteCommand::new("editor.findInFiles", "Find in Files", "Editor")
            .with_shortcut("Ctrl+Shift+F"),
        PaletteCommand::new("editor.find", "Find in File", "Editor").with_shortcut("Ctrl+F"),
        PaletteCommand::new("editor.closeTab", "Close Tab", "Editor").with_shortcut("Ctrl+W"),
        PaletteCommand::new("editor.nextTab", "Next Tab", "Editor").with_shortcut("Ctrl+Tab"),
        PaletteCommand::new("editor.prevTab", "Previous Tab", "Editor")
            .with_shortcut("Ctrl+Shift+Tab"),
        PaletteCommand::new("terminal.open", "Open Terminal", "Terminal")
            .with_shortcut("Ctrl+`"),
        PaletteCommand::new("lsp.format", "Format Document", "LSP")
            .with_shortcut("Ctrl+Alt+L"),
        PaletteCommand::new("lsp.goto", "Go to Definition", "LSP").with_shortcut("F12"),
        PaletteCommand::new("lsp.references", "Find References", "LSP")
            .with_shortcut("Shift+F12"),
        PaletteCommand::new("lsp.rename", "Rename Symbol", "LSP").with_shortcut("F2"),
        PaletteCommand::new("lsp.hover", "Show Hover Info", "LSP").with_shortcut("Ctrl+K"),
        PaletteCommand::new("lsp.signatureHelp", "Signature Help", "LSP")
            .with_shortcut("Ctrl+Shift+Space"),
        PaletteCommand::new("lsp.completions", "Trigger Suggest", "LSP")
            .with_shortcut("Ctrl+Space"),
        PaletteCommand::new("editor.outline", "Symbol Outline", "Editor")
            .with_shortcut("Ctrl+Shift+O"),
        PaletteCommand::new("editor.diff", "Compare with Saved", "Editor")
            .with_shortcut("Ctrl+Shift+D"),
        PaletteCommand::new("editor.miniMap", "Toggle Mini-Map", "Editor")
            .with_shortcut("Ctrl+Shift+M"),
        PaletteCommand::new("editor.bookmarkList", "List Bookmarks", "Editor")
            .with_shortcut("Ctrl+Shift+B"),
        PaletteCommand::new("editor.bookmarkClear", "Clear All Bookmarks", "Editor"),
        PaletteCommand::new("view.cycleTheme", "Cycle Theme", "View")
            .with_shortcut("Ctrl+Alt+T"),
    ]
}

/// Traduce un id de comando del catálogo al `Msg` correspondiente. Si
/// el id es desconocido, devuelve `None` y el host lo reporta como
/// status. Mantener en sync con [`build_command_catalog`].
pub(crate) fn palette_id_to_msg(id: &str) -> Option<Msg> {
    Some(match id {
        "editor.save" => Msg::Save,
        "editor.saveAs" => Msg::SaveAsOpen,
        "editor.openFile" => Msg::Picker(PickerMsg::Open),
        "editor.findInFiles" => Msg::Fif(FifMsg::Open),
        "editor.find" => Msg::FindOpen,
        "editor.closeTab" => Msg::CloseTab(usize::MAX), // será no-op si no hay tabs
        "editor.nextTab" => Msg::NextTab,
        "editor.prevTab" => Msg::PrevTab,
        "terminal.open" => Msg::Term(ShumaTermMsg::Open),
        "lsp.format" => Msg::FormatRequest,
        "lsp.goto" => Msg::GotoDefinitionRequest,
        "lsp.references" => Msg::ReferencesRequest,
        "lsp.rename" => Msg::RenameOpen,
        "lsp.hover" => Msg::HoverRequest,
        "lsp.signatureHelp" => Msg::SignatureHelpRequest,
        "lsp.completions" => Msg::CompletionsRequest,
        "editor.outline" => Msg::Outline(OutlineMsg::Open),
        "editor.diff" => Msg::Diff(DiffMsg::Open),
        "editor.miniMap" => Msg::MiniMap(MiniMapMsg::Open),
        "editor.bookmarkList" => Msg::Bookmarks(BookmarksMsg::OpenList),
        "editor.bookmarkClear" => Msg::Bookmarks(BookmarksMsg::ClearAll),
        "view.cycleTheme" => Msg::CycleTheme,
        _ => return None,
    })
}

/// Routea un PaletteMsg al módulo command-palette. Lazy-init en `Open`.
/// En `Invoke(id)`: cierra el palette y dispatcha el Msg correspondiente
/// — el comando se ejecuta en el siguiente turno del loop.
pub(crate) fn apply_palette(model: Model, pm: PaletteMsg, handle: &Handle<Msg>) -> Model {
    let mut m = model;
    if matches!(pm, PaletteMsg::Open) && m.palette.is_none() {
        m.palette = Some(PaletteState::new(&m.palette_commands));
        m.status = format!(
            "command palette · {} comandos · ↓↑ Enter ejecuta · Esc cierra",
            m.palette_commands.len(),
        );
        return m;
    }
    let action = match m.palette.as_mut() {
        Some(state) => palette::apply(state, pm, &m.palette_commands),
        None => return m,
    };
    match action {
        PaletteAction::None => {}
        PaletteAction::Close => m.palette = None,
        PaletteAction::Invoke(id) => {
            m.palette = None;
            match palette_id_to_msg(&id) {
                Some(msg) => handle.dispatch(msg),
                None => m.status = format!("comando desconocido: {id}"),
            }
        }
    }
    m
}

/// Routea un ShumaTermMsg al módulo terminal. Lazy-init: el shell se
/// spawnea en la raíz del workspace cuando el user dispara Ctrl+`.
pub(crate) fn apply_term(model: Model, tm: ShumaTermMsg) -> Model {
    let mut m = model;
    if matches!(tm, ShumaTermMsg::Open) && m.term.is_none() {
        let cwd = m.root.display().to_string();
        m.term = Some(term::spawn(cwd));
        m.status = "terminal · Ctrl+` cierra · Ctrl+Shift+W cierra".into();
        return m;
    }
    let action = match m.term.as_mut() {
        Some(state) => term::apply(state, tm),
        None => return m,
    };
    match action {
        ShumaTermAction::None => {}
        ShumaTermAction::Close => {
            // Drop del state envía SIGTERM al shell — ver Drop impl del módulo.
            m.term = None;
            m.status = "terminal cerrado".into();
        }
        ShumaTermAction::SetStatus(s) => m.status = s,
    }
    m
}

/// Activa el tab `idx` si es válido. No-op si está fuera de rango.
pub(crate) fn activate_tab(mut model: Model, idx: usize) -> Model {
    if idx < model.tabs.len() {
        model.active = Some(idx);
        // Limpiamos popups anclados al tab anterior — anchor era una pos
        // específica que ya no aplica.
        model.completions = None;
        model.hover = None;
        model.sig_help = None;
        model.references = None;
        model.rename = None;
        model.lsp.clear_completions();
        model.lsp.clear_hover();
        model.lsp.clear_signature_help();
        model.lsp.clear_references();
        model.lsp.clear_workspace_edit();
    }
    model
}

/// Cierra el tab `idx`. Notifica `did_close` al LSP, reajusta `active`,
/// y limpia popups si era el activo.
pub(crate) fn close_tab(mut model: Model, idx: usize) -> Model {
    if idx >= model.tabs.len() {
        return model;
    }
    let was_active = model.active == Some(idx);
    let closed_path = model.tabs[idx].path.clone();
    model.tabs.remove(idx);
    model.lsp.did_close(&closed_path);
    // Reajustamos `active`:
    //  - si quedaron 0 tabs: None.
    //  - si cerramos el activo: nuevo activo = min(idx, len-1).
    //  - si cerramos uno previo al activo: active baja 1.
    //  - si cerramos uno posterior al activo: queda igual.
    model.active = if model.tabs.is_empty() {
        None
    } else if was_active {
        Some(idx.min(model.tabs.len() - 1))
    } else {
        model.active.map(|a| if a > idx { a - 1 } else { a })
    };
    if was_active {
        model.completions = None;
        model.hover = None;
        model.sig_help = None;
        model.references = None;
        model.rename = None;
        model.lsp.clear_completions();
        model.lsp.clear_hover();
        model.lsp.clear_signature_help();
        model.lsp.clear_references();
        model.lsp.clear_workspace_edit();
    }
    model.status = format!("cerrado · {}", relative_to(&model.root, &closed_path));
    model
}

/// Tres diagnostics fake repartidos en las primeras líneas — Error,
/// Warning, Info. Solo para validar el render del subrayado.
pub(crate) fn demo_diagnostics(content: &str) -> Vec<Diagnostic> {
    use llimphi_widget_text_editor::Severity;
    let mut out = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate().take(20) {
        if line.contains("TODO") {
            out.push(Diagnostic {
                range: llimphi_widget_text_editor::DiagnosticRange {
                    start: Pos::new(i, 0),
                    end: Pos::new(i, line.chars().count()),
                },
                severity: Severity::Warning,
                message: "TODO pendiente".into(),
                source: Some("demo".into()),
            });
        }
        if line.contains("FIXME") {
            out.push(Diagnostic {
                range: llimphi_widget_text_editor::DiagnosticRange {
                    start: Pos::new(i, 0),
                    end: Pos::new(i, line.chars().count()),
                },
                severity: Severity::Error,
                message: "FIXME crítico".into(),
                source: Some("demo".into()),
            });
        }
    }
    out
}

pub(crate) fn apply_editor_key(mut model: Model, ev: KeyEvent) -> Model {
    let Some(idx) = model.active else { return model };
    let r = model.tabs[idx]
        .editor
        .apply_key_with_clipboard(&ev, &mut model.clipboard);
    if r.changed() {
        model.tabs[idx].dirty = true;
        let path = model.tabs[idx].path.clone();
        let text = model.tabs[idx].editor.text();
        model.lsp.did_change(&path, &text);
    }
    if r.touched() {
        model.tabs[idx].editor.ensure_caret_visible(EDITOR_VISIBLE_LINES);
    }
    // Si el popup de completions está abierto, actualizamos el filter
    // según el prefijo actual del caret. Si no quedan matches → cerramos.
    if let Some(bar) = model.completions.as_mut() {
        let line = model.tabs[idx].editor.cursor.caret.line;
        let col = model.tabs[idx].editor.cursor.caret.col;
        let (_, prefix) = model.tabs[idx].editor.buffer.current_word_prefix(line, col);
        bar.filter = prefix;
        let filtered = bar.filtered_indices();
        if filtered.is_empty() && !bar.items.is_empty() {
            model.completions = None;
            model.lsp.clear_completions();
        } else {
            bar.selected = 0;
        }
    }
    // Pull diagnostics actuales del LSP. Es barato — sólo lee del state
    // compartido.
    let path = model.tabs[idx].path.clone();
    let diags = model.lsp.diagnostics(&path);
    if !diags.is_empty() || !model.tabs[idx].editor.diagnostics.is_empty() {
        model.tabs[idx].editor.set_diagnostics(diags);
    }
    model
}

pub(crate) fn apply_editor_pointer(mut model: Model, ev: PointerEvent) -> Model {
    let Some(idx) = model.active else { return model };
    let metrics = EditorMetrics::for_font_size(13.0);
    let scroll = model.tabs[idx].editor.scroll_offset;
    match ev {
        PointerEvent::Click { x, y } => {
            model.drag_accum = (0.0, 0.0);
            let (line, col) = metrics.screen_to_pos(x, y, scroll);
            model.tabs[idx].editor.set_caret_at(line, col);
        }
        PointerEvent::Drag { initial_x, initial_y, dx, dy } => {
            model.drag_accum.0 += dx;
            model.drag_accum.1 += dy;
            let cur_x = initial_x + model.drag_accum.0;
            let cur_y = initial_y + model.drag_accum.1;
            let (line, col) = metrics.screen_to_pos(cur_x, cur_y, scroll);
            model.tabs[idx].editor.extend_selection_to(line, col);
        }
    }
    model
}

/// Aplica una lista de TextEdits al EditorState en orden descendente
/// por start offset (las edits tempranas no desplazan posiciones
/// posteriores). Cada TextEdit es un reemplazo [start..end) → new_text.
/// Aplica edits a un archivo del disco (no abierto). Carga, aplica
/// ordenados desc por start, escribe atómico (write + fsync no, simple).
pub(crate) fn apply_text_edits_to_file(path: &Path, edits: &[TextEdit]) -> std::io::Result<usize> {
    let content = fs::read_to_string(path)?;
    let mut buf = llimphi_widget_text_editor::Buffer::from_str(&content);
    let mut sorted: Vec<TextEdit> = edits.to_vec();
    sorted.sort_by(|a, b| {
        let oa = buf.pos_to_offset(a.start_line, a.start_col);
        let ob = buf.pos_to_offset(b.start_line, b.start_col);
        ob.cmp(&oa)
    });
    for e in sorted {
        let s = buf.pos_to_offset(e.start_line, e.start_col);
        let en = buf.pos_to_offset(e.end_line, e.end_col);
        if en > s {
            buf.delete(s, en);
        }
        if !e.new_text.is_empty() {
            buf.insert(s, &e.new_text);
        }
    }
    let new_text = buf.text();
    let len = new_text.len();
    fs::write(path, new_text)?;
    Ok(len)
}

pub(crate) fn apply_text_edits_in_place(editor: &mut EditorState, mut edits: Vec<TextEdit>) {
    // Ordenar desc por start.
    edits.sort_by(|a, b| {
        let oa = editor.buffer.pos_to_offset(a.start_line, a.start_col);
        let ob = editor.buffer.pos_to_offset(b.start_line, b.start_col);
        ob.cmp(&oa)
    });
    for e in edits {
        let start_off = editor.buffer.pos_to_offset(e.start_line, e.start_col);
        let end_off = editor.buffer.pos_to_offset(e.end_line, e.end_col);
        if end_off > start_off {
            editor.buffer.delete(start_off, end_off);
        }
        if !e.new_text.is_empty() {
            editor.buffer.insert(start_off, &e.new_text);
        }
    }
    editor.bump_edit_seq();
    // Clampea el caret a la nueva longitud.
    let last_line = editor.buffer.len_lines().saturating_sub(1);
    let max_col = editor.buffer.line_len_chars(editor.cursor.caret.line.min(last_line));
    editor.cursor.caret.col = editor.cursor.caret.col.min(max_col);
}

pub(crate) fn find_step(mut model: Model, forward: bool) -> Model {
    let Some(idx) = model.active else { return model };
    let Some(find) = model.find.as_ref() else { return model };
    if find.state.query.is_empty() {
        return model;
    }
    let tab_buf = &model.tabs[idx].editor.buffer;
    let tab_cursor = &model.tabs[idx].editor.cursor;
    let result = if forward {
        find_next(tab_buf, &find.state, tab_cursor)
    } else {
        find_prev(tab_buf, &find.state, tab_cursor)
    };
    let Some((start, end)) = result else {
        model.status = format!("sin matches para «{}»", find.state.query);
        return model;
    };
    let total = all_matches(&model.tabs[idx].editor.buffer, &find.state).len();
    // Selecciona la match (anchor=start, caret=end) y la deja visible.
    let tab = &mut model.tabs[idx];
    tab.editor.cursor.anchor = Some(Pos::new(start.line, start.col));
    tab.editor.cursor.caret = Pos::new(end.line, end.col);
    tab.editor.cursor.desired_col = end.col;
    tab.editor.ensure_caret_visible(EDITOR_VISIBLE_LINES);
    model.status = format!("match · {total} totales");
    model
}

pub(crate) fn save_open_file(model: Model, handle: &Handle<Msg>) -> Model {
    let Some(tab) = model.active_tab() else {
        return model;
    };
    let path = tab.path.clone();
    let content = tab.editor.text();
    let h = handle.clone();
    handle.spawn(move || {
        let result = fs::write(&path, content).map_err(|e| e.to_string());
        Msg::SaveResult(result)
    });
    let _ = h;
    let mut m = model;
    m.status = "guardando…".to_string();
    m
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

/// mtime del archivo si se puede leer. `None` si no existe o falla
/// metadata — el watcher trata ambos casos igual: ignora.
