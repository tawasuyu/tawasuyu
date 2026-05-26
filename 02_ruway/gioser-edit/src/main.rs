//! `gioser-edit` — editor de archivos rudimentario sobre Llimphi.
//!
//! - **Tree** a la izquierda (240 px) con el contenido del directorio
//!   `cwd` (o el primer argumento). Click expande/colapsa directorios;
//!   click en archivo lo abre como tab nuevo (o activa el existente).
//! - **Tab strip** arriba del editor (vía `llimphi-widget-tabs`): un tab
//!   por archivo abierto, prefijo `●` cuando está modificado. Click
//!   cambia de tab.
//! - **Editor** a la derecha: text-editor multilínea con syntax highlight
//!   derivado de la extensión (`.rs` → Rust, `.py` → Python, `.wat` → Wat,
//!   resto → Plain). Caret, selección, bracket matching, gutter con
//!   line numbers, undo/redo, copy/cut/paste.
//! - **Atajos globales**: Ctrl+S guarda · Ctrl+W cierra tab · Ctrl+Tab /
//!   Ctrl+Shift+Tab ciclan tabs · Ctrl+P fuzzy file picker (walk
//!   recursivo del workspace, hasta 50k archivos) · Ctrl+Shift+F
//!   find-in-files (JetBrains style: panel con input + lista de matches
//!   file:line + snippet, Enter abre el match) · Ctrl+F find en archivo
//!   actual · Ctrl+G next match · Ctrl+Space completions · Ctrl+K hover
//!   · Ctrl+Alt+L format (estilo JetBrains) · Ctrl+Shift+Space
//!   signatureHelp · F12 goto-def · Shift+F12 references · F2 rename.
//! - **Atajos del editor**: arrows + Shift/Ctrl, Home/End, Ctrl+Home/End,
//!   PageUp/Down, Tab/Shift+Tab, Backspace/Delete, Ctrl+Z/Y/Shift+Z
//!   (undo/redo), Ctrl+C/X/V (clipboard del sistema vía arboard).
//!
//! Limitaciones: el tree se construye al arrancar (no watcher), no
//! confirma overwrites externos, no hay save-as todavía.

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
use llimphi_module_command_palette::{
    self as palette, Command as PaletteCommand, PaletteAction, PaletteMsg, PalettePalette,
    PaletteState,
};
use llimphi_module_diff_viewer::{
    self as diff, DiffAction, DiffMsg, DiffPalette, DiffState,
};
use llimphi_module_fif::{self as fif, FifAction, FifMsg, FifPalette, FifState};
use llimphi_module_file_picker::{
    self as picker, PickerAction, PickerMsg, PickerPalette, PickerState,
};
use llimphi_module_shuma_term::{
    self as term, ShumaTermAction, ShumaTermMsg, ShumaTermPalette, ShumaTermState,
};
use llimphi_module_symbol_outline::{
    self as outline, OutlineAction, OutlineMsg, OutlinePalette, OutlineState, SymbolItem,
};
use llimphi_widget_tabs::{tabs_view, TabsPalette, TabsSpec};
use llimphi_widget_text_editor::{
    all_matches, find_next, find_prev, text_editor_view_full, Clipboard, Diagnostic,
    EditorMetrics, EditorPalette, EditorState, FindState, Language, PointerEvent, Pos,
};
use llimphi_widget_text_editor_lsp::{
    CompletionItem, DefinitionLocation, DocumentSymbolEntry, HoverInfo, LspClient, NoopLspClient,
    RustAnalyzerClient, SignatureHelpInfo, TextEdit,
};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

const TREE_WIDTH: f32 = 240.0;
const TREE_ROW_H: f32 = 22.0;
const TREE_INDENT: f32 = 16.0;
const HEADER_H: f32 = 28.0;
/// Altura del tab strip (sin contar la línea de acento).
const TAB_STRIP_H: f32 = 26.0;
/// Cuántas líneas mostramos en el viewport del editor. Aproximación
/// estática: (alto ventana ~760 − header 28) / line_height(~18) ≈ 40.
const EDITOR_VISIBLE_LINES: usize = 40;
/// Altura del panel terminal cuando está abierto. ~14 filas de 14px +
/// header 18px ≈ 214px — redondeado a 220.
const TERM_PANEL_H: f32 = 220.0;
/// Altura del panel diff cuando está abierto. ~30 filas de 15px +
/// header 18px ≈ 468px — redondeado a 480.
const DIFF_PANEL_H: f32 = 480.0;

#[derive(Clone)]
enum Msg {
    ToggleNode(usize),
    SelectNode(usize),
    EditKey(KeyEvent),
    EditorPointer(PointerEvent),
    Save,
    SaveResult(Result<(), String>),
    Scroll(i32),
    /// Cambia el tab activo. El índice se asume válido; en caso contrario
    /// se ignora.
    ActivateTab(usize),
    /// Cierra el tab dado. Si era el activo, salta al anterior (o `None`
    /// si era el último). Notifica `did_close` al LSP.
    CloseTab(usize),
    /// Atajo Ctrl+Tab.
    NextTab,
    /// Atajo Ctrl+Shift+Tab.
    PrevTab,
    /// Mensajes del módulo file-picker.
    Picker(PickerMsg),
    /// Mensajes del módulo find-in-files. El host los wrappea para
    /// rutearlos a `llimphi_module_fif::apply`.
    Fif(FifMsg),
    /// Mensajes del módulo terminal integrado (Ctrl+`).
    Term(ShumaTermMsg),
    /// Mensajes del módulo command palette (Ctrl+Shift+P).
    Palette(PaletteMsg),
    /// Mensajes del módulo symbol outline (Ctrl+Shift+O).
    Outline(OutlineMsg),
    /// El LSP devolvió document symbols — repoblar el outline.
    OutlineRefresh(Vec<SymbolItem>),
    /// Mensajes del módulo diff viewer (Ctrl+Shift+D).
    Diff(DiffMsg),
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

/// Un archivo abierto en su tab. El editor + el flag `dirty` viven aquí;
/// switchear tabs es cuestión de mover el índice `Model.active`.
struct Tab {
    path: PathBuf,
    editor: EditorState,
    dirty: bool,
}

struct Model {
    root: PathBuf,
    nodes: Vec<TreeNode>,
    selected: Option<usize>,
    /// Walk recursivo de todos los archivos bajo `root` (skip dotfiles,
    /// `target/`, `node_modules/`). Cacheado al arrancar; lo consume el
    /// fuzzy file picker (Ctrl+P).
    all_files: Vec<PathBuf>,
    /// Estado del picker; `None` cerrado.
    picker: Option<PickerState>,
    /// Estado del find-in-files; `None` cerrado.
    fif: Option<FifState>,
    /// Terminal integrado; `None` cerrado. Cuando está abierto, las
    /// teclas pasan al PTY (con excepciones del módulo).
    term: Option<ShumaTermState>,
    /// Command palette; `None` cerrado.
    palette: Option<PaletteState>,
    /// Catálogo estático de comandos disponibles. Se construye en
    /// `init` y se reusa en cada apertura del palette — el palette no
    /// lo copia, sólo guarda índices.
    palette_commands: Vec<PaletteCommand>,
    /// Symbol outline; `None` cerrado.
    outline: Option<OutlineState>,
    /// Últimos símbolos devueltos por el LSP para el tab activo. Se
    /// repuebla en cada `OutlineRefresh`; vacío hasta que llega la
    /// primera respuesta.
    outline_symbols: Vec<SymbolItem>,
    /// Diff viewer; `None` cerrado. Snapshot del diff: si el buffer
    /// cambia con el panel abierto, las filas no se recomputan — el
    /// usuario cierra y reabre para refrescar (semántica congelada,
    /// como VS Code "Compare with Saved").
    diff: Option<DiffState>,
    tabs: Vec<Tab>,
    /// Índice del tab activo dentro de `tabs`. `None` si no hay ninguno
    /// abierto todavía.
    active: Option<usize>,
    clipboard: ArboardClipboard,
    status: String,
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

impl Model {
    fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|i| self.tabs.get(i))
    }
    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        match self.active {
            Some(i) => self.tabs.get_mut(i),
            None => None,
        }
    }
    fn active_path(&self) -> Option<PathBuf> {
        self.active_tab().map(|t| t.path.clone())
    }
    fn tab_idx_for(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|t| t.path == path)
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
        // Tick más rápido para drenar el PTY del terminal (si está abierto).
        // Sin esto, el output de comandos del shell aparece a saltos de 400ms.
        handle.spawn_periodic(std::time::Duration::from_millis(50), || {
            Msg::Term(ShumaTermMsg::Tick)
        });

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
        let all_files = walk_files(&root);
        let lsp: Box<dyn LspClient> = if lsp_on {
            Box::new(RustAnalyzerClient::with_command(root.clone(), &lsp_cmd))
        } else {
            Box::new(NoopLspClient)
        };
        let lsp_label = if lsp_on { format!("lsp:{lsp_cmd}") } else { "lsp:off".into() };
        let status = format!(
            "{} · {} entradas · {} archivos · {lsp_label}",
            root.display(),
            nodes.len(),
            all_files.len(),
        );
        Model {
            root,
            nodes,
            selected: None,
            all_files,
            picker: None,
            fif: None,
            term: None,
            palette: None,
            palette_commands: build_command_catalog(),
            outline: None,
            outline_symbols: Vec::new(),
            diff: None,
            tabs: Vec::new(),
            active: None,
            clipboard: ArboardClipboard::new(),
            status,
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
                if let Some(tab) = m.active_tab_mut() {
                    tab.editor.scroll_by(delta);
                }
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
            Msg::Diff(dm) => apply_diff(model, dm),
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
                m.status = "goto-def · esperando LSP…".into();
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
                let Some(path) = m.active_path() else { return m };
                let Some(r) = m.rename.as_mut() else { return m };
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
                m.status = "formatting · esperando LSP…".into();
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
                m.status = "formatting · aplicado".into();
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
                    m.status = format!("goto-def · {}:{}", loc.path.display(), loc.line + 1);
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
                        m.tabs.push(Tab { path: loc.path.clone(), editor, dirty: false });
                        m.active = Some(m.tabs.len() - 1);
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
                        let path_disp = m
                            .active_tab()
                            .map(|t| t.path.display().to_string())
                            .unwrap_or_default();
                        if let Some(tab) = m.active_tab_mut() {
                            tab.dirty = false;
                        }
                        format!("guardado · {path_disp}")
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
        if model.active_tab().is_none() {
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

        // Atajos globales
        if event.modifiers.ctrl {
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
    let (open, dirty_mark) = match model.active_tab() {
        Some(tab) => (
            relative_to(&model.root, &tab.path),
            if tab.dirty { " · ● modificado" } else { "" },
        ),
        None => ("(sin archivo abierto)".to_string(), ""),
    };
    let tabs_count = if model.tabs.len() > 1 {
        format!(" · {} tabs", model.tabs.len())
    } else {
        String::new()
    };
    let text = format!(
        "gioser-edit · {} · {}{}{}  ·  {}",
        model.root.display(),
        open,
        dirty_mark,
        tabs_count,
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
        // Sin esto, el `flex_shrink: 1.0` default de taffy comprime el
        // header cuando body pide percent(1.0) + grow:1 sobre el column
        // raíz — overflow de 28px se reparte mitad y mitad.
        flex_shrink: 0.0,
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
    .children(vec![tree_panel(model, theme), right_panel(model, theme)])
}

/// Columna derecha: editor arriba; si el terminal está abierto, va
/// como panel inferior fijo de 220px (estilo VS Code).
fn right_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let editor = editor_panel(model, theme);
    let mut children = vec![editor];
    if let Some(state) = model.term.as_ref() {
        children.push(term::view(
            state,
            &ShumaTermPalette::from_theme(theme),
            TERM_PANEL_H,
            Msg::Term,
        ));
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(children)
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
    let inner = active_editor_content(model, theme);
    if model.tabs.is_empty() {
        // Sin tabs todavía: solo placeholder, sin tab strip.
        return View::new(Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![inner]);
    }
    let labels: Vec<String> = model
        .tabs
        .iter()
        .map(|t| {
            let name = t.path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
            if t.dirty {
                format!("● {name}")
            } else {
                name.to_string()
            }
        })
        .collect();
    let active = model.active.unwrap_or(0);
    tabs_view(TabsSpec {
        labels,
        active,
        on_select: Msg::ActivateTab,
        content: inner,
        tab_height: TAB_STRIP_H,
        palette: TabsPalette::from_theme(theme),
        tab_width: None,
    })
}

/// Contenido del tab activo: bars (find/completions/hover/etc.) + editor.
/// Si no hay tab activo, devuelve el placeholder.
fn active_editor_content(model: &Model, theme: &Theme) -> View<Msg> {
    let mut children: Vec<View<Msg>> = Vec::new();
    if let Some(p) = model.palette.as_ref() {
        let pal = PalettePalette::from_theme(theme);
        children.push(palette::view(p, &model.palette_commands, &pal, Msg::Palette));
    }
    if let Some(o) = model.outline.as_ref() {
        let pal = OutlinePalette::from_theme(theme);
        children.push(outline::view(o, &model.outline_symbols, &pal, Msg::Outline));
    }
    if let Some(d) = model.diff.as_ref() {
        let pal = DiffPalette::from_theme(theme);
        children.push(diff::view(d, &pal, DIFF_PANEL_H, Msg::Diff));
    }
    if let Some(p) = model.picker.as_ref() {
        let palette = PickerPalette::from_theme(theme);
        children.push(picker::view(p, &model.all_files, &model.root, &palette, Msg::Picker));
    }
    if let Some(f) = model.fif.as_ref() {
        let palette = FifPalette::from_theme(theme);
        children.push(fif::view(f, &model.all_files, &model.root, &palette, Msg::Fif));
    }
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
    let editor_view = match model.active_tab() {
        None => empty_editor_placeholder(theme),
        Some(tab) => {
            let language = language_for_path(&tab.path);
            let palette = EditorPalette::from_theme(theme);
            let metrics = EditorMetrics::for_font_size(13.0);
            let matches: Vec<(usize, usize)> = model
                .find
                .as_ref()
                .filter(|f| !f.state.query.is_empty())
                .map(|f| all_matches(&tab.editor.buffer, &f.state))
                .unwrap_or_default();
            text_editor_view_full(
                &tab.editor,
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

/// Walk recursivo: todos los archivos bajo `root`, excluyendo dotfiles,
/// `target/` y `node_modules/`. Devuelve paths absolutos. Cap a 50k para
/// que un mal directorio no funda RAM.
const PICKER_FILE_CAP: usize = 50_000;
fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= PICKER_FILE_CAP {
            break;
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else { continue };
            if name_str.starts_with('.') || name_str == "target" || name_str == "node_modules" {
                continue;
            }
            let path = entry.path();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                stack.push(path);
            } else {
                out.push(path);
                if out.len() >= PICKER_FILE_CAP {
                    break;
                }
            }
        }
    }
    out.sort();
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
    open_path(model, node.path)
}

/// Abre un archivo: si ya hay un tab con ese path lo activa; si no, lee
/// del disco, crea EditorState nuevo, notifica `did_open` al LSP y empuja
/// un tab nuevo. Mensaje de status según el resultado.
fn open_path(mut model: Model, path: PathBuf) -> Model {
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
            model.tabs.push(Tab { path: path.clone(), editor, dirty: false });
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
fn apply_picker(model: Model, pm: PickerMsg) -> Model {
    let mut m = model;
    if matches!(pm, PickerMsg::Open) && m.picker.is_none() {
        m.picker = Some(PickerState::new(&m.all_files, &m.root));
        m.status = format!(
            "picker · {} archivos · ↓↑ Enter abre · Esc cierra",
            m.all_files.len(),
        );
        return m;
    }
    let action = match m.picker.as_mut() {
        Some(state) => picker::apply(state, pm, &m.all_files, &m.root),
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
/// resultante a la mutación apropiada del Model. Único lugar de gioser-edit
/// que conoce los detalles del módulo.
fn apply_fif(model: Model, fmsg: FifMsg) -> Model {
    let mut m = model;
    // Lazy-init: el host emite `FifMsg::Open` cuando el user dispara el
    // shortcut; recién ahí construimos el state.
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
        FifAction::Close => {
            m.fif = None;
        }
        FifAction::Searched { matches, elapsed, query } => {
            m.status = format!(
                "find-in-files · «{query}» · {matches} matches · {:.0} ms",
                elapsed.as_secs_f64() * 1000.0,
            );
        }
        FifAction::OpenAt { path, line, col } => {
            m.fif = None;
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
const DIFF_VISIBLE_ROWS: usize = ((DIFF_PANEL_H - 18.0) / 15.0) as usize;

/// Routea un DiffMsg al módulo diff. Lazy-init: en `Open`, lee el
/// archivo de disco y compara contra el buffer actual. Snapshot
/// congelado — cambios subsecuentes del buffer no recomputan.
fn apply_diff(model: Model, dm: DiffMsg) -> Model {
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
fn symbols_lsp_to_module(lsp: Vec<DocumentSymbolEntry>) -> Vec<SymbolItem> {
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
fn apply_outline(model: Model, om: OutlineMsg) -> Model {
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
fn build_command_catalog() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand::new("editor.save", "Save File", "Editor").with_shortcut("Ctrl+S"),
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
    ]
}

/// Traduce un id de comando del catálogo al `Msg` correspondiente. Si
/// el id es desconocido, devuelve `None` y el host lo reporta como
/// status. Mantener en sync con [`build_command_catalog`].
fn palette_id_to_msg(id: &str) -> Option<Msg> {
    Some(match id {
        "editor.save" => Msg::Save,
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
        _ => return None,
    })
}

/// Routea un PaletteMsg al módulo command-palette. Lazy-init en `Open`.
/// En `Invoke(id)`: cierra el palette y dispatcha el Msg correspondiente
/// — el comando se ejecuta en el siguiente turno del loop.
fn apply_palette(model: Model, pm: PaletteMsg, handle: &Handle<Msg>) -> Model {
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
fn apply_term(model: Model, tm: ShumaTermMsg) -> Model {
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
fn activate_tab(mut model: Model, idx: usize) -> Model {
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
fn close_tab(mut model: Model, idx: usize) -> Model {
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

fn apply_editor_pointer(mut model: Model, ev: PointerEvent) -> Model {
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

fn save_open_file(model: Model, handle: &Handle<Msg>) -> Model {
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
