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
use llimphi_widget_text_editor_lsp::{LspClient, NoopLspClient, RustAnalyzerClient};
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
    /// Tick periódico — pull de diagnostics del LSP.
    PollLsp,
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
                    // Sólo reemplaza si hay cambio — evita repaint si
                    // el LSP no envió nada nuevo.
                    if diags != m.editor.diagnostics {
                        m.editor.set_diagnostics(diags);
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
        // Atajos globales
        if event.modifiers.ctrl {
            if matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("s")) {
                return Some(Msg::Save);
            }
            if matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("f"))
                && model.open_file.is_some()
            {
                return Some(Msg::FindOpen);
            }
            if matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("g"))
                && model.find.is_some()
            {
                return Some(if event.modifiers.shift { Msg::FindPrev } else { Msg::FindNext });
            }
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
