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
//! La navegación **no se reimplementa**: reusa
//! [`nahual_source_core::Navigator`] sobre `PosixSource` — el mismo núcleo
//! que monta `nahual-shell-llimphi` (listado, ordenado, **filtro vivo**,
//! breadcrumb, scroll por rueda). Para POSIX el `NodeId` de cada nodo es su
//! ruta absoluta, así que el **marcado múltiple** es un `BTreeSet<NodeId>` y
//! los URIs salen directo del id. Encima va lo propio de un diálogo: panel
//! de **lugares** + **mónadas** de `chasqui`, campo de nombre para guardar,
//! cuadro de filtro y los botones Aceptar/Cancelar.
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
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_scroll::{scroll_y, ScrollPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

use nahual_source_core::{Navigator, Node, NodeId, NodeKind, PosixSource, SortKey};

// ============================================================================
// Geometría
// ============================================================================

const HEADER_H: f32 = 40.0;
const TOOLBAR_H: f32 = 38.0;
const FOOTER_H: f32 = 56.0;
const SIDEBAR_W: f32 = 210.0;
const ROW_H: f32 = 24.0;
/// Alto de fila del listado (coincide con el del shell de nahual).
const LIST_ROW_H: f32 = 22.0;
/// Timeout corto al daemon de chasqui: si no contesta, seguimos sin mónadas.
const MONAD_TIMEOUT: Duration = Duration::from_millis(700);

/// Cuántas filas del listado caben en el alto de ventana dado.
fn rows_for_height(win_h: f32) -> usize {
    let body = (win_h - HEADER_H - TOOLBAR_H - FOOTER_H - 8.0).max(LIST_ROW_H);
    (body / LIST_ROW_H).floor().max(1.0) as usize
}

// ============================================================================
// Configuración del invocador (CLI) — set una sola vez en `main`
// ============================================================================

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Abrir: elegir uno o varios archivos existentes.
    Open,
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
    let mut title = String::new();
    let mut accept_label = String::new();
    let mut folder: Option<PathBuf> = None;
    let mut current_name = String::new();
    let mut out: Option<PathBuf> = None;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--mode" => mode_save = it.next().as_deref() == Some("save"),
            // `--multiple` ya no cambia nada: Abrir siempre permite marcar
            // varios (Espacio / click). Se acepta por compatibilidad.
            "--multiple" => {}
            "--title" => title = it.next().unwrap_or_default(),
            "--accept-label" => accept_label = it.next().unwrap_or_default(),
            "--current-folder" => folder = it.next().map(PathBuf::from),
            "--current-name" => current_name = it.next().unwrap_or_default(),
            "--out" => out = it.next().map(PathBuf::from),
            _ => {}
        }
    }

    let mode = if mode_save { Mode::Save } else { Mode::Open };
    let folder = folder
        .filter(|p| p.is_dir())
        .or_else(dirs_home)
        .unwrap_or_else(|| PathBuf::from("/"));
    if title.is_empty() {
        title = match mode {
            Mode::Save => "Guardar como".to_string(),
            Mode::Open => "Abrir".to_string(),
        };
    }
    if accept_label.is_empty() {
        accept_label = match mode {
            Mode::Save => "Guardar".to_string(),
            Mode::Open => "Abrir".to_string(),
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

/// Construye el navegador POSIX anclado en `/` (para poder subir hasta la
/// raíz del filesystem) y parado en `cwd`, sembrando la pila de ancestros
/// para que el breadcrumb tenga la ruta completa. Réplica del `posix_nav`
/// de `nahual-shell-llimphi` (que es `pub(crate)` y no se puede importar).
fn posix_nav(cwd: &Path) -> Navigator {
    use std::path::Component;
    let mut stack = vec![Node::new("/", "/", true).with_kind(NodeKind::Dir)];
    let mut acc = PathBuf::from("/");
    for comp in cwd.components() {
        if let Component::Normal(c) = comp {
            acc.push(c);
            stack.push(
                Node::new(
                    acc.to_string_lossy().into_owned(),
                    c.to_string_lossy().into_owned(),
                    true,
                )
                .with_kind(NodeKind::Dir),
            );
        }
    }
    Navigator::open_at(Box::new(PosixSource::new("/")), stack)
        .or_else(|_| Navigator::open(Box::new(PosixSource::new("/"))))
        .expect("la raíz / siempre se puede listar")
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

/// Qué tiene el foco de teclado (para rutear las pulsaciones).
#[derive(Clone, Copy, PartialEq)]
enum Focus {
    /// La lista de archivos (navegación con flechas/Enter/Espacio).
    None,
    /// El cuadro de filtro vivo.
    Filter,
    /// El campo de nombre (modo Guardar).
    Filename,
}

/// Qué muestra el panel central.
enum Pane {
    /// El contenido de una carpeta — delegado a `Navigator`.
    Folder,
    /// Los archivos miembros de una mónada de chasqui.
    Monad(MonadId),
}

struct Model {
    /// Núcleo de navegación reutilizado de nahual.
    nav: Navigator,
    /// Archivos marcados (selección múltiple). Para POSIX el id es la ruta.
    marked: BTreeSet<NodeId>,
    filter_input: TextInputState,
    filename: TextInputState,
    focus: Focus,
    pane: Pane,
    monads: Vec<MonadView>,
    monad_files: Vec<FileView>,
    monad_sel: Option<usize>,
    side_scroll: f32,
    win_h: f32,
    status: String,
}

// ============================================================================
// Mensajes
// ============================================================================

#[derive(Clone)]
enum Msg {
    Go(PathBuf),
    Select(usize),
    OpenSelected,
    Nav(i32),
    Parent,
    Wheel(f32),
    ToggleMark,
    CycleSort,
    FilterFocus,
    FilenameFocus,
    ClearFilter,
    Key(KeyEvent),
    ShowMonad(MonadId),
    BackToFolder,
    SelectMonadFile(usize),
    MonadsLoaded(Vec<MonadView>),
    MonadFilesLoaded(MonadId, Vec<FileView>),
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
        (880, 600)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let c = cfg();
        let mut nav = posix_nav(&c.folder);
        nav.visible_rows = rows_for_height(600.0);
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
            nav,
            marked: BTreeSet::new(),
            filter_input: TextInputState::new(),
            filename,
            focus: if matches!(c.mode, Mode::Save) {
                Focus::Filename
            } else {
                Focus::None
            },
            pane: Pane::Folder,
            monads: Vec::new(),
            monad_files: Vec::new(),
            monad_sel: None,
            side_scroll: 0.0,
            win_h: 600.0,
            status: String::new(),
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Go(path) => {
                let mut nav = posix_nav(&path);
                nav.visible_rows = rows_for_height(model.win_h);
                model.nav = nav;
                model.pane = Pane::Folder;
                model.status.clear();
            }
            Msg::Select(idx) => {
                if matches!(model.pane, Pane::Folder) && model.nav.select(idx) {
                    let info = model
                        .nav
                        .selected_node()
                        .map(|n| (n.is_container, n.id.clone(), n.name.clone()));
                    if let Some((is_dir, id, name)) = info {
                        if is_dir {
                            // Click en carpeta = entrar (el navegador relee).
                            let _ = model.nav.open_selected();
                        } else {
                            match cfg().mode {
                                // Abrir = marcar/desmarcar (multi-selección).
                                Mode::Open => {
                                    if !model.marked.insert(id.clone()) {
                                        model.marked.remove(&id);
                                    }
                                }
                                // Guardar = precargar el nombre.
                                Mode::Save => model.filename.set_text(name),
                            }
                        }
                    }
                    model.status.clear();
                }
            }
            Msg::OpenSelected => {
                if matches!(model.pane, Pane::Folder)
                    && model.nav.selected_node().map(|n| n.is_container) == Some(true)
                {
                    let _ = model.nav.open_selected();
                }
            }
            Msg::Nav(d) => {
                if matches!(model.pane, Pane::Folder) {
                    if d < 0 {
                        model.nav.up();
                    } else {
                        model.nav.down();
                    }
                }
            }
            Msg::Parent => {
                if matches!(model.pane, Pane::Folder) {
                    let _ = model.nav.parent();
                }
            }
            Msg::Wheel(delta) => {
                if matches!(model.pane, Pane::Folder) {
                    model.nav.apply_wheel(delta);
                }
            }
            Msg::ToggleMark => {
                if matches!(cfg().mode, Mode::Open) && matches!(model.pane, Pane::Folder) {
                    if let Some(id) = model
                        .nav
                        .selected_node()
                        .filter(|n| !n.is_container)
                        .map(|n| n.id.clone())
                    {
                        if !model.marked.insert(id.clone()) {
                            model.marked.remove(&id);
                        }
                    }
                }
            }
            Msg::CycleSort => {
                let (k, _) = model.nav.sort();
                let next = match k {
                    SortKey::Name => SortKey::Mtime,
                    SortKey::Mtime => SortKey::Size,
                    SortKey::Size => SortKey::Kind,
                    SortKey::Kind => SortKey::Name,
                };
                model.nav.set_sort_to(next, true);
            }
            Msg::FilterFocus => model.focus = Focus::Filter,
            Msg::FilenameFocus => model.focus = Focus::Filename,
            Msg::ClearFilter => {
                model.filter_input.clear();
                model.nav.set_filter(String::new());
                model.nav.selected = 0;
            }
            Msg::Key(e) => match model.focus {
                Focus::Filename => {
                    model.filename.apply_key(&e);
                }
                Focus::Filter => {
                    model.filter_input.apply_key(&e);
                    model.nav.set_filter(model.filter_input.text());
                    model.nav.selected = 0;
                    model.nav.visible_offset = 0;
                }
                Focus::None => {}
            },
            Msg::ShowMonad(id) => {
                model.pane = Pane::Monad(id);
                model.monad_files.clear();
                model.monad_sel = None;
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
                model.status.clear();
            }
            Msg::SelectMonadFile(i) => {
                if i < model.monad_files.len() {
                    model.monad_sel = Some(i);
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
            Msg::SideScroll(delta) => {
                model.side_scroll = (model.side_scroll + delta).max(0.0);
            }
            Msg::Resize(h) => {
                model.win_h = h;
                model.nav.visible_rows = rows_for_height(h);
            }
            Msg::Accept => return accept(model, handle),
            Msg::Cancel => finish(&model, 1, Vec::new(), handle),
        }
        model
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        let text_focus = model.focus != Focus::None;
        if event.state != KeyState::Pressed {
            return if text_focus {
                Some(Msg::Key(event.clone()))
            } else {
                None
            };
        }
        match &event.key {
            Key::Named(NamedKey::Escape) => {
                if model.focus == Focus::Filter && !model.filter_input.is_empty() {
                    Some(Msg::ClearFilter)
                } else {
                    Some(Msg::Cancel)
                }
            }
            Key::Named(NamedKey::Enter) => match model.focus {
                Focus::Filename => Some(Msg::Accept),
                Focus::Filter => None,
                Focus::None => {
                    if matches!(model.pane, Pane::Folder)
                        && model.nav.selected_node().map(|n| n.is_container) == Some(true)
                    {
                        Some(Msg::OpenSelected)
                    } else {
                        Some(Msg::Accept)
                    }
                }
            },
            Key::Named(NamedKey::Backspace) if model.focus == Focus::None => Some(Msg::Parent),
            Key::Named(NamedKey::ArrowUp) if model.focus == Focus::None => Some(Msg::Nav(-1)),
            Key::Named(NamedKey::ArrowDown) if model.focus == Focus::None => Some(Msg::Nav(1)),
            Key::Named(NamedKey::Space) if model.focus == Focus::None => Some(Msg::ToggleMark),
            _ if text_focus => Some(Msg::Key(event.clone())),
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
            // El acumulador fraccional vive en Navigator::apply_wheel.
            Some(Msg::Wheel(delta.y * 3.0))
        }
    }

    fn on_resize(_model: &Model, _w: u32, h: u32) -> Option<Msg> {
        Some(Msg::Resize(h as f32))
    }

    fn ime_allowed() -> bool {
        true
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = load_theme();
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![
            header(&theme),
            toolbar(model, &theme),
            body(model, &theme),
            footer(model, &theme),
        ])
    }
}

/// Resuelve la acción de Aceptar según el modo, o deja un status si falta
/// algo (nombre vacío, nada seleccionado).
fn accept(mut model: Model, handle: &Handle<Msg>) -> Model {
    match cfg().mode {
        Mode::Save => {
            let raw = model.filename.text();
            let name = raw.trim();
            if name.is_empty() {
                model.status = "Escribí un nombre de archivo".to_string();
                return model;
            }
            let candidate = Path::new(name);
            let path = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                PathBuf::from(model.nav.current_id()).join(name)
            };
            finish(&model, 0, vec![path_to_uri(&path)], handle);
            model
        }
        Mode::Open => {
            let uris = selected_uris(&model);
            if uris.is_empty() {
                model.status = "Seleccioná un archivo (Espacio o click)".to_string();
                return model;
            }
            finish(&model, 0, uris, handle);
            model
        }
    }
}

/// Los URIs a devolver en modo Abrir: los marcados, o como fallback el
/// archivo actualmente resaltado.
fn selected_uris(model: &Model) -> Vec<String> {
    match model.pane {
        Pane::Folder => {
            if !model.marked.is_empty() {
                model
                    .marked
                    .iter()
                    .map(|id| path_to_uri(Path::new(id)))
                    .collect()
            } else {
                model
                    .nav
                    .selected_node()
                    .filter(|n| !n.is_container)
                    .map(|n| vec![path_to_uri(Path::new(&n.id))])
                    .unwrap_or_default()
            }
        }
        Pane::Monad(_) => model
            .monad_sel
            .and_then(|i| model.monad_files.get(i))
            .map(|f| vec![path_to_uri(Path::new(&f.path))])
            .unwrap_or_default(),
    }
}

// ============================================================================
// Vistas
// ============================================================================

fn header(theme: &Theme) -> View<Msg> {
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
    .children(vec![View::new(grow()).text_aligned(
        cfg().title.clone(),
        14.0,
        theme.fg_text,
        Alignment::Start,
    )])
}

fn toolbar(model: &Model, theme: &Theme) -> View<Msg> {
    let up = button_styled(
        "↑",
        Style {
            size: Size {
                width: length(30.0_f32),
                height: length(26.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            flex_shrink: 0.0,
            ..Default::default()
        },
        Alignment::Center,
        &ButtonPalette::from_theme(theme),
        Msg::Parent,
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
        model.nav.current_id().to_string(),
        11.5,
        theme.fg_muted,
        Alignment::Start,
    );

    // Botón de ordenamiento — cicla Nombre → Fecha → Tamaño → Tipo.
    let (key, _) = model.nav.sort();
    let klabel = match key {
        SortKey::Name => "Nombre",
        SortKey::Mtime => "Fecha",
        SortKey::Size => "Tamaño",
        SortKey::Kind => "Tipo",
    };
    let sort_btn = button_styled(
        format!("⇅ {klabel}"),
        btn_style(108.0),
        Alignment::Center,
        &ButtonPalette::from_theme(theme),
        Msg::CycleSort,
    );

    // Cuadro de filtro vivo.
    let filter = View::new(Style {
        size: Size {
            width: length(200.0_f32),
            height: length(26.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![text_input_view(
        &model.filter_input,
        "🔍 filtrar…",
        model.focus == Focus::Filter,
        &TextInputPalette::from_theme(theme),
        Msg::FilterFocus,
    )]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(TOOLBAR_H),
        },
        padding: pad(10.0, 6.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![up, crumb, sort_btn, spacer_w(8.0), filter])
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
    .children(vec![sidebar(model, theme), main_pane(model, theme)])
}

fn sidebar(model: &Model, theme: &Theme) -> View<Msg> {
    let cwd = PathBuf::from(model.nav.current_id());
    let mut rows: Vec<View<Msg>> = Vec::new();
    rows.push(section_header("LUGARES", theme));
    if let Some(home) = dirs_home() {
        rows.push(place_row("🏠 Inicio", &home, &cwd, theme));
    }
    rows.push(place_row("⌂ Raíz", Path::new("/"), &cwd, theme));
    rows.push(place_row("◇ Carpeta inicial", &cfg().folder, &cwd, theme));

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
        0.0,
        (model.win_h - HEADER_H - TOOLBAR_H - FOOTER_H).max(80.0),
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

fn main_pane(model: &Model, theme: &Theme) -> View<Msg> {
    let content = match model.pane {
        Pane::Folder => folder_list_view(model, ListPalette::from_theme(theme)),
        Pane::Monad(_) => {
            let mut rows: Vec<View<Msg>> = Vec::new();
            rows.push(row_button("‹ Volver a carpetas", false, theme, Msg::BackToFolder));
            for (i, f) in model.monad_files.iter().enumerate() {
                rows.push(row_button(
                    f.path.clone(),
                    model.monad_sel == Some(i),
                    theme,
                    Msg::SelectMonadFile(i),
                ));
            }
            if model.monad_files.is_empty() && !model.status.is_empty() {
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
                    .text_aligned(model.status.clone(), 11.5, theme.fg_muted, Alignment::Start),
                );
            }
            View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: percent(1.0_f32),
                    height: auto_h(),
                },
                padding: pad(6.0, 6.0),
                ..Default::default()
            })
            .children(rows)
        }
    };

    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![content])
}

/// Pinta los hijos visibles (ya filtrados/ordenados por el Navigator) como
/// `llimphi-widget-list`. Calcado del `navigator_list_view` del shell, con un
/// check `✓` al frente de cada archivo marcado.
fn folder_list_view(model: &Model, palette: ListPalette) -> View<Msg> {
    use std::cmp::min;
    let nav = &model.nav;
    let multi = matches!(cfg().mode, Mode::Open);
    let visibles = nav.visible();
    let start = nav.visible_offset.min(visibles.len());
    let end = min(visibles.len(), start + nav.visible_rows);

    let rows: Vec<ListRow<Msg>> = visibles[start..end]
        .iter()
        .map(|(idx, n)| {
            let mark = if !multi {
                ""
            } else if model.marked.contains(&n.id) {
                "✓ "
            } else {
                "  "
            };
            let icon = if n.is_container { "▸ " } else { "  " };
            let label = if n.is_container {
                format!("{mark}{icon}{}/", n.name)
            } else {
                format!("{mark}{icon}{}", n.name)
            };
            ListRow {
                label,
                selected: *idx == nav.selected,
                on_click: Msg::Select(*idx),
            }
        })
        .collect();

    let truncated_hint = if visibles.len() > end {
        Some(format!("… y {} más (rueda o ↓ para ver más)", visibles.len() - end))
    } else {
        None
    };

    let total = visibles.len();
    let caption = if !nav.filter().is_empty() {
        format!("{total} coinciden con «{}» · Esc limpia", nav.filter())
    } else if multi && !model.marked.is_empty() {
        format!(
            "{total} entradas · {} marcados · Espacio marca · Enter entra",
            model.marked.len()
        )
    } else {
        format!("{total} entradas · ↑↓ navega · Enter entra · ⌫ sube · Espacio marca")
    };

    list_view(ListSpec {
        rows,
        total,
        caption: Some(caption),
        truncated_hint,
        row_height: LIST_ROW_H,
        palette,
    })
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
                model.focus == Focus::Filename,
                &TextInputPalette::from_theme(theme),
                Msg::FilenameFocus,
            )]),
        );
    } else {
        let info = if !model.status.is_empty() {
            model.status.clone()
        } else {
            selection_label(model)
        };
        left.push(View::new(grow()).text_aligned(info, 11.5, theme.fg_muted, Alignment::Start));
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

/// Texto del footer en modo Abrir: el conteo de marcados, el archivo
/// resaltado, o vacío.
fn selection_label(model: &Model) -> String {
    match model.pane {
        Pane::Folder => {
            let m = model.marked.len();
            if m == 1 {
                "1 archivo seleccionado".to_string()
            } else if m > 1 {
                format!("{m} archivos seleccionados")
            } else {
                model
                    .nav
                    .selected_node()
                    .filter(|n| !n.is_container)
                    .map(|n| n.name.clone())
                    .unwrap_or_default()
            }
        }
        Pane::Monad(_) => model
            .monad_sel
            .and_then(|i| model.monad_files.get(i))
            .map(|f| f.path.clone())
            .unwrap_or_default(),
    }
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
    row_button(label.to_string(), path == cwd, theme, Msg::Go(path.to_path_buf()))
}

/// Fila clickeable genérica (sidebar / archivos de mónada).
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
    bitacora::abrir("mirada");
    let _ = CONFIG.set(parse_args());
    llimphi_ui::run::<FileChooser>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_encodes_unsafe_bytes_not_slashes() {
        assert_eq!(path_to_uri(Path::new("/home/a/x.txt")), "file:///home/a/x.txt");
        assert_eq!(
            path_to_uri(Path::new("/home/a b/c.txt")),
            "file:///home/a%20b/c.txt"
        );
        assert!(path_to_uri(Path::new("/tmp/ñ")).starts_with("file:///tmp/"));
        assert!(!path_to_uri(Path::new("/tmp/ñ")).contains('ñ'));
    }

    #[test]
    fn rows_scale_with_height() {
        assert!(rows_for_height(900.0) > rows_for_height(400.0));
        assert!(rows_for_height(100.0) >= 1);
    }

    #[test]
    fn nav_anchors_at_cwd_with_breadcrumb() {
        // Anclado en `/`, el id actual es la raíz y el breadcrumb arranca ahí.
        let nav = posix_nav(Path::new("/"));
        assert_eq!(nav.current_id().as_str(), "/");
        assert_eq!(nav.ancestors().len(), 1);

        // Para un subdirectorio, la pila de ancestros tiene un nivel por
        // componente (/ + tmp = 2) y el id actual es esa ruta.
        let nav = posix_nav(Path::new("/tmp"));
        assert_eq!(nav.current_id().as_str(), "/tmp");
        assert_eq!(nav.ancestors().len(), 2);
    }
}
