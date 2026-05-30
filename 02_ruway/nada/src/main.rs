//! `nada` — editor de archivos rudimentario sobre Llimphi.
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
#![allow(unused_imports)]

mod prelude;
mod view;
mod fsutil;
mod actions;
mod session;
mod clipboard;
mod update;
mod keys;

use crate::prelude::*;
use crate::actions::*;
use crate::fsutil::*;
use crate::session::*;
use crate::clipboard::*;

pub(crate) const TREE_WIDTH: f32 = 240.0;
pub(crate) const TREE_ROW_H: f32 = 22.0;
pub(crate) const TREE_INDENT: f32 = 16.0;
pub(crate) const HEADER_H: f32 = 34.0;
/// Altura del status bar inferior (estilo VS Code).
pub(crate) const STATUS_H: f32 = 24.0;
/// Grosor de las lineas accent que separan header/body/status.
pub(crate) const SEP_H: f32 = 1.0;
/// Altura del tab strip (sin contar la línea de acento).
pub(crate) const TAB_STRIP_H: f32 = 26.0;
/// Cuántas líneas mostramos en el viewport del editor. Aproximación
/// estática: (alto ventana ~760 − header 28) / line_height(~18) ≈ 40.
pub(crate) const EDITOR_VISIBLE_LINES: usize = 40;
/// Altura del panel terminal cuando está abierto. ~14 filas de 14px +
/// header 18px ≈ 214px — redondeado a 220.
pub(crate) const TERM_PANEL_H: f32 = 220.0;
/// Altura del panel diff cuando está abierto. ~30 filas de 15px +
/// header 18px ≈ 468px — redondeado a 480.
pub(crate) const DIFF_PANEL_H: f32 = 480.0;

#[derive(Clone)]
pub(crate) enum Msg {
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
    MiniMap(MiniMapMsg),
    Bookmarks(BookmarksMsg),
    CycleTheme,
    /// El bus `wawa-config` publicó una versión nueva. Aplicamos el
    /// theme y locale del payload; los flags que no nos competen
    /// (apps, módulos, acento, reloj) los ignoramos.
    WawaConfigChanged(Box<wawa_config::WawaConfig>),
    SaveSession,
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
    /// Refresco del mapa git (cada ~3 s desde un thread).
    GitStatusChanged(GitStatusMap),
    /// El LSP devolvió text edits (de formatting o rename) para el
    /// archivo abierto — aplicar todos en orden descendente.
    TextEditsApply(Vec<TextEdit>),
    /// Ctrl+Shift+S — abre prompt con el path actual prepopulado.
    SaveAsOpen,
    SaveAsKey(KeyEvent),
    SaveAsSubmit,
    SaveAsClose,
}

#[derive(Debug, Clone)]
pub(crate) struct TreeNode {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
    expanded: bool,
}

/// Un archivo abierto en su tab. El editor + el flag `dirty` viven aquí;
/// switchear tabs es cuestión de mover el índice `Model.active`.
pub(crate) struct Tab {
    path: PathBuf,
    editor: EditorState,
    dirty: bool,
    /// mtime del archivo la última vez que lo leímos o escribimos. Si en
    /// el siguiente `PollLsp` el mtime de disco difiere, alguien lo tocó
    /// por fuera — el host avisa o recarga según `dirty`.
    last_mtime: Option<std::time::SystemTime>,
    /// `true` si ya advertimos al user del cambio externo desde el último
    /// reload — para no spamear el status bar cada poll.
    external_warned: bool,
}

pub(crate) struct Model {
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
    minimap: Option<MiniMapState>,
    bookmarks: BookmarksState,
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
    /// Etiqueta corta del LSP activo para mostrar en la status bar.
    /// Se setea una vez en init y no muta despues.
    lsp_label: String,
    theme: Theme,
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
    /// Subscripción al bus de configuración del SO (`wawa-config`).
    /// Mantiene vivo el watcher mientras el editor corre; al droparlo
    /// dejan de llegar `WawaConfigChanged`. `None` si la plataforma
    /// no expone ProjectDirs (caso muy raro).
    _wawa_watcher: Option<wawa_config::ConfigWatcher>,
    /// Si `true`, `Msg::Save` dispara primero `request_formatting` y
    /// guarda recién al volver los `TextEditsApply`. Lo enciende
    /// `--fmt-on-save` desde CLI; off por default para no romper save
    /// si el LSP devuelve edits que rompen sintaxis.
    format_on_save: bool,
    /// Idx del tab al que hay que guardar tras aplicar el próximo
    /// `TextEditsApply`. `None` si el último format fue manual.
    pending_save_after_format: Option<usize>,
    /// Prompt de Save-As (Ctrl+Shift+S); `None` cerrado.
    save_as: Option<SaveAsBar>,
    /// Marca git por path absoluto. Repoblado cada ~3 s por un hilo que
    /// ejecuta `git status --porcelain` desde `root`. Vacío si no es
    /// un repo git o git no está instalado.
    git_status: GitStatusMap,
    /// Cola LRU de archivos abiertos recientemente (cap 20). El picker
    /// los muestra al tope cuando se abre — mejor que tener que escribir
    /// el nombre para encontrar algo que acabás de cerrar.
    recent_files: std::collections::VecDeque<PathBuf>,
}

pub(crate) const RECENT_FILES_CAP: usize = 20;

pub(crate) type GitStatusMap = std::collections::HashMap<PathBuf, char>;

pub(crate) struct SaveAsBar {
    input: TextInputState,
}

pub(crate) struct RenameBar {
    input: TextInputState,
    /// Pos donde se pidió el rename.
    anchor: (usize, usize),
    /// `true` mientras esperamos la respuesta del LSP tras submit.
    waiting: bool,
    /// Cuándo se llamó a `request_rename` — `None` si todavía no hubo
    /// submit. Sirve para detectar timeouts del LSP.
    submitted_at: Option<std::time::Instant>,
    /// `true` después de avisar al user del timeout, para no spamear.
    timeout_warned: bool,
}

pub(crate) struct ReferencesBar {
    items: Vec<DefinitionLocation>,
    selected: usize,
    /// Pos donde se pidió la búsqueda.
    anchor: (usize, usize),
    /// Cuándo se disparó el request — para detectar timeouts del LSP.
    requested_at: std::time::Instant,
    /// `true` después de avisarle al user que el LSP no respondió, para
    /// no spamear status.
    timeout_warned: bool,
}

pub(crate) struct SignatureHelpBar {
    info: Option<SignatureHelpInfo>,
    anchor: (usize, usize),
}

pub(crate) struct HoverPopup {
    info: Option<HoverInfo>,
    anchor: (usize, usize),
}

pub(crate) struct CompletionsBar {
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

pub(crate) struct FindBarState {
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
        "nada"
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
        // Tick lento para persistir la sesion (5s). Save es best-effort —
        // si el disco falla no rompe el editor; se reintenta al proximo tick.
        handle.spawn_periodic(std::time::Duration::from_secs(5), || Msg::SaveSession);

        // Tick git status: cada 3 s ejecutamos `git status --porcelain`
        // desde root y publicamos el mapa. Si no es repo git o git no
        // está, el comando falla silenciosamente y el mapa queda vacío.
        {
            let args: Vec<String> = env::args().skip(1).collect();
            let root_for_git = args
                .iter()
                .find(|a| !a.starts_with("--"))
                .map(PathBuf::from)
                .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            let root_for_git = fs::canonicalize(&root_for_git).unwrap_or(root_for_git);
            handle.spawn_periodic(std::time::Duration::from_secs(3), move || {
                Msg::GitStatusChanged(query_git_status(&root_for_git))
            });
        }

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
        let lsp_label = if lsp_on { format!("● lsp:{lsp_cmd}") } else { "○ lsp:off".into() };
        let status = format!(
            "{} entradas · {} archivos indexados",
            nodes.len(),
            all_files.len(),
        );
        let model = Model {
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
            minimap: None,
            bookmarks: BookmarksState::new(),
            diff: None,
            tabs: Vec::new(),
            active: None,
            clipboard: ArboardClipboard::new(),
            status,
            drag_accum: (0.0, 0.0),
            find: None,
            demo_lsp,
            lsp,
            lsp_label,
            theme: Theme::dark(),
            completions: None,
            hover: None,
            sig_help: None,
            references: None,
            rename: None,
            _wawa_watcher: None,
            format_on_save: args.iter().any(|a| a == "--fmt-on-save"),
            pending_save_after_format: None,
            save_as: None,
            git_status: GitStatusMap::new(),
            recent_files: std::collections::VecDeque::with_capacity(RECENT_FILES_CAP),
        };
        // Restaurar sesion previa si la hay: tabs, bookmarks, theme.
        // Best-effort: si load_session falla o paths ya no existen, arranca limpio.
        let mut model = match load_session() {
            Some(sess) => restore_session(model, sess),
            None => model,
        };
        // Bus de configuración del SO. Si hay un panel abierto y ya
        // configuró un theme/idioma global, lo respetamos por encima
        // de la sesión local. La sesión sigue siendo útil cuando el
        // bus no está disponible o no fue inicializado todavía.
        let wawa_cfg = wawa_config::WawaConfig::load();
        model.theme = theme_from_wawa(&wawa_cfg, &model.theme);
        let _ = rimay_localize::set_locale(&wawa_cfg.lang);
        // Subscripción: cualquier cambio futuro reentra al update.
        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("nada · wawa-config watcher: {e}"))
        .ok();
        model._wawa_watcher = watcher;
        model
    }
    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        crate::update::update(model, msg, handle)
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
        crate::keys::on_key(model, event)
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme.clone();
        let header = header_bar(model, &theme);
        let body = body_view(model, &theme);
        let status = status_bar(model, &theme);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![
            header,
            separator_line(&theme),
            body,
            separator_line(&theme),
            status,
        ])
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<EditorApp>();
}
