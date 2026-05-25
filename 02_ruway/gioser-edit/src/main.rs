//! `gioser-edit` — editor de archivos rudimentario sobre Llimphi.
//!
//! - **Tree** a la izquierda (220 px) con el contenido del directorio
//!   `cwd` (o el primer argumento). Click expande/colapsa directorios;
//!   click en archivo lo carga al editor.
//! - **Editor** a la derecha: text-editor multilínea con syntax highlight
//!   derivado de la extensión (`.rs` → Rust, `.py` → Python, `.wat` → Wat,
//!   resto → Plain). Caret, selección, bracket matching, gutter con
//!   line numbers, undo/redo, copy/cut/paste.
//! - **Atajos** dentro del editor: arrows + Shift/Ctrl, Home/End,
//!   Ctrl+Home/End, PageUp/Down, Tab/Shift+Tab, Backspace/Delete,
//!   Ctrl+Z/Y/Shift+Z (undo/redo), Ctrl+C/X/V (clipboard del sistema
//!   vía arboard). **Ctrl+S guarda** el archivo abierto.
//!
//! Limitaciones MVP: el tree se construye al arrancar (no watcher), un
//! solo archivo abierto a la vez (sin tabs), no marca "modified", no
//! confirma overwrites externos.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Rect, Size, Style},
    AlignItems,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_text_editor::{
    all_matches, find_next, find_prev, text_editor_view_full, Clipboard, Diagnostic,
    EditorMetrics, EditorPalette, EditorState, FindState, Language, PointerEvent, Pos,
};
use llimphi_widget_text_editor_lsp::{
    CompletionItem, DefinitionLocation, HoverInfo, LspClient, NoopLspClient, RustAnalyzerClient,
    SignatureHelpInfo, TextEdit,
};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

const TREE_WIDTH: f32 = 240.0;
const TREE_ROW_H: f32 = 22.0;
const TREE_INDENT: f32 = 16.0;
const HEADER_H: f32 = 28.0;
/// Cuántas líneas mostramos en el viewport del editor. Aproximación
/// estática: (alto ventana ~760 − header 28) / line_height(~18) ≈ 40.
const EDITOR_VISIBLE_LINES: usize = 40;

#[derive(Clone)]
enum Msg {
    ToggleNode(usize),
    SelectNode(usize),
    EditKey(KeyEvent),
    EditorPointer(PointerEvent),
    Save,
    SaveResult(Result<(), String>),
    Scroll(i32),
    // Find
    FindOpen,
    FindClose,
    FindKey(KeyEvent),
    FindNext,
    FindPrev,
    /// Tick periódico — pull de diagnostics + completions del LSP.
    PollLsp,
    /// Ctrl+Space — pide completions al LSP en la pos del caret.
    CompletionsRequest,
    /// El usuario navega el dropdown de completions.
    CompletionsNav { delta: i32 },
    /// Aplica el item seleccionado (Enter).
    CompletionsApply,
    /// Cierra el dropdown.
    CompletionsClose,
    /// Ctrl+K — pide hover en pos del caret.
    HoverRequest,
    /// Cierra el popup de hover (Esc, o cambio de cursor).
    HoverClose,
    /// F12 — pide goto-definition. Cuando llega, abre el archivo
    /// destino y posiciona el caret.
    GotoDefinitionRequest,
    /// El LSP devolvió la definition — abrir destino + posicionar.
    GotoDefinitionApply(DefinitionLocation),
    /// Ctrl+Shift+F — pide formatting.
    FormatRequest,
    /// Ctrl+Shift+Space — pide signatureHelp.
    SignatureHelpRequest,
    SignatureHelpClose,
    /// Shift+F12 — pide references al símbolo en el caret.
    ReferencesRequest,
    ReferencesNav { delta: i32 },
    ReferencesApply,
    ReferencesClose,
    /// F2 — abre prompt para escribir el nuevo nombre.
    RenameOpen,
    RenameKey(KeyEvent),
    RenameSubmit,
    RenameClose,
    /// Aplicar WorkspaceEdit (de rename) en N archivos.
    RenameApply(std::collections::HashMap<PathBuf, Vec<TextEdit>>),
    /// El LSP devolvió text edits (de formatting o rename) para el
    /// archivo abierto — aplicar todos en orden descendente.
    TextEditsApply(Vec<TextEdit>),
}

#[derive(Debug, Clone)]
struct TreeNode {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
    expanded: bool,
}

struct Model {
    root: PathBuf,
    nodes: Vec<TreeNode>,
    selected: Option<usize>,
    open_file: Option<PathBuf>,
    editor: EditorState,
    clipboard: ArboardClipboard,
    status: String,
    dirty: bool,
    /// Acumulado de drag del editor: cada `Msg::EditorPointer(Drag)`
    /// suma `(dx, dy)`. Pos actual = `initial + drag_accum`.
    drag_accum: (f32, f32),
    /// Modo find: cuando es Some, la barra del find está abierta y las
    /// teclas van al input en lugar de al editor.
    find: Option<FindBarState>,
    /// Demo de diagnostics fake (--demo-lsp): TODO/FIXME en .rs/.py se
    /// pintan como warning/error. Útil cuando no hay rust-analyzer y
    /// querés ver el render del subrayado.
    demo_lsp: bool,
    /// Cliente LSP real: `--lsp` spawnea rust-analyzer (o el binary
    /// pasado con `--lsp-cmd=...`). En modo no-op cuando no se pide.
    lsp: Box<dyn LspClient>,
    /// Items del popup de completions; `None` si el popup está cerrado.
    completions: Option<CompletionsBar>,
    /// Popup de hover; `None` cerrado.
    hover: Option<HoverPopup>,
    /// Popup de signatureHelp; `None` cerrado.
    sig_help: Option<SignatureHelpBar>,
    /// Lista de references; `None` cerrada.
    references: Option<ReferencesBar>,
    /// Prompt de rename con el nuevo nombre + pos original; `None` cerrado.
    rename: Option<RenameBar>,
}

struct RenameBar {
    input: TextInputState,
    /// Pos donde se pidió el rename.
    anchor: (usize, usize),
    /// `true` mientras esperamos la respuesta del LSP tras submit.
    waiting: bool,
}

struct ReferencesBar {
    items: Vec<DefinitionLocation>,
    selected: usize,
    /// Pos donde se pidió la búsqueda.
    anchor: (usize, usize),
}

struct SignatureHelpBar {
    info: Option<SignatureHelpInfo>,
    anchor: (usize, usize),
}

struct HoverPopup {
    info: Option<HoverInfo>,
    anchor: (usize, usize),
}

struct CompletionsBar {
    items: Vec<CompletionItem>,
    selected: usize,
    /// Pos donde se pidió la completion — para anclar el popup visual.
    anchor: (usize, usize),
    /// Prefijo actual derivado del buffer en cada frame. Se filtran
    /// los items por `label.to_lowercase().contains(filter.to_lowercase())`.
    filter: String,
}

impl CompletionsBar {
    fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.items.len()).collect();
        }
        let f = self.filter.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, c)| c.label.to_lowercase().contains(&f))
            .map(|(i, _)| i)
            .collect()
    }
}

struct FindBarState {
    input: TextInputState,
    state: FindState,
}

impl FindBarState {
    fn new() -> Self {
        Self { input: TextInputState::new(), state: FindState::new() }
    }
    /// Sincroniza el FindState con el contenido actual del input.
    fn sync(&mut self) {
        self.state.query = self.input.text();
    }
}

struct EditorApp;

impl App for EditorApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "gioser-edit"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 760)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        // Tick periódico para refrescar diagnostics del LSP.
        handle.spawn_periodic(std::time::Duration::from_millis(400), || Msg::PollLsp);

        let args: Vec<String> = env::args().skip(1).collect();
        let demo_lsp = args.iter().any(|a| a == "--demo-lsp");
        let lsp_on = args.iter().any(|a| a == "--lsp");
        let lsp_cmd = args
            .iter()
            .find_map(|a| a.strip_prefix("--lsp-cmd=").map(|s| s.to_string()))
            .unwrap_or_else(|| "rust-analyzer".to_string());
        let root = args
            .iter()
            .find(|a| !a.starts_with("--"))
            .map(PathBuf::from)
            .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let root = fs::canonicalize(&root).unwrap_or(root);
        let nodes = scan_root(&root);
        let lsp: Box<dyn LspClient> = if lsp_on {
            Box::new(RustAnalyzerClient::with_command(root.clone(), &lsp_cmd))
        } else {
            Box::new(NoopLspClient)
        };
        let lsp_label = if lsp_on { format!("lsp:{lsp_cmd}") } else { "lsp:off".into() };
        let status = format!("{} · {} entradas · {lsp_label}", root.display(), nodes.len());
        Model {
            root,
            nodes,
            selected: None,
            open_file: None,
            editor: EditorState::new(),
            clipboard: ArboardClipboard::new(),
            status,
            dirty: false,
            drag_accum: (0.0, 0.0),
            find: None,
            demo_lsp,
            lsp,
            completions: None,
            hover: None,
            sig_help: None,
            references: None,
            rename: None,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::ToggleNode(i) => toggle_node(model, i),
            Msg::SelectNode(i) => select_node(model, i),
            Msg::EditKey(ev) => apply_editor_key(model, ev),
            Msg::EditorPointer(ev) => apply_editor_pointer(model, ev),
            Msg::Save => save_open_file(model, handle),
            Msg::Scroll(delta) => {
                let mut m = model;
                m.editor.scroll_by(delta);
                m
            }
            Msg::FindOpen => {
                let mut m = model;
                if m.find.is_none() {
                    m.find = Some(FindBarState::new());
                    m.status = "find · Ctrl+G siguiente · Esc cierra".to_string();
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
                if let Some(path) = m.open_file.as_ref() {
                    let diags = m.lsp.diagnostics(path);
                    if diags != m.editor.diagnostics {
                        m.editor.set_diagnostics(diags);
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
                m
            }
            Msg::CompletionsRequest => {
                let mut m = model;
                let Some(path) = m.open_file.clone() else { return m };
                let line = m.editor.cursor.caret.line;
                let col = m.editor.cursor.caret.col;
                m.lsp.clear_completions();
                m.lsp.request_completions(&path, line, col);
                let (_, prefix) = m.editor.buffer.current_word_prefix(line, col);
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
                let line = m.editor.cursor.caret.line;
                let caret_col = m.editor.cursor.caret.col;
                let (word_start, _) = m.editor.buffer.current_word_prefix(line, caret_col);
                if word_start < caret_col {
                    m.editor.cursor.anchor =
                        Some(llimphi_widget_text_editor::Pos::new(line, word_start));
                    m.editor.cursor.caret =
                        llimphi_widget_text_editor::Pos::new(line, caret_col);
                }
                let _ = llimphi_widget_text_editor::ops::replace_selection(
                    &mut m.editor.buffer,
                    &mut m.editor.cursor,
                    &text,
                );
                m.editor.bump_edit_seq();
                m.dirty = true;
                if let Some(path) = m.open_file.clone() {
                    let new_text = m.editor.text();
                    m.lsp.did_change(&path, &new_text);
                }
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
                let Some(path) = m.open_file.clone() else { return m };
                let line = m.editor.cursor.caret.line;
                let col = m.editor.cursor.caret.col;
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
                let Some(path) = m.open_file.clone() else { return m };
                let line = m.editor.cursor.caret.line;
                let col = m.editor.cursor.caret.col;
                m.lsp.clear_definition();
                m.lsp.request_definition(&path, line, col);
                m.status = "goto-def · esperando LSP…".into();
                m
            }
            Msg::ReferencesRequest => {
                let mut m = model;
                let Some(path) = m.open_file.clone() else { return m };
                let line = m.editor.cursor.caret.line;
                let col = m.editor.cursor.caret.col;
                m.lsp.clear_references();
                m.lsp.request_references(&path, line, col, true);
                m.references = Some(ReferencesBar {
                    items: Vec::new(),
                    selected: 0,
                    anchor: (line, col),
                });
                m.status = "references · esperando LSP…".into();
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
                        return Self::update(m2, Msg::GotoDefinitionApply(loc), handle);
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
                if m.open_file.is_none() {
                    return m;
                }
                let line = m.editor.cursor.caret.line;
                let col = m.editor.cursor.caret.col;
                let (start, word) = m.editor.buffer.current_word_prefix(line, col);
                let _ = start;
                let mut input = TextInputState::new();
                input.set_text(&word);
                m.rename = Some(RenameBar {
                    input,
                    anchor: (line, col),
                    waiting: false,
                });
                m.status = "rename · Enter aplica · Esc cancela".into();
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
                let Some(r) = m.rename.as_mut() else { return m };
                let Some(path) = m.open_file.clone() else { return m };
                let new_name = r.input.text();
                if new_name.is_empty() {
                    return m;
                }
                m.lsp.clear_workspace_edit();
                m.lsp.request_rename(&path, r.anchor.0, r.anchor.1, &new_name);
                r.waiting = true;
                m.status = format!("rename → «{new_name}» · esperando LSP…");
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
                let open_path = m.open_file.clone();
                for (path, edits) in we {
                    if Some(&path) == open_path.as_ref() {
                        apply_text_edits_in_place(&mut m.editor, edits);
                        m.dirty = true;
                        if let Some(p) = open_path.clone() {
                            let new_text = m.editor.text();
                            m.lsp.did_change(&p, &new_text);
                        }
                        files_changed += 1;
                    } else {
                        match apply_text_edits_to_file(&path, &edits) {
                            Ok(n) => {
                                files_changed += 1;
                                bytes_written += n;
                            }
                            Err(e) => {
                                m.status = format!("rename · error en {}: {e}", path.display());
                                return m;
                            }
                        }
                    }
                }
                m.status = format!("rename · {files_changed} archivos · {bytes_written} bytes");
                m
            }
            Msg::SignatureHelpRequest => {
                let mut m = model;
                let Some(path) = m.open_file.clone() else { return m };
                let line = m.editor.cursor.caret.line;
                let col = m.editor.cursor.caret.col;
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
                let Some(path) = m.open_file.clone() else { return m };
                m.lsp.clear_text_edits();
                m.lsp.request_formatting(&path, 4, true);
                m.status = "formatting · esperando LSP…".into();
                m
            }
            Msg::TextEditsApply(edits) => {
                let mut m = model;
                apply_text_edits_in_place(&mut m.editor, edits);
                m.dirty = true;
                if let Some(path) = m.open_file.clone() {
                    let new_text = m.editor.text();
                    m.lsp.did_change(&path, &new_text);
                }
                m.status = "formatting · aplicado".into();
                m
            }
            Msg::GotoDefinitionApply(loc) => {
                let mut m = model;
                m.lsp.clear_definition();
                match fs::read_to_string(&loc.path) {
                    Ok(content) => {
                        // Si está abriendo otro archivo, did_close al previo.
                        if let Some(prev) = m.open_file.take() {
                            if prev != loc.path {
                                m.lsp.did_close(&prev);
                            }
                        }
                        let was_open = m.open_file.is_some();
                        m.editor = EditorState::new();
                        m.editor.set_text(&content);
                        m.editor.set_caret_at(loc.line, loc.col);
                        m.editor.ensure_caret_visible(EDITOR_VISIBLE_LINES);
                        if !was_open {
                            let ext = loc.path.extension().and_then(|s| s.to_str()).unwrap_or("");
                            m.lsp.did_open(&loc.path, ext, &content);
                        }
                        m.open_file = Some(loc.path.clone());
                        m.dirty = false;
                        m.status = format!("goto-def · {}:{}", loc.path.display(), loc.line + 1);
                    }
                    Err(e) => {
                        m.status = format!("goto-def · error abriendo {}: {e}", loc.path.display());
                    }
                }
                m
            }
            Msg::SaveResult(r) => {
                let mut m = model;
                m.status = match r {
                    Ok(()) => {
                        m.dirty = false;
                        format!(
                            "guardado · {}",
                            m.open_file.as_deref().map(Path::display).map(|d| d.to_string()).unwrap_or_default()
                        )
                    }
                    Err(e) => format!("error guardando: {e}"),
                };
                m
            }
        }
    }

    fn on_wheel(
        model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Self::Msg> {
        if model.open_file.is_none() {
            return None;
        }
        // llimphi-ui ya invierte el signo de winit (`y: -y` en LineDelta).
        // Por convención llimphi, delta.y > 0 = rueda hacia abajo = scroll
        // contenido hacia abajo. Sin inversión adicional.
        let lines = (delta.y * 3.0).round() as i32;
        if lines == 0 {
            None
        } else {
            Some(Msg::Scroll(lines))
        }
    }

    fn on_key(model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if event.state != KeyState::Pressed {
            return None;
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

        // Atajos globales
        if event.modifiers.ctrl {
            if matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("s")) {
                return Some(Msg::Save);
            }
            if !event.modifiers.shift
                && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("f"))
                && model.open_file.is_some()
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
                && model.open_file.is_some()
            {
                return Some(Msg::CompletionsRequest);
            }
            // Ctrl+K pide hover en la pos del caret.
            if matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("k"))
                && model.open_file.is_some()
            {
                return Some(Msg::HoverRequest);
            }
            // Ctrl+Shift+F = format. (Ctrl+F sin Shift sigue siendo find.)
            if event.modifiers.shift
                && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("f"))
                && model.open_file.is_some()
            {
                return Some(Msg::FormatRequest);
            }
            // Ctrl+Shift+Space = signatureHelp.
            if event.modifiers.shift
                && matches!(&event.key, Key::Named(NamedKey::Space))
                && model.open_file.is_some()
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
            && model.open_file.is_some()
        {
            return Some(if event.modifiers.shift {
                Msg::ReferencesRequest
            } else {
                Msg::GotoDefinitionRequest
            });
        }
        // F2 = rename.
        if matches!(&event.key, Key::Named(NamedKey::F2))
            && model.open_file.is_some()
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
            && model.editor.has_multi_cursor()
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

        if model.open_file.is_none() {
            return None;
        }
        Some(Msg::EditKey(event.clone()))
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let header = header_bar(model, &theme);
        let body = body_view(model, &theme);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, body])
    }
}

fn header_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let open = model
        .open_file
        .as_deref()
        .map(|p| relative_to(&model.root, p))
        .unwrap_or_else(|| "(sin archivo abierto)".to_string());
    let dirty = if model.dirty { " · ● modificado" } else { "" };
    let text = format!(
        "gioser-edit · {} · {}{}  ·  {}",
        model.root.display(),
        open,
        dirty,
        model.status,
    );
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(HEADER_H) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(text, 11.0, theme.fg_muted, Alignment::Start)
}

fn body_view(model: &Model, theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![tree_panel(model, theme), editor_panel(model, theme)])
}

fn tree_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let rows: Vec<TreeRow<Msg>> = model
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| TreeRow {
            label: row_label(n),
            depth: n.depth,
            has_children: n.is_dir,
            expanded: n.expanded,
            selected: model.selected == Some(i),
            on_toggle: Msg::ToggleNode(i),
            on_select: Msg::SelectNode(i),
        })
        .collect();

    let spec = TreeSpec {
        rows,
        row_height: TREE_ROW_H,
        indent_px: TREE_INDENT,
        palette: TreePalette::from_theme(theme),
    };

    View::new(Style {
        size: Size { width: length(TREE_WIDTH), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![tree_view(spec)])
}

fn editor_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let mut children: Vec<View<Msg>> = Vec::new();
    if let Some(find) = model.find.as_ref() {
        children.push(find_bar(find, theme));
    }
    if let Some(bar) = model.completions.as_ref() {
        children.push(completions_bar_view(bar, theme));
    }
    if let Some(hp) = model.hover.as_ref() {
        children.push(hover_view(hp, theme));
    }
    if let Some(bar) = model.sig_help.as_ref() {
        children.push(sig_help_view(bar, theme));
    }
    if let Some(rb) = model.references.as_ref() {
        children.push(references_view(rb, &model.root, theme));
    }
    if let Some(rn) = model.rename.as_ref() {
        children.push(rename_view(rn, theme));
    }
    let editor_view = match &model.open_file {
        None => empty_editor_placeholder(theme),
        Some(path) => {
            let language = language_for_path(path);
            let palette = EditorPalette::from_theme(theme);
            let metrics = EditorMetrics::for_font_size(13.0);
            let matches: Vec<(usize, usize)> = model
                .find
                .as_ref()
                .filter(|f| !f.state.query.is_empty())
                .map(|f| all_matches(&model.editor.buffer, &f.state))
                .unwrap_or_default();
            text_editor_view_full(
                &model.editor,
                &palette,
                metrics,
                EDITOR_VISIBLE_LINES,
                language,
                &matches,
                |ev| Some(Msg::EditorPointer(ev)),
            )
        }
    };
    children.push(editor_view);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}

const FIND_BAR_H: f32 = 32.0;
const COMPLETIONS_BAR_H: f32 = 120.0;
const COMPLETIONS_ROW_H: f32 = 22.0;
const COMPLETIONS_MAX_ITEMS_VISIBLE: usize = 5;

const HOVER_BAR_H: f32 = 96.0;
const SIG_HELP_BAR_H: f32 = 56.0;
const REFS_BAR_H: f32 = 160.0;
const RENAME_BAR_H: f32 = 56.0;

fn rename_view(rb: &RenameBar, theme: &Theme) -> View<Msg> {
    let tp = TextInputPalette::from_theme(theme);
    let header = if rb.waiting {
        format!("rename @ {}:{} · esperando LSP…", rb.anchor.0 + 1, rb.anchor.1)
    } else {
        format!("rename @ {}:{} · Enter aplica · Esc cancela", rb.anchor.0 + 1, rb.anchor.1)
    };
    let header_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        padding: Rect {
            left: length(8.0_f32), right: length(8.0_f32),
            top: length(0.0_f32), bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(header, 10.0, theme.fg_muted, Alignment::Start);

    let input_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(RENAME_BAR_H - 18.0) },
        padding: Rect {
            left: length(6.0_f32), right: length(6.0_f32),
            top: length(2.0_f32), bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![text_input_view(
        &rb.input,
        "nuevo nombre",
        true,
        &tp,
        Msg::RenameOpen,
    )]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(RENAME_BAR_H) },
        ..Default::default()
    })
    .children(vec![header_view, input_view])
}
const REFS_ROW_H: f32 = 20.0;
const REFS_MAX_VISIBLE: usize = 7;

fn references_view(bar: &ReferencesBar, root: &Path, theme: &Theme) -> View<Msg> {
    let header = if bar.items.is_empty() {
        format!(
            "references @ {}:{} · esperando LSP…",
            bar.anchor.0 + 1, bar.anchor.1,
        )
    } else {
        format!(
            "references @ {}:{} · {} / {} · ↓↑ navega · Enter abre · Esc cierra",
            bar.anchor.0 + 1, bar.anchor.1,
            bar.selected + 1, bar.items.len(),
        )
    };
    let mut rows: Vec<View<Msg>> = Vec::new();
    rows.push(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
            padding: Rect {
                left: length(8.0_f32), right: length(8.0_f32),
                top: length(0.0_f32), bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(theme.bg_panel_alt)
        .text_aligned(header, 10.0, theme.fg_muted, Alignment::Start),
    );
    let visible_start = bar.selected.saturating_sub(REFS_MAX_VISIBLE.saturating_sub(1));
    let visible_end = (visible_start + REFS_MAX_VISIBLE).min(bar.items.len());
    for i in visible_start..visible_end {
        let loc = &bar.items[i];
        let selected = i == bar.selected;
        let bg = if selected { theme.bg_selected } else { theme.bg_panel };
        let label = format!("{}:{}", relative_to(root, &loc.path), loc.line + 1);
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(REFS_ROW_H) },
                padding: Rect {
                    left: length(10.0_f32), right: length(8.0_f32),
                    top: length(0.0_f32), bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(bg)
            .text_aligned(label, 11.0, theme.fg_text, Alignment::Start),
        );
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(REFS_BAR_H) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(rows)
}

fn sig_help_view(bar: &SignatureHelpBar, theme: &Theme) -> View<Msg> {
    let header = format!(
        "signatureHelp @ {}:{} · Esc cierra",
        bar.anchor.0 + 1,
        bar.anchor.1,
    );
    let body_text = match bar.info.as_ref() {
        None => "esperando LSP…".to_string(),
        Some(info) => {
            let active = info
                .param_labels
                .get(info.active_param)
                .map(|s| format!(" · activo: «{s}»"))
                .unwrap_or_default();
            format!("{}{active}", info.label)
        }
    };
    let header_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(header, 10.0, theme.fg_muted, Alignment::Start);
    let body_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(SIG_HELP_BAR_H - 18.0) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(body_text, 12.0, theme.fg_text, Alignment::Start);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(SIG_HELP_BAR_H) },
        ..Default::default()
    })
    .children(vec![header_view, body_view])
}

fn hover_view(hp: &HoverPopup, theme: &Theme) -> View<Msg> {
    let header = format!(
        "hover @ {}:{} · Esc cierra",
        hp.anchor.0 + 1,
        hp.anchor.1,
    );
    let body_text = match hp.info.as_ref() {
        None => "esperando LSP…".to_string(),
        Some(info) => truncate_hover(&info.contents, 600),
    };

    let header_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(header, 10.0, theme.fg_muted, Alignment::Start);

    let body_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(HOVER_BAR_H - 18.0) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(body_text, 11.0, theme.fg_text, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(HOVER_BAR_H) },
        ..Default::default()
    })
    .children(vec![header_view, body_view])
}

fn truncate_hover(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max - 1).collect();
        format!("{cut}…")
    }
}

fn completions_bar_view(bar: &CompletionsBar, theme: &Theme) -> View<Msg> {
    let filtered = bar.filtered_indices();
    let mut rows: Vec<View<Msg>> = Vec::with_capacity(COMPLETIONS_MAX_ITEMS_VISIBLE);
    let filter_label = if bar.filter.is_empty() {
        String::new()
    } else {
        format!(" filtro «{}»", bar.filter)
    };
    let header = if bar.items.is_empty() {
        format!(
            "completions @ {}:{}{} · esperando LSP…",
            bar.anchor.0 + 1, bar.anchor.1, filter_label,
        )
    } else if filtered.is_empty() {
        format!(
            "completions @ {}:{}{} · sin matches",
            bar.anchor.0 + 1, bar.anchor.1, filter_label,
        )
    } else {
        format!(
            "completions @ {}:{}{} · {} / {} · Tab/Enter aplica · Esc cierra",
            bar.anchor.0 + 1,
            bar.anchor.1,
            filter_label,
            bar.selected + 1,
            filtered.len(),
        )
    };
    rows.push(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(theme.bg_panel_alt)
        .text_aligned(header, 10.0, theme.fg_muted, Alignment::Start),
    );

    let visible_start = bar
        .selected
        .saturating_sub(COMPLETIONS_MAX_ITEMS_VISIBLE.saturating_sub(1));
    let visible_end = (visible_start + COMPLETIONS_MAX_ITEMS_VISIBLE).min(filtered.len());
    for vi in visible_start..visible_end {
        let item_idx = filtered[vi];
        let item = &bar.items[item_idx];
        let selected = vi == bar.selected;
        let bg = if selected { theme.bg_selected } else { theme.bg_panel };
        let kind = item.kind.as_deref().unwrap_or("?");
        let detail = item.detail.as_deref().unwrap_or("");
        let label = format!("[{kind:>5}] {}  {}", item.label, detail);
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(COMPLETIONS_ROW_H) },
                padding: Rect {
                    left: length(10.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(bg)
            .text_aligned(label, 11.0, theme.fg_text, Alignment::Start),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(COMPLETIONS_BAR_H) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(rows)
}

fn find_bar(find: &FindBarState, theme: &Theme) -> View<Msg> {
    let tp = TextInputPalette::from_theme(theme);
    let input = text_input_view(&find.input, "buscar… (Enter / Ctrl+G siguiente · Shift inverso · Esc cierra)", true, &tp, Msg::FindOpen);
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(FIND_BAR_H) },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![input])
}

fn empty_editor_placeholder(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(20.0_f32),
            bottom: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        "Seleccioná un archivo del árbol para empezar a editar. \
         Atajos: arrows con Shift selecciona · Ctrl+arrows salta palabra · \
         Ctrl+Z/Y undo/redo · Ctrl+C/X/V clipboard · Ctrl+S guarda.",
        12.0,
        theme.fg_muted,
        Alignment::Start,
    )
}

// ---------------------------------------------------------------------
// Tree logic
// ---------------------------------------------------------------------

fn scan_root(root: &Path) -> Vec<TreeNode> {
    let mut out: Vec<TreeNode> = Vec::new();
    visit_dir(root, 0, false, &mut out);
    out
}

fn visit_dir(dir: &Path, depth: usize, into_expanded: bool, out: &mut Vec<TreeNode>) {
    let _ = into_expanded;
    let mut entries: Vec<(PathBuf, bool)> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| !n.starts_with('.') && n != "target" && n != "node_modules")
                    .unwrap_or(false)
            })
            .map(|e| {
                let p = e.path();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                (p, is_dir)
            })
            .collect(),
        Err(_) => return,
    };
    // Directorios primero, luego archivos; ambos alfabéticos.
    entries.sort_by(|a, b| match (a.1, b.1) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.0.file_name().cmp(&b.0.file_name()),
    });

    for (path, is_dir) in entries {
        out.push(TreeNode {
            path: path.clone(),
            depth,
            is_dir,
            expanded: false,
        });
    }
}

fn toggle_node(mut model: Model, i: usize) -> Model {
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

fn select_node(mut model: Model, i: usize) -> Model {
    let Some(node) = model.nodes.get(i).cloned() else {
        return model;
    };
    model.selected = Some(i);
    if node.is_dir {
        // Click en directorio = toggle también, así no necesita el chevron.
        return toggle_node(model, i);
    }
    match fs::read_to_string(&node.path) {
        Ok(content) => {
            // Si había un archivo abierto antes, notificamos al LSP que
            // se cerró.
            if let Some(prev) = model.open_file.take() {
                model.lsp.did_close(&prev);
            }
            model.editor = EditorState::new();
            model.editor.set_text(&content);
            // Demo de diagnostics fake (--demo-lsp).
            if model.demo_lsp {
                let ext = node.path.extension().and_then(|s| s.to_str()).unwrap_or("");
                if ext == "rs" || ext == "py" {
                    model.editor.set_diagnostics(demo_diagnostics(&content));
                }
            }
            // LSP real (--lsp): notifica al server.
            let ext = node.path.extension().and_then(|s| s.to_str()).unwrap_or("");
            model.lsp.did_open(&node.path, ext, &content);
            model.open_file = Some(node.path.clone());
            model.dirty = false;
            model.status = format!("abierto · {} bytes", content.len());
        }
        Err(e) => {
            model.status = format!("error abriendo: {e}");
        }
    }
    model
}

/// Tres diagnostics fake repartidos en las primeras líneas — Error,
/// Warning, Info. Solo para validar el render del subrayado.
fn demo_diagnostics(content: &str) -> Vec<Diagnostic> {
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

fn apply_editor_key(mut model: Model, ev: KeyEvent) -> Model {
    let r = model.editor.apply_key_with_clipboard(&ev, &mut model.clipboard);
    if r.changed() {
        model.dirty = true;
        if let Some(path) = model.open_file.clone() {
            let text = model.editor.text();
            model.lsp.did_change(&path, &text);
        }
    }
    if r.touched() {
        model.editor.ensure_caret_visible(EDITOR_VISIBLE_LINES);
    }
    // Si el popup de completions está abierto, actualizamos el filter
    // según el prefijo actual del caret. Si no quedan matches → cerramos.
    if let Some(bar) = model.completions.as_mut() {
        let line = model.editor.cursor.caret.line;
        let col = model.editor.cursor.caret.col;
        let (_, prefix) = model.editor.buffer.current_word_prefix(line, col);
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
    if let Some(path) = model.open_file.as_ref() {
        let diags = model.lsp.diagnostics(path);
        if !diags.is_empty() || !model.editor.diagnostics.is_empty() {
            model.editor.set_diagnostics(diags);
        }
    }
    model
}

fn apply_editor_pointer(mut model: Model, ev: PointerEvent) -> Model {
    let metrics = EditorMetrics::for_font_size(13.0);
    let scroll = model.editor.scroll_offset;
    match ev {
        PointerEvent::Click { x, y } => {
            model.drag_accum = (0.0, 0.0);
            let (line, col) = metrics.screen_to_pos(x, y, scroll);
            model.editor.set_caret_at(line, col);
        }
        PointerEvent::Drag { initial_x, initial_y, dx, dy } => {
            model.drag_accum.0 += dx;
            model.drag_accum.1 += dy;
            let cur_x = initial_x + model.drag_accum.0;
            let cur_y = initial_y + model.drag_accum.1;
            let (line, col) = metrics.screen_to_pos(cur_x, cur_y, scroll);
            model.editor.extend_selection_to(line, col);
        }
    }
    model
}

/// Aplica una lista de TextEdits al EditorState en orden descendente
/// por start offset (las edits tempranas no desplazan posiciones
/// posteriores). Cada TextEdit es un reemplazo [start..end) → new_text.
/// Aplica edits a un archivo del disco (no abierto). Carga, aplica
/// ordenados desc por start, escribe atómico (write + fsync no, simple).
fn apply_text_edits_to_file(path: &Path, edits: &[TextEdit]) -> std::io::Result<usize> {
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

fn apply_text_edits_in_place(editor: &mut EditorState, mut edits: Vec<TextEdit>) {
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

fn find_step(mut model: Model, forward: bool) -> Model {
    let Some(find) = model.find.as_ref() else { return model };
    if find.state.query.is_empty() {
        return model;
    }
    let result = if forward {
        find_next(&model.editor.buffer, &find.state, &model.editor.cursor)
    } else {
        find_prev(&model.editor.buffer, &find.state, &model.editor.cursor)
    };
    let Some((start, end)) = result else {
        model.status = format!("sin matches para «{}»", find.state.query);
        return model;
    };
    // Selecciona la match (anchor=start, caret=end) y la deja visible.
    model.editor.cursor.anchor = Some(Pos::new(start.line, start.col));
    model.editor.cursor.caret = Pos::new(end.line, end.col);
    model.editor.cursor.desired_col = end.col;
    model.editor.ensure_caret_visible(EDITOR_VISIBLE_LINES);
    let total = all_matches(&model.editor.buffer, &find.state).len();
    model.status = format!("match · {total} totales");
    model
}

fn save_open_file(model: Model, handle: &Handle<Msg>) -> Model {
    let Some(path) = model.open_file.clone() else {
        return model;
    };
    let content = model.editor.text();
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

fn row_label(n: &TreeNode) -> String {
    let name = n.path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
    // Sin prefijo Unicode/emoji — el chevron del tree widget ya distingue
    // dirs (v/>) de archivos (espacio). Las fuentes default no tienen
    // glyphs para 📁/📄 y dibujan cuadrados de fallback.
    if n.is_dir {
        format!("{name}/")
    } else {
        name.to_owned()
    }
}

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn language_for_path(path: &Path) -> Language {
    let ext = path.extension().and_then(OsStr::to_str).unwrap_or("");
    Language::from_cell_language(ext)
}

// ---------------------------------------------------------------------
// Clipboard backend (arboard)
// ---------------------------------------------------------------------

struct ArboardClipboard {
    inner: Option<arboard::Clipboard>,
}

impl ArboardClipboard {
    fn new() -> Self {
        Self {
            inner: arboard::Clipboard::new().ok(),
        }
    }
}

impl Clipboard for ArboardClipboard {
    fn get(&mut self) -> Option<String> {
        self.inner.as_mut()?.get_text().ok()
    }
    fn set(&mut self, s: &str) {
        if let Some(c) = self.inner.as_mut() {
            let _ = c.set_text(s.to_owned());
        }
    }
}

/// `Color::transparent()` para fills "vacíos" sin importar tema — quedaba
/// huérfano de un branch viejo, lo dejamos por si surge un placeholder
/// que lo necesite.
#[allow(dead_code)]
fn transparent() -> Color {
    Color::TRANSPARENT
}

fn main() {
    llimphi_ui::run::<EditorApp>();
}
