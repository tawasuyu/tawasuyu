//! `mirada-filechooser` — el diálogo gráfico de **Abrir** / **Guardar
//! como** del escritorio mirada, como ventana Llimphi independiente.
//!
//! No habla D-Bus: lo **lanza `mirada-portal`** como subproceso cuando una
//! app ajena pide `org.freedesktop.impl.portal.FileChooser`. El portal le
//! pasa la petición por argumentos de línea de comando y le indica un
//! archivo `--out` donde dejar el resultado (JSON con los URIs elegidos);
//! al cerrarse, el portal lo lee y responde por el bus. La separación es
//! obligada: el portal corre en `tokio current_thread` y `llimphi_ui::run`
//! se adueña del hilo principal con su propio event loop (winit/wgpu) —
//! dos dueños del main no conviven en un proceso.
//!
//! Aparte de navegar carpetas, el panel izquierdo lista las **mónadas**
//! (clusters semánticos de archivos de `chasqui`) si su daemon está vivo;
//! abrir una mónada muestra sus archivos miembros como destino directo.
//! Si el daemon no responde, la sección simplemente queda vacía.
//!
//! Probarlo suelto, sin D-Bus:
//! ```text
//! cargo run -p mirada-filechooser -- --mode open --title "Abrir" \
//!     --current-folder "$HOME" --out /tmp/fc.json
//! cat /tmp/fc.json
//! ```

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use chasqui_card::query::{client as monad_client, transport, FileView, MonadView};
use chasqui_card::MonadId;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Rect, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};

use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

// ============================================================================
// Geometría
// ============================================================================

const HEADER_H: f32 = 40.0;
const TOOLBAR_H: f32 = 34.0;
const FOOTER_H: f32 = 56.0;
const SIDEBAR_W: f32 = 210.0;
const ROW_H: f32 = 24.0;
/// Timeout corto al daemon de chasqui: si no contesta, seguimos sin mónadas.
const MONAD_TIMEOUT: Duration = Duration::from_millis(700);

// ============================================================================
// Configuración del invocador (CLI) — set una sola vez en `main`
// ============================================================================

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Abrir: elegir archivos existentes. `multiple` = selección múltiple.
    Open { multiple: bool },
    /// Guardar como: tipear un nombre nuevo en una carpeta.
    Save,
}

struct Config {
    mode: Mode,
    title: String,
    accept_label: String,
    folder: PathBuf,
    current_name: String,
    out: PathBuf,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

fn cfg() -> &'static Config {
    CONFIG.get().expect("CONFIG sin inicializar")
}

/// Parser mínimo de `--flag valor`. Suficiente para lo que pasa el portal;
/// no pretende cubrir GNU getopt.
fn parse_args() -> Config {
    let mut mode_save = false;
    let mut multiple = false;
    let mut title = String::new();
    let mut accept_label = String::new();
    let mut folder: Option<PathBuf> = None;
    let mut current_name = String::new();
    let mut out: Option<PathBuf> = None;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--mode" => mode_save = it.next().as_deref() == Some("save"),
            "--multiple" => multiple = true,
            "--title" => title = it.next().unwrap_or_default(),
            "--accept-label" => accept_label = it.next().unwrap_or_default(),
            "--current-folder" => folder = it.next().map(PathBuf::from),
            "--current-name" => current_name = it.next().unwrap_or_default(),
            "--out" => out = it.next().map(PathBuf::from),
            _ => {}
        }
    }

    let mode = if mode_save {
        Mode::Save
    } else {
        Mode::Open { multiple }
    };
    let folder = folder
        .filter(|p| p.is_dir())
        .or_else(|| dirs_home())
        .unwrap_or_else(|| PathBuf::from("/"));
    if title.is_empty() {
        title = match mode {
            Mode::Save => "Guardar como".to_string(),
            Mode::Open { .. } => "Abrir".to_string(),
        };
    }
    if accept_label.is_empty() {
        accept_label = match mode {
            Mode::Save => "Guardar".to_string(),
            Mode::Open { .. } => "Abrir".to_string(),
        };
    }

    Config {
        mode,
        title,
        accept_label,
        folder,
        current_name,
        out: out.unwrap_or_else(|| PathBuf::from("/dev/stdout")),
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

// ============================================================================
// URIs y resultado
// ============================================================================

/// `Path` absoluto → URI `file://` con percent-encoding de los bytes no
/// seguros (lo que esperan los consumidores del portal). No codifica `/`.
fn path_to_uri(path: &Path) -> String {
    const UNRESERVED: &[u8] = b"-_.~/";
    let mut out = String::from("file://");
    for &b in path.to_string_lossy().as_bytes() {
        if b.is_ascii_alphanumeric() || UNRESERVED.contains(&b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(char::from_digit((b >> 4) as u32, 16).unwrap().to_ascii_uppercase());
            out.push(char::from_digit((b & 0xf) as u32, 16).unwrap().to_ascii_uppercase());
        }
    }
    out
}

/// Escribe el resultado al archivo `--out` y cierra la ventana. `response`:
/// 0 = ok, 1 = cancelado (convención del portal).
fn finish(model: &Model, response: u32, uris: Vec<String>, handle: &Handle<Msg>) {
    let json = serde_json::json!({
        "response": response,
        "uris": uris,
        "current_name": model.filename.text(),
    });
    let _ = std::fs::write(&cfg().out, serde_json::to_vec(&json).unwrap_or_default());
    handle.quit();
}

// ============================================================================
// Modelo
// ============================================================================

/// Una fila del listado de carpeta.
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

/// Qué muestra el panel central.
enum Pane {
    /// El contenido de `cwd`.
    Folder,
    /// Los archivos miembros de una mónada.
    Monad(MonadId),
}

struct Model {
    cwd: PathBuf,
    entries: Vec<Entry>,
    /// Índices seleccionados en la vista de carpeta.
    selected: BTreeSet<usize>,
    pane: Pane,
    monads: Vec<MonadView>,
    monad_files: Vec<FileView>,
    monad_sel: BTreeSet<usize>,
    filename: TextInputState,
    /// `true` cuando el campo de nombre tiene el foco (rutea el teclado).
    name_focused: bool,
    list_scroll: f32,
    side_scroll: f32,
    win_h: f32,
    status: String,
}

impl Model {
    fn list_viewport(&self) -> f32 {
        (self.win_h - HEADER_H - TOOLBAR_H - FOOTER_H - 8.0).max(80.0)
    }
    fn list_content(&self) -> f32 {
        let n = match self.pane {
            Pane::Folder => self.entries.len(),
            Pane::Monad(_) => self.monad_files.len() + 1, // +1 por la cabecera
        };
        n as f32 * ROW_H
    }
}

/// Lee `dir`, ordena carpetas primero y luego por nombre (case-insensitive),
/// ocultando los archivos dot. Errores de lectura → lista vacía.
fn read_entries(dir: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
            out.push(Entry {
                name,
                path: ent.path(),
                is_dir,
            });
        }
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

// ============================================================================
// Mensajes
// ============================================================================

#[derive(Clone)]
enum Msg {
    EnterDir(PathBuf),
    GoUp,
    SelectEntry(usize),
    ShowMonad(MonadId),
    BackToFolder,
    SelectMonadFile(usize),
    MonadsLoaded(Vec<MonadView>),
    MonadFilesLoaded(MonadId, Vec<FileView>),
    NameFocus,
    Key(KeyEvent),
    ListScroll(f32),
    SideScroll(f32),
    Resize(f32),
    Accept,
    Cancel,
}

// ============================================================================
// App
// ============================================================================

struct FileChooser;

impl App for FileChooser {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "mirada · archivos"
    }

    fn app_id() -> Option<&'static str> {
        Some("mirada.filechooser")
    }

    fn initial_size() -> (u32, u32) {
        (860, 580)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let c = cfg();
        let entries = read_entries(&c.folder);
        let mut filename = TextInputState::new();
        filename.set_text(c.current_name.clone());

        // Carga de mónadas en un hilo aparte: si el daemon no está, devolvemos
        // lista vacía y el panel queda sin esa sección.
        handle.spawn(|| {
            let sock = transport::default_socket_path();
            match monad_client::list_monads(&sock, MONAD_TIMEOUT) {
                Ok(r) => Msg::MonadsLoaded(r.monads),
                Err(_) => Msg::MonadsLoaded(Vec::new()),
            }
        });

        Model {
            cwd: c.folder.clone(),
            entries,
            selected: BTreeSet::new(),
            pane: Pane::Folder,
            monads: Vec::new(),
            monad_files: Vec::new(),
            monad_sel: BTreeSet::new(),
            filename,
            name_focused: matches!(c.mode, Mode::Save),
            list_scroll: 0.0,
            side_scroll: 0.0,
            win_h: 580.0,
            status: String::new(),
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::EnterDir(path) => {
                model.entries = read_entries(&path);
                model.cwd = path;
                model.selected.clear();
                model.pane = Pane::Folder;
                model.list_scroll = 0.0;
                model.status.clear();
            }
            Msg::GoUp => {
                if let Some(parent) = model.cwd.parent().map(|p| p.to_path_buf()) {
                    model.entries = read_entries(&parent);
                    model.cwd = parent;
                    model.selected.clear();
                    model.pane = Pane::Folder;
                    model.list_scroll = 0.0;
                }
            }
            Msg::SelectEntry(i) => {
                if let Some(e) = model.entries.get(i) {
                    if e.is_dir {
                        let path = e.path.clone();
                        return Self::update(model, Msg::EnterDir(path), handle);
                    }
                    let multiple = matches!(cfg().mode, Mode::Open { multiple: true });
                    if multiple {
                        if !model.selected.insert(i) {
                            model.selected.remove(&i);
                        }
                    } else {
                        model.selected.clear();
                        model.selected.insert(i);
                        // En "guardar", elegir un archivo precarga su nombre.
                        if matches!(cfg().mode, Mode::Save) {
                            model.filename.set_text(e.name.clone());
                        }
                    }
                    model.status.clear();
                }
            }
            Msg::ShowMonad(id) => {
                model.pane = Pane::Monad(id);
                model.monad_files.clear();
                model.monad_sel.clear();
                model.list_scroll = 0.0;
                model.status = "Resolviendo mónada…".to_string();
                handle.spawn(move || {
                    let sock = transport::default_socket_path();
                    match monad_client::resolve_monad(&sock, id, MONAD_TIMEOUT) {
                        Ok(r) => Msg::MonadFilesLoaded(id, r.members),
                        Err(_) => Msg::MonadFilesLoaded(id, Vec::new()),
                    }
                });
            }
            Msg::BackToFolder => {
                model.pane = Pane::Folder;
                model.list_scroll = 0.0;
                model.status.clear();
            }
            Msg::SelectMonadFile(i) => {
                if model.monad_files.get(i).is_some() {
                    let multiple = matches!(cfg().mode, Mode::Open { multiple: true });
                    if multiple {
                        if !model.monad_sel.insert(i) {
                            model.monad_sel.remove(&i);
                        }
                    } else {
                        model.monad_sel.clear();
                        model.monad_sel.insert(i);
                    }
                    model.status.clear();
                }
            }
            Msg::MonadsLoaded(list) => model.monads = list,
            Msg::MonadFilesLoaded(id, files) => {
                if matches!(model.pane, Pane::Monad(cur) if cur == id) {
                    model.monad_files = files;
                    model.status.clear();
                }
            }
            Msg::NameFocus => model.name_focused = true,
            Msg::Key(e) => {
                if model.name_focused {
                    model.filename.apply_key(&e);
                }
            }
            Msg::ListScroll(delta) => {
                model.list_scroll = clamp_offset(
                    model.list_scroll + delta,
                    model.list_content(),
                    model.list_viewport(),
                );
            }
            Msg::SideScroll(delta) => {
                model.side_scroll = (model.side_scroll + delta).max(0.0);
            }
            Msg::Resize(h) => {
                model.win_h = h;
                model.list_scroll = clamp_offset(
                    model.list_scroll,
                    model.list_content(),
                    model.list_viewport(),
                );
            }
            Msg::Accept => return accept(model, handle),
            Msg::Cancel => {
                finish(&model, 1, Vec::new(), handle);
            }
        }
        model
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            // El texto sí necesita ver el release/repeat: lo maneja apply_key.
            if model.name_focused {
                return Some(Msg::Key(event.clone()));
            }
            return None;
        }
        match &event.key {
            Key::Named(NamedKey::Escape) => Some(Msg::Cancel),
            Key::Named(NamedKey::Enter) => Some(Msg::Accept),
            Key::Named(NamedKey::Backspace) if !model.name_focused => Some(Msg::GoUp),
            _ if model.name_focused => Some(Msg::Key(event.clone())),
            _ => None,
        }
    }

    fn on_wheel(
        _model: &Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Msg> {
        if delta.y == 0.0 {
            None
        } else {
            Some(Msg::ListScroll(delta.y * ROW_H * 2.0))
        }
    }

    fn on_resize(_model: &Model, _w: u32, h: u32) -> Option<Msg> {
        // El layout flex se reacomoda solo; guardamos la altura sólo para
        // dimensionar el viewport del scroll. `h` es físico; a 1.0 de escala
        // coincide con el lógico — suficiente para el cálculo del thumb.
        Some(Msg::Resize(h as f32))
    }

    fn ime_allowed() -> bool {
        true
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = load_theme();
        let root = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![
            header(model, &theme),
            toolbar(model, &theme),
            body(model, &theme),
            footer(model, &theme),
        ]);
        root
    }
}

/// Resuelve la acción de Aceptar según el modo, o deja un status si falta
/// algo (nombre vacío, nada seleccionado).
fn accept(model: Model, handle: &Handle<Msg>) -> Model {
    match cfg().mode {
        Mode::Save => {
            let name = model.filename.text();
            let name = name.trim();
            if name.is_empty() {
                let mut m = model;
                m.status = "Escribí un nombre de archivo".to_string();
                return m;
            }
            let candidate = Path::new(name);
            let path = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                model.cwd.join(name)
            };
            finish(&model, 0, vec![path_to_uri(&path)], handle);
            model
        }
        Mode::Open { .. } => {
            let uris = selected_uris(&model);
            if uris.is_empty() {
                let mut m = model;
                m.status = "Seleccioná al menos un archivo".to_string();
                return m;
            }
            finish(&model, 0, uris, handle);
            model
        }
    }
}

fn selected_uris(model: &Model) -> Vec<String> {
    match model.pane {
        Pane::Folder => model
            .selected
            .iter()
            .filter_map(|&i| model.entries.get(i))
            .filter(|e| !e.is_dir)
            .map(|e| path_to_uri(&e.path))
            .collect(),
        Pane::Monad(_) => model
            .monad_sel
            .iter()
            .filter_map(|&i| model.monad_files.get(i))
            .map(|f| path_to_uri(Path::new(&f.path)))
            .collect(),
    }
}

// ============================================================================
// Vistas
// ============================================================================

fn header(_model: &Model, theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_H),
        },
        padding: pad(14.0, 0.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![View::new(grow())
        .text_aligned(cfg().title.clone(), 14.0, theme.fg_text, Alignment::Start)])
}

fn toolbar(model: &Model, theme: &Theme) -> View<Msg> {
    let up = button_styled(
        "↑",
        Style {
            size: Size {
                width: length(30.0_f32),
                height: length(24.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            flex_shrink: 0.0,
            ..Default::default()
        },
        Alignment::Center,
        &ButtonPalette::from_theme(theme),
        Msg::GoUp,
    );

    let crumb = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        padding: pad(10.0, 0.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        model.cwd.display().to_string(),
        11.5,
        theme.fg_muted,
        Alignment::Start,
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(TOOLBAR_H),
        },
        padding: pad(10.0, 5.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![up, crumb])
}

fn body(model: &Model, theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![sidebar(model, theme), main_list(model, theme)])
}

fn sidebar(model: &Model, theme: &Theme) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = Vec::new();
    rows.push(section_header("LUGARES", theme));
    if let Some(home) = dirs_home() {
        rows.push(place_row("🏠 Inicio", &home, &model.cwd, theme));
    }
    rows.push(place_row("⌂ Raíz", Path::new("/"), &model.cwd, theme));
    rows.push(place_row("◇ Carpeta inicial", &cfg().folder, &model.cwd, theme));

    if !model.monads.is_empty() {
        rows.push(spacer(8.0));
        rows.push(section_header("MÓNADAS", theme));
        for m in &model.monads {
            let active = matches!(model.pane, Pane::Monad(id) if id == m.id);
            let label = format!("◈ {}  ({})", m.label, m.cardinality);
            rows.push(
                row_button(label, active, theme, Msg::ShowMonad(m.id)).aria_label(m.label.clone()),
            );
        }
    }

    let content = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto_h(),
        },
        padding: pad(8.0, 8.0),
        ..Default::default()
    })
    .children(rows);

    let scrolled = scroll_y(
        model.side_scroll,
        0.0, // contenido natural; el scroll sólo aparece si desborda el clip
        model.list_viewport(),
        content,
        Msg::SideScroll,
        &ScrollPalette::from_theme(theme),
    );

    View::new(Style {
        size: Size {
            width: length(SIDEBAR_W),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![scrolled])
}

fn main_list(model: &Model, theme: &Theme) -> View<Msg> {
    let content = match model.pane {
        Pane::Folder => {
            let rows: Vec<TreeRow<Msg>> = model
                .entries
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let label = if e.is_dir {
                        format!("{}/", e.name)
                    } else {
                        e.name.clone()
                    };
                    TreeRow {
                        label,
                        depth: 0,
                        has_children: e.is_dir,
                        expanded: false,
                        selected: model.selected.contains(&i),
                        on_toggle: Msg::SelectEntry(i),
                        on_select: Msg::SelectEntry(i),
                        icon: None,
                        on_context: None,
                        editor: None,
                        trailing: None,
                    }
                })
                .collect();
            tree_view(TreeSpec {
                rows,
                row_height: ROW_H,
                indent_px: 14.0,
                palette: TreePalette::from_theme(theme),
                guides: false,
            })
        }
        Pane::Monad(_) => {
            let mut rows: Vec<View<Msg>> = Vec::new();
            rows.push(
                row_button("‹ Volver a carpetas", false, theme, Msg::BackToFolder),
            );
            for (i, f) in model.monad_files.iter().enumerate() {
                rows.push(row_button(
                    f.path.clone(),
                    model.monad_sel.contains(&i),
                    theme,
                    Msg::SelectMonadFile(i),
                ));
            }
            if model.monad_files.is_empty() {
                rows.push(
                    View::new(Style {
                        size: Size {
                            width: percent(1.0_f32),
                            height: length(ROW_H),
                        },
                        padding: pad(10.0, 0.0),
                        align_items: Some(AlignItems::Center),
                        ..Default::default()
                    })
                    .text_aligned(
                        model.status.clone(),
                        11.5,
                        theme.fg_muted,
                        Alignment::Start,
                    ),
                );
            }
            View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: percent(1.0_f32),
                    height: auto_h(),
                },
                ..Default::default()
            })
            .children(rows)
        }
    };

    let scrolled = scroll_y(
        model.list_scroll,
        model.list_content(),
        model.list_viewport(),
        content,
        Msg::ListScroll,
        &ScrollPalette::from_theme(theme),
    );

    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![scrolled])
}

fn footer(model: &Model, theme: &Theme) -> View<Msg> {
    let mut left: Vec<View<Msg>> = Vec::new();

    if matches!(cfg().mode, Mode::Save) {
        left.push(
            View::new(Style {
                size: Size {
                    width: length(64.0_f32),
                    height: percent(1.0_f32),
                },
                align_items: Some(AlignItems::Center),
                flex_shrink: 0.0,
                ..Default::default()
            })
            .text_aligned("Nombre:", 11.5, theme.fg_muted, Alignment::Start),
        );
        left.push(
            View::new(Style {
                flex_grow: 1.0,
                size: Size {
                    width: percent(0.0_f32),
                    height: length(28.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .children(vec![text_input_view(
                &model.filename,
                "nombre del archivo",
                model.name_focused,
                &TextInputPalette::from_theme(theme),
                Msg::NameFocus,
            )]),
        );
    } else {
        // En "abrir", el hueco izquierdo muestra el status o la selección.
        let info = if !model.status.is_empty() {
            model.status.clone()
        } else {
            let n = match model.pane {
                Pane::Folder => model.selected.len(),
                Pane::Monad(_) => model.monad_sel.len(),
            };
            match n {
                0 => String::new(),
                1 => "1 archivo seleccionado".to_string(),
                k => format!("{k} archivos seleccionados"),
            }
        };
        left.push(
            View::new(grow())
                .text_aligned(info, 11.5, theme.fg_muted, Alignment::Start),
        );
    }

    let cancel = button_styled(
        "Cancelar",
        btn_style(108.0),
        Alignment::Center,
        &ButtonPalette::from_theme(theme),
        Msg::Cancel,
    );

    let mut accept_pal = ButtonPalette::from_theme(theme);
    accept_pal.bg = theme.accent;
    accept_pal.bg_hover = theme.accent;
    accept_pal.fg = theme.bg_app;
    let accept = button_styled(
        cfg().accept_label.clone(),
        btn_style(120.0),
        Alignment::Center,
        &accept_pal,
        Msg::Accept,
    );

    let mut children = left;
    children.push(spacer_w(8.0));
    children.push(cancel);
    children.push(spacer_w(8.0));
    children.push(accept);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(FOOTER_H),
        },
        padding: pad(14.0, 12.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(children)
}

// ============================================================================
// Pequeños constructores de vista
// ============================================================================

fn section_header(label: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(label.to_string(), 9.5, theme.fg_muted, Alignment::Start)
}

/// Fila de "lugar" del sidebar: navega a `path`, resaltada si es el cwd.
fn place_row(label: &str, path: &Path, cwd: &Path, theme: &Theme) -> View<Msg> {
    row_button(
        label.to_string(),
        path == cwd,
        theme,
        Msg::EnterDir(path.to_path_buf()),
    )
}

/// Fila clickeable genérica (sidebar / archivos de mónada): texto a la
/// izquierda, fondo resaltado si `active`, hover.
fn row_button(label: impl Into<String>, active: bool, theme: &Theme, msg: Msg) -> View<Msg> {
    let bg = if active { theme.bg_selected } else { theme.bg_panel };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(ROW_H),
        },
        padding: pad(8.0, 0.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(theme.bg_row_hover)
    .radius(4.0)
    .text_aligned(label.into(), 11.5, theme.fg_text, Alignment::Start)
    .on_click(msg)
}

fn spacer(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
}

fn spacer_w(w: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(w),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
}

// ============================================================================
// Helpers de estilo
// ============================================================================

fn pad(x: f32, y: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(x),
        right: length(x),
        top: length(y),
        bottom: length(y),
    }
}

fn grow() -> Style {
    Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    }
}

fn btn_style(w: f32) -> Style {
    Style {
        size: Size {
            width: length(w),
            height: length(30.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    }
}

fn auto_h() -> llimphi_ui::llimphi_layout::taffy::Dimension {
    use llimphi_ui::llimphi_layout::taffy::prelude::auto;
    auto()
}

// ============================================================================
// Tema
// ============================================================================

/// Resuelve el tema activo igual que `mirada-portal`: lee el nombre que
/// persiste `nahual` en `$XDG_CONFIG_HOME/nahual/theme` (fallback `$HOME/
/// .config`). Si no existe o no resuelve, cae al default de Llimphi.
fn load_theme() -> Theme {
    let name = theme_config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match name {
        Some(n) => Theme::by_name(&n).unwrap_or_default(),
        None => Theme::default(),
    }
}

fn theme_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| dirs_home().map(|h| h.join(".config")))?;
    Some(base.join("nahual").join("theme"))
}

// ============================================================================
// Entrada
// ============================================================================

fn main() {
    let _ = CONFIG.set(parse_args());
    llimphi_ui::run::<FileChooser>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_encodes_unsafe_bytes_not_slashes() {
        assert_eq!(path_to_uri(Path::new("/home/a/x.txt")), "file:///home/a/x.txt");
        // Espacios y acentos se codifican; las barras no.
        assert_eq!(
            path_to_uri(Path::new("/home/a b/c.txt")),
            "file:///home/a%20b/c.txt"
        );
        assert!(path_to_uri(Path::new("/tmp/ñ")).starts_with("file:///tmp/"));
        assert!(!path_to_uri(Path::new("/tmp/ñ")).contains('ñ'));
    }

    #[test]
    fn entries_dirs_first_and_skip_hidden() {
        let dir = std::env::temp_dir().join(format!("mfc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("zsub")).unwrap();
        std::fs::create_dir_all(dir.join("asub")).unwrap();
        std::fs::write(dir.join("b.txt"), b"x").unwrap();
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        std::fs::write(dir.join(".oculto"), b"x").unwrap();

        let entries = read_entries(&dir);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        // Carpetas primero (ordenadas), luego archivos (ordenados); sin dotfiles.
        assert_eq!(names, vec!["asub", "zsub", "a.txt", "b.txt"]);
        assert!(entries[0].is_dir && !entries[2].is_dir);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
