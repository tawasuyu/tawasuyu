//! `pluma-notebook-llimphi` — visor read-only de notebooks sobre Llimphi.
//!
//! Uso:
//!   pluma-notebook-llimphi [ruta.pluma-nb]
//!
//! Sin argumento, abre un notebook demo con las celdas posicionadas en
//! canvas para mostrar el modo espacial.
//!
//! Dos modos de presentación:
//!   - **Lineal**: cards apilados verticalmente (notebook tradicional).
//!     Se elige cuando ninguna celda del notebook tiene `position`.
//!   - **Canvas**: cards absolutamente posicionados en (x, y) según
//!     `Cell::position`, con conectores S-codo entre cada celda y sus
//!     dependencias. Se elige automáticamente si al menos una celda
//!     tiene `position` definida.
//!
//! MVP: sólo render. Sin edición de fuentes, sin ejecución contra kernel,
//! sin scroll/pan/zoom. Edición → integrar `pluma-editor-llimphi`.
//! Ejecución → cablear `pluma-notebook-exec::{run_all, run_from}`.

use std::env;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pluma_notebook_kernel_multi::MultiKernel;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Rect, Size, Style},
    AlignItems,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_text_editor::{
    text_editor_view_highlighted, EditorMetrics, EditorPalette, EditorState, Language,
    PointerEvent,
};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_widget_skeleton::{skeleton_view, SkeletonPalette};
use llimphi_widget_empty::{empty_view, EmptyPalette};
use llimphi_widget_toast::{toast_stack_view, Toast};
use llimphi_icons::Icon;
use llimphi_motion::{animate, motion, Tween};
use llimphi_clipboard::SystemClipboard;
use std::sync::Arc;
use pluma_notebook_core::{
    Cell, CellId, CellKind, CellOutput, CellState, Notebook, Position as CanvasPos,
};

#[derive(Clone)]
enum Msg {
    /// Desplaza el viewport del canvas por `(dx, dy)`.
    PanBy(f32, f32),
    /// Centra el viewport en el bounding box de las celdas posicionadas
    /// (atajo "Home" cuando no estás editando).
    ResetViewport,
    /// Cambia el zoom por un factor multiplicativo (ej. 1.1 = +10%).
    ZoomBy(f32),
    /// Resetea zoom a 1.0.
    ZoomReset,
    /// Zoom + viewport para que entre todo el bounding box (fit-all).
    FitAll,
    /// Mueve una celda en el canvas por `(dx, dy)` (delta desde el evento
    /// anterior — no acumulado desde el press).
    MoveCell { id: CellId, dx: f32, dy: f32 },
    /// El usuario pidió ejecutar desde una celda — corre `run_from` en un
    /// thread aparte y dispatcha `RunCompleted` al volver.
    RunFrom(CellId),
    /// El kernel terminó: reemplaza el notebook por la versión con los
    /// estados actualizados.
    RunCompleted(Notebook),
    /// Entra modo edición sobre una celda. Carga el `source` actual en
    /// el TextInput.
    StartEdit(CellId),
    /// Tecla aplicada al input en edición.
    EditKey(KeyEvent),
    /// Click o drag sobre el área de texto del editor.
    EditorPointer(PointerEvent),
    /// Guarda el draft del input como nuevo `source` (vía `set_source`,
    /// que marca stale + propaga) y sale del modo edición.
    CommitEdit,
    /// Descarta el draft y sale del modo edición.
    CancelEdit,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Comando del menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Navegación por teclado en el menú principal (`+1` baja, `-1` sube).
    MenuNav(i32),
    /// Enter en el menú principal: ejecuta la fila activa.
    MenuActivate,
    /// Tick de animación de menús (sólo re-render).
    MenuTick,
    /// Navegación por teclado en el menú de edición.
    EditNav(i32),
    /// Enter en el menú de edición: ejecuta la fila activa.
    EditActivate,
    /// Right-click → menú de edición en `(x,y)` de ventana sobre la celda
    /// en edición (si la hay).
    EditMenuOpen(f32, f32),
    /// Acción del menú de edición sobre la celda en edición.
    EditMenuAction(EditAction),
    /// Cierra cualquier menú abierto.
    CloseMenus,
    /// Tick de animación (~50ms) — fuerza repaint para el shimmer del
    /// skeleton mientras hay una corrida en vuelo. Se auto-rearma sólo
    /// mientras `running_from` siga `Some` (ver `arm_tick`).
    Tick,
    /// Un toast cumplió su vida: se descarta del stack.
    ToastExpire(u64),
    /// La ventana cambió de tamaño — actualiza el viewport usado para
    /// posicionar el stack de toasts.
    Resize(u32, u32),
}

/// Estado de una celda en edición. Editor completo: rope buffer + flechas
/// + selección + undo/redo + indent auto + bracket matching. **Ctrl+Enter**
/// commitea (set_source en el notebook), **Esc** cancela. La card crece
/// a alto extendido mientras dura la edición.
struct EditState {
    id: CellId,
    editor: EditorState,
    /// Acumulado de drag para drag-to-select del mouse — análogo a
    /// nada. Pos actual = initial + accum.
    drag_accum: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Linear,
    Canvas,
}

struct Model {
    notebook: Notebook,
    mode: Mode,
    /// Offset del viewport en modo canvas — el usuario lo cambia
    /// arrastrando el fondo o con la rueda del mouse. Se suma a cada
    /// `Cell::position` al render.
    viewport: (f32, f32),
    /// Factor de zoom del canvas. 1.0 = nativo; > 1 = más grande;
    /// rango sensato 0.25..4.0.
    zoom: f32,
    /// Celda raíz de una corrida en curso (si la hay). Bloquea nuevos
    /// pedidos hasta que el thread devuelva `RunCompleted`.
    running_from: Option<CellId>,
    /// Estado de la celda en edición, si la hay.
    editing: Option<EditState>,
    /// Archivo de origen (None = demo embebido).
    source: Option<PathBuf>,
    /// Mensaje de error si load falló — se muestra en el header.
    load_error: Option<String>,
    /// Menú principal: índice del menú raíz abierto (`None` cerrado).
    menu_open: Option<usize>,
    /// Fila resaltada por teclado en el menú principal (`usize::MAX` = ninguna).
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    menu_anim: Tween<f32>,
    /// Menú de edición contextual: ancla `(x,y)` en ventana (`None` cerrado).
    edit_menu: Option<(f32, f32)>,
    /// Fila resaltada por teclado en el menú de edición (`usize::MAX` = ninguna).
    edit_active: usize,
    /// Animación de aparición del menú de edición (0→1).
    edit_anim: Tween<f32>,
    /// Portapapeles del sistema para cortar/copiar/pegar en las celdas.
    clipboard: SystemClipboard,
    /// Toasts vivos (resultado de una ejecución: OK o falla).
    toasts: Vec<Toast>,
    /// Id incremental para correlacionar toast ↔ Msg de expiración.
    next_toast: u64,
    /// Hay una cadena de `Msg::Tick` en vuelo (evita rearmar dos).
    ticking: bool,
    /// Tamaño actual de la ventana — ancla el stack de toasts (bottom-right).
    win: (f32, f32),
}

struct Viewer;

impl App for Viewer {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma-notebook"
    }

    fn initial_size() -> (u32, u32) {
        (980, 760)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let arg = env::args().nth(1).map(PathBuf::from);
        let (notebook, source, load_error) = match arg {
            None => (demo_notebook(), None, None),
            Some(p) => match pluma_notebook_store::load(&p) {
                Ok(nb) => (nb, Some(p), None),
                Err(e) => (Notebook::new(), Some(p), Some(e.to_string())),
            },
        };
        let mode = if notebook.cells().iter().any(|c| c.position.is_some()) {
            Mode::Canvas
        } else {
            Mode::Linear
        };
        Model {
            notebook,
            mode,
            viewport: (0.0, 0.0),
            zoom: 1.0,
            running_from: None,
            editing: None,
            source,
            load_error,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_menu: None,
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
            clipboard: SystemClipboard::new(),
            toasts: Vec::new(),
            next_toast: 0,
            ticking: false,
            win: {
                let (w, h) = Self::initial_size();
                (w as f32, h as f32)
            },
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::PanBy(dx, dy) => Model {
                viewport: (model.viewport.0 + dx, model.viewport.1 + dy),
                ..model
            },
            Msg::ResetViewport => Model {
                viewport: viewport_to_fit(&model.notebook),
                ..model
            },
            Msg::ZoomBy(factor) => Model {
                zoom: (model.zoom * factor).clamp(0.25, 4.0),
                ..model
            },
            Msg::ZoomReset => Model { zoom: 1.0, ..model },
            Msg::FitAll => {
                let (zoom, viewport) = fit_all(&model.notebook);
                Model { zoom, viewport, ..model }
            }
            Msg::MoveCell { id, dx, dy } => {
                let mut nb = model.notebook;
                if let Some(p) = nb.position(id) {
                    nb.set_position(id, Some(CanvasPos::new(p.x + dx, p.y + dy)));
                }
                Model { notebook: nb, ..model }
            }
            Msg::RunFrom(id) => {
                // Ya hay una corrida en curso → ignoramos el pedido.
                if model.running_from.is_some() {
                    return model;
                }
                let mut nb = model.notebook.clone();
                handle.spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("tokio runtime");
                    let kernel = MultiKernel::new();
                    let _ = rt.block_on(pluma_notebook_exec::run_from(&mut nb, &kernel, id));
                    Msg::RunCompleted(nb)
                });
                let mut model = Model { running_from: Some(id), ..model };
                arm_tick(&mut model, handle);
                model
            }
            Msg::RunCompleted(nb) => {
                let mut model = model;
                // ¿Aparecieron fallas nuevas respecto del estado previo? Eso
                // distingue una corrida exitosa de una que rompió algo.
                let before = model.notebook.cells().iter().filter(|c| c.state == CellState::Failed).count();
                let after = nb.cells().iter().filter(|c| c.state == CellState::Failed).count();
                model.notebook = nb;
                model.running_from = None;
                let id = model.next_toast;
                model.next_toast += 1;
                let toast = if after > before {
                    Toast::error(id, "La ejecución falló", TOAST_TTL)
                } else {
                    Toast::success(id, "Celda ejecutada", TOAST_TTL)
                };
                push_toast(&mut model, handle, toast);
                model
            }
            Msg::StartEdit(id) => {
                let Some(cell) = model.notebook.cell(id) else { return model };
                let mut editor = EditorState::new();
                editor.set_text(&cell.source);
                Model {
                    editing: Some(EditState { id, editor, drag_accum: (0.0, 0.0) }),
                    ..model
                }
            }
            Msg::EditKey(ev) => {
                let mut model = model;
                let Model { editing, clipboard, .. } = &mut model;
                if let Some(edit) = editing.as_mut() {
                    let _ = edit.editor.apply_key_with_clipboard(&ev, clipboard);
                }
                model
            }
            Msg::MenuOpen(idx) => {
                let mut model = Model {
                    menu_open: idx,
                    menu_active: usize::MAX,
                    edit_menu: None,
                    ..model
                };
                if idx.is_some() {
                    model.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
                model
            }
            Msg::MenuCommand(cmd) => handle_menu_command(model, cmd),
            Msg::MenuNav(dir) => {
                let mut model = model;
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    model.menu_active = menubar_nav(&menu, mi, model.menu_active, dir);
                }
                model
            }
            Msg::MenuActivate => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    if let Some(cmd) = menubar_command_at(&menu, mi, model.menu_active) {
                        return handle_menu_command(model, cmd);
                    }
                }
                model
            }
            Msg::MenuTick => model,
            Msg::EditNav(dir) => {
                let mut model = model;
                let flags = notebook_edit_flags(&model);
                model.edit_active = editmenu::edit_menu_step(flags, model.edit_active, dir);
                model
            }
            Msg::EditActivate => {
                let mut model = model;
                let flags = notebook_edit_flags(&model);
                if let Some(action) = editmenu::edit_menu_action_at(flags, model.edit_active) {
                    model.edit_menu = None;
                    let Model { editing, clipboard, .. } = &mut model;
                    if let Some(edit) = editing.as_mut() {
                        let _ = editmenu::apply(&mut edit.editor, action, clipboard);
                    }
                }
                model
            }
            Msg::EditMenuOpen(x, y) => {
                let mut model = Model {
                    edit_menu: Some((x, y)),
                    edit_active: usize::MAX,
                    menu_open: None,
                    ..model
                };
                model.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(handle, motion::FAST, || Msg::MenuTick);
                model
            }
            Msg::EditMenuAction(action) => {
                let mut model = model;
                model.edit_menu = None;
                let Model { editing, clipboard, .. } = &mut model;
                if let Some(edit) = editing.as_mut() {
                    let _ = editmenu::apply(&mut edit.editor, action, clipboard);
                }
                model
            }
            Msg::CloseMenus => Model {
                menu_open: None,
                menu_active: usize::MAX,
                edit_menu: None,
                edit_active: usize::MAX,
                ..model
            },
            Msg::EditorPointer(ev) => {
                let mut model = model;
                if let Some(edit) = model.editing.as_mut() {
                    let metrics = EditorMetrics::for_font_size(12.0);
                    let scroll = edit.editor.scroll_offset;
                    match ev {
                        PointerEvent::Click { x, y } => {
                            edit.drag_accum = (0.0, 0.0);
                            let (line, col) = metrics.screen_to_pos(x, y, scroll);
                            edit.editor.set_caret_at(line, col);
                        }
                        PointerEvent::Drag { initial_x, initial_y, dx, dy } => {
                            edit.drag_accum.0 += dx;
                            edit.drag_accum.1 += dy;
                            let cx = initial_x + edit.drag_accum.0;
                            let cy = initial_y + edit.drag_accum.1;
                            let (line, col) = metrics.screen_to_pos(cx, cy, scroll);
                            edit.editor.extend_selection_to(line, col);
                        }
                    }
                }
                model
            }
            Msg::CommitEdit => {
                let mut model = model;
                if let Some(edit) = model.editing.take() {
                    let _ = model.notebook.set_source(edit.id, edit.editor.text());
                }
                model
            }
            Msg::CancelEdit => Model { editing: None, ..model },
            Msg::Tick => {
                // El thread durmió ~50ms; sólo rearmamos si seguimos corriendo.
                let mut model = model;
                model.ticking = false;
                arm_tick(&mut model, handle);
                model
            }
            Msg::ToastExpire(tid) => {
                let mut model = model;
                model.toasts.retain(|t| t.id != tid);
                model
            }
            Msg::Resize(w, h) => Model { win: (w as f32, h as f32), ..model },
        }
    }

    fn on_key(model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // Menús abiertos: las flechas navegan y tienen prioridad sobre todo.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        if model.edit_menu.is_some() {
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::EditNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::EditNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::EditActivate),
                _ => None,
            };
        }
        // Sin edición activa: atajos del canvas.
        if model.editing.is_none() {
            if model.mode == Mode::Canvas {
                match &event.key {
                    Key::Named(NamedKey::Home) => return Some(Msg::ResetViewport),
                    // F = fit-all (zoom + viewport).
                    Key::Character(s) if s.eq_ignore_ascii_case("f") => return Some(Msg::FitAll),
                    // 0 = reset zoom a 1.0.
                    Key::Character(s) if s == "0" => return Some(Msg::ZoomReset),
                    // + / - zoom.
                    Key::Character(s) if s == "+" || s == "=" => return Some(Msg::ZoomBy(1.1)),
                    Key::Character(s) if s == "-" => return Some(Msg::ZoomBy(1.0 / 1.1)),
                    _ => {}
                }
            }
            return None;
        }
        match &event.key {
            // Ctrl+Enter commitea; Enter solo inserta \n al editor.
            Key::Named(NamedKey::Enter) if event.modifiers.ctrl => Some(Msg::CommitEdit),
            Key::Named(NamedKey::Escape) => Some(Msg::CancelEdit),
            _ => Some(Msg::EditKey(event.clone())),
        }
    }

    fn on_wheel(
        model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        modifiers: Modifiers,
    ) -> Option<Self::Msg> {
        if model.mode != Mode::Canvas {
            return None;
        }
        // Ctrl+wheel = zoom (factor 1.1 por click).
        if modifiers.ctrl {
            let factor = if delta.y > 0.0 { 1.0 / 1.1 } else { 1.1 };
            return Some(Msg::ZoomBy(factor));
        }
        // 32 px por "línea" del wheel; llimphi-ui ya invierte el signo
        // de winit, re-invertimos para que rueda-abajo mueva el
        // contenido hacia arriba (estilo editor).
        const STEP: f32 = 32.0;
        let dx = delta.x * STEP;
        let dy = -delta.y * STEP;
        if dx == 0.0 && dy == 0.0 {
            None
        } else {
            Some(Msg::PanBy(dx, dy))
        }
    }

    fn on_resize(_model: &Self::Model, w: u32, h: u32) -> Option<Self::Msg> {
        Some(Msg::Resize(w, h))
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = Palette::from_theme(&theme);

        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let header = header_bar(model, &palette);
        // Sin celdas (notebook vacío o load fallido) → empty-state con
        // orientación en vez de un lienzo en negro.
        let body = if model.notebook.len() == 0 {
            let pal = EmptyPalette::from_theme(&theme);
            let (titulo, desc): (&str, &str) = if model.load_error.is_some() {
                ("No se pudo abrir", "El archivo no cargó. Revisá la ruta .pluma-nb del header.")
            } else {
                ("Notebook vacío", "No hay celdas para mostrar. Pasá una ruta .pluma-nb para abrir un cuaderno.")
            };
            empty_view(Icon::FileText, titulo, Some(desc), &pal)
        } else {
            match model.mode {
                Mode::Linear => linear_view(&model.notebook, &palette),
                Mode::Canvas => canvas_view(
                    &model.notebook,
                    model.viewport,
                    model.zoom,
                    model.editing.as_ref(),
                    model.running_from,
                    &palette,
                ),
            }
        };

        let root = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(palette.bg)
        .on_right_click_at(|x, y, _, _| Some(Msg::EditMenuOpen(x, y)))
        .children(vec![menubar, header, body]);

        // Overlay de toasts (bottom-right). Click en uno = descartarlo.
        let now = Instant::now();
        let alive: Vec<Toast> = model.toasts.iter().filter(|t| t.is_alive(now)).cloned().collect();
        if alive.is_empty() {
            root
        } else {
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .children(vec![root, toast_stack_view(&alive, model.win, Msg::ToastExpire)])
        }
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        let theme = Theme::dark();
        if let Some((x, y)) = model.edit_menu {
            let flags = notebook_edit_flags(model);
            let (w, h) = Self::initial_size();
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                (w as f32, h as f32),
                &theme,
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras {
                    appear: model.edit_anim.value(),
                    ..Default::default()
                },
            ));
        }
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

/// `MenuBarSpec` compartido por `view` y `view_overlay`.
fn menubar_spec<'a>(menu: &'a app_bus::AppMenu, model: &Model, theme: &'a Theme) -> MenuBarSpec<'a, Msg> {
    let (w, h) = Viewer::initial_size();
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: (w as f32, h as f32),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Menú principal del notebook. Editar refleja en gris el estado real de
/// la celda en edición (si no hay celda en edición, todo gris).
/// `EditFlags` de la celda en edición, para nav/ejecución por teclado del
/// menú de edición. Sin celda en edición, flags vacíos (todo gris).
fn notebook_edit_flags(model: &Model) -> EditFlags {
    match model.editing.as_ref() {
        Some(e) => EditFlags::from_editor(&e.editor, false),
        None => EditFlags::default(),
    }
}

fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};
    let ed = model.editing.as_ref().map(|e| &e.editor);
    let has_sel = ed.map(|e| e.has_selection()).unwrap_or(false);
    let can_undo = ed.map(|e| e.can_undo()).unwrap_or(false);
    let can_redo = ed.map(|e| e.can_redo()).unwrap_or(false);
    let editing = ed.is_some();

    let dis = |it: MenuItem, on: bool| if on { it } else { it.disabled() };

    // Etiquetas de UI localizadas; el 2º arg de MenuItem::new es el id de
    // comando estable — nunca se localiza.
    let t = rimay_localize::t;

    // Menú de idioma: autónimos sin traducir (convención del SO).
    // El item activo lleva ✔. El comando `lang.<code>` lo resuelve
    // `handle_menu_command` → set_locale + persiste en wawa-config.
    let cur = rimay_localize::current_locale();
    let lang_item = |label: &str, code: &str| {
        let mut it = MenuItem::new(label, format!("lang.{code}"));
        if cur == code {
            it = it.icon("\u{2714}");
        }
        it
    };

    AppMenu::new()
        .menu(
            Menu::new(t("edit"))
                .item(dis(MenuItem::new(t("undo"), "edit.undo").shortcut("Ctrl+Z"), can_undo))
                .item(dis(MenuItem::new(t("redo"), "edit.redo").shortcut("Ctrl+Y"), can_redo))
                .item(dis(MenuItem::new(t("cut"), "edit.cut").shortcut("Ctrl+X").separated(), has_sel))
                .item(dis(MenuItem::new(t("copy"), "edit.copy").shortcut("Ctrl+C"), has_sel))
                .item(MenuItem::new(t("paste"), "edit.paste").shortcut("Ctrl+V"))
                .item(dis(MenuItem::new(t("select-all"), "edit.selectall").shortcut("Ctrl+A").separated(), editing)),
        )
        .menu(
            Menu::new(t("view"))
                .item(MenuItem::new(t("pluma-notebook-fit-all"), "view.fitall").shortcut("F"))
                .item(MenuItem::new(t("pluma-notebook-center"), "view.reset").shortcut("Inicio"))
                .item(MenuItem::new(t("pluma-notebook-zoom-reset"), "view.zoomreset").separated()),
        )
        .menu(
            Menu::new(t("language"))
                .item(lang_item("Español", "es-PE"))
                .item(lang_item("English", "en-US"))
                .item(lang_item("Runasimi", "qu-PE")),
        )
}

/// Traduce el comando del menú al `Msg` real y lo aplica. Cierra el menú.
fn handle_menu_command(mut model: Model, command: String) -> Model {
    model.menu_open = None;
    // Cambio de idioma desde el menú "Idioma": aplica el locale en caliente
    // y lo persiste en wawa-config para que sobreviva reinicios.
    if let Some(code) = command.strip_prefix("lang.") {
        let _ = rimay_localize::set_locale(code);
        let mut cfg = wawa_config::WawaConfig::load();
        cfg.lang = code.to_string();
        let _ = cfg.save();
        return model;
    }
    let action = match command.as_str() {
        "edit.undo" => Some(EditAction::Undo),
        "edit.redo" => Some(EditAction::Redo),
        "edit.cut" => Some(EditAction::Cut),
        "edit.copy" => Some(EditAction::Copy),
        "edit.paste" => Some(EditAction::Paste),
        "edit.selectall" => Some(EditAction::SelectAll),
        _ => None,
    };
    if let Some(a) = action {
        let Model { editing, clipboard, .. } = &mut model;
        if let Some(edit) = editing.as_mut() {
            let _ = editmenu::apply(&mut edit.editor, a, clipboard);
        }
        return model;
    }
    match command.as_str() {
        "view.fitall" => {
            let (zoom, viewport) = fit_all(&model.notebook);
            Model { zoom, viewport, ..model }
        }
        "view.reset" => Model { viewport: viewport_to_fit(&model.notebook), ..model },
        "view.zoomreset" => Model { zoom: 1.0, ..model },
        _ => model,
    }
}

/// Paleta semántica del visor — sale del Theme y se pasa por las funciones
/// de render para no leer `Theme::dark()` desde cada una.
struct Palette {
    bg: Color,
    bg_panel: Color,
    bg_card: Color,
    fg_text: Color,
    fg_muted: Color,
    fg_error: Color,
    accent_stale: Color,
    accent_failed: Color,
    accent_fresh: Color,
    edge: Color,
}

impl Palette {
    fn from_theme(t: &Theme) -> Self {
        Self {
            bg: t.bg_app,
            bg_panel: t.bg_panel,
            bg_card: t.bg_panel_alt,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
            accent_stale: t.fg_muted,
            accent_failed: t.fg_destructive,
            accent_fresh: t.fg_text,
            edge: t.accent,
        }
    }
}

/// Cuánto vive un toast antes de auto-descartarse.
const TOAST_TTL: Duration = Duration::from_secs(4);

/// Hash estable de una cadena → `key` para animaciones implícitas (el mismo
/// contenido produce siempre la misma key entre rebuilds).
fn key_of(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Empuja un toast al stack y programa su expiración.
fn push_toast(model: &mut Model, handle: &Handle<Msg>, toast: Toast) {
    let id = toast.id;
    model.toasts.push(toast);
    handle.spawn(move || {
        std::thread::sleep(TOAST_TTL);
        Msg::ToastExpire(id)
    });
}

/// Arranca la cadena de ticks (~50ms) si hay una corrida en vuelo y no hay ya
/// una corriendo. Se auto-detiene cuando `running_from` vuelve a `None`.
fn arm_tick(model: &mut Model, handle: &Handle<Msg>) {
    if model.ticking || model.running_from.is_none() {
        return;
    }
    model.ticking = true;
    handle.spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        Msg::Tick
    });
}

fn header_bar(model: &Model, palette: &Palette) -> View<Msg> {
    let origen = match (&model.source, &model.load_error) {
        (_, Some(err)) => format!("error de carga: {err}"),
        (Some(p), None) => p.display().to_string(),
        (None, None) => "(demo embebido — pasá una ruta .pluma-nb para abrir un archivo)".to_string(),
    };
    let digest = model
        .notebook
        .notebook_digest()
        .map(|d| short_hex(&d))
        .unwrap_or_else(|| "—(ciclo)".to_string());
    let modo = match model.mode {
        Mode::Linear => "lineal".to_string(),
        Mode::Canvas => format!(
            "canvas · viewport {:+.0},{:+.0} · zoom {:.2}× · Home recentra · F fit-all · 0/+/- zoom",
            model.viewport.0, model.viewport.1, model.zoom,
        ),
    };
    let running = model
        .running_from
        .map(|id| format!(" · ejecutando #{id}…"))
        .unwrap_or_default();
    let texto = format!(
        "pluma-notebook · {} celdas · modo {} · digest {}{} · {}",
        model.notebook.len(),
        modo,
        digest,
        running,
        origen,
    );
    let color = if model.load_error.is_some() { palette.fg_error } else { palette.fg_muted };

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .text_aligned(texto, 11.0, color, Alignment::Start)
}

// ---------------------------------------------------------------------
// Modo lineal — stack vertical, lo de siempre.
// ---------------------------------------------------------------------

fn linear_view(nb: &Notebook, palette: &Palette) -> View<Msg> {
    let cards: Vec<View<Msg>> = nb.cells().iter().map(|c| linear_card(c, palette)).collect();
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(cards)
}

fn linear_card(cell: &Cell, palette: &Palette) -> View<Msg> {
    let height = linear_body_height(&cell.source) + 30.0;
    card_with_height(
        cell,
        palette,
        Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: length(height) },
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        },
        linear_body_height(&cell.source),
    )
}

const LINEAR_MAX_BODY_LINES: usize = 8;
const LINEAR_MAX_BODY_CHARS: usize = 400;

fn linear_body_height(source: &str) -> f32 {
    let lines = source.lines().count().min(LINEAR_MAX_BODY_LINES).max(1);
    16.0 * lines as f32 + 12.0
}

// ---------------------------------------------------------------------
// Modo canvas — cada celda en su (x, y) + conectores S-codo del DAG.
// ---------------------------------------------------------------------

/// Tamaño fijo del card en canvas — el alto del body sale del card, no
/// del texto, para que los conectores sean estables.
const CANVAS_CARD_W: f32 = 280.0;
const CANVAS_CARD_H: f32 = 112.0;
/// Alto extendido cuando la card está en modo edición — da espacio
/// para que el editor multilínea muestre varias líneas y caret/brackets.
const CANVAS_CARD_H_EDITING: f32 = 240.0;
/// Ancho del scrollbar vertical.
const SCROLLBAR_W: f32 = 10.0;
/// Aproximación del alto visible del canvas (ventana 760 − header 28).
/// Sin medir el frame real; ajustamos si la ventana cambia mucho.
const APPROX_VIEWPORT_H: f32 = 732.0;
const CANVAS_BODY_LINES_VISIBLE: usize = 3;
const CANVAS_HEADER_H: f32 = 18.0;
const CANVAS_FOOTER_H: f32 = 16.0;
const CANVAS_BODY_H: f32 = CANVAS_CARD_H - CANVAS_HEADER_H - CANVAS_FOOTER_H;
const CANVAS_EDITOR_H: f32 = CANVAS_CARD_H_EDITING - CANVAS_HEADER_H - CANVAS_FOOTER_H;

fn canvas_view(
    nb: &Notebook,
    viewport: (f32, f32),
    zoom: f32,
    editing: Option<&EditState>,
    running_from: Option<CellId>,
    palette: &Palette,
) -> View<Msg> {
    let (vx, vy) = viewport;
    let card_w = CANVAS_CARD_W * zoom;
    let card_h = CANVAS_CARD_H * zoom;
    let mut children: Vec<View<Msg>> = Vec::new();

    // Aristas primero (capa de fondo) — del prerrequisito al dependiente.
    for cell in nb.cells() {
        let Some(child_pos) = cell.position else { continue };
        for dep_id in &cell.depends_on {
            let Some(dep) = nb.cell(*dep_id) else { continue };
            let Some(dep_pos) = dep.position else { continue };
            let x1 = vx + dep_pos.x * zoom + card_w * 0.5;
            let y1 = vy + dep_pos.y * zoom + card_h;
            let x2 = vx + child_pos.x * zoom + card_w * 0.5;
            let y2 = vy + child_pos.y * zoom;
            children.extend(edge_segments(x1, y1, x2, y2, palette.edge));
        }
    }

    // Cards encima — draggables (mueven Cell::position). En edición se
    // mantienen al tamaño base (sin zoom) para que el editor multilínea
    // sea legible aunque el resto esté zoom-out.
    for cell in nb.cells() {
        let Some(pos) = cell.position else { continue };
        let edit = editing.filter(|e| e.id == cell.id);
        let scale = if edit.is_some() { 1.0 } else { zoom };
        let running = running_from == Some(cell.id);
        children.push(canvas_card_scaled(
            cell,
            edit,
            running,
            palette,
            vx + pos.x * zoom,
            vy + pos.y * zoom,
            scale,
        ));
    }

    let huerfanas = nb.cells().iter().filter(|c| c.position.is_none()).count();
    if huerfanas > 0 {
        children.push(orphan_notice(huerfanas, palette));
    }

    // Scrollbar vertical — sólo si el contenido excede el viewport.
    if let Some(sb) = scrollbar_v(nb, viewport, palette) {
        children.push(sb);
    }

    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .draggable(|phase, dx, dy| match phase {
        DragPhase::Move => Some(Msg::PanBy(dx, dy)),
        DragPhase::End => None,
    })
    .children(children)
}

/// Devuelve `Some(view)` con el track + thumb a la derecha cuando el
/// contenido (bounding box vertical de las celdas) excede el viewport;
/// `None` si todo cabe.
fn scrollbar_v(nb: &Notebook, viewport: (f32, f32), palette: &Palette) -> Option<View<Msg>> {
    let (min_y, max_y) = vertical_bounds(nb)?;
    let content_h = (max_y - min_y) + 80.0; // margen para que el thumb no toque el borde
    if content_h <= APPROX_VIEWPORT_H {
        return None;
    }

    // viewport.1 (vy) traduce el contenido: contenido visible es
    // [-vy + min_y, -vy + min_y + APPROX_VIEWPORT_H). El "scroll
    // logical" = posición desde el tope del contenido.
    let scroll = (-viewport.1 - min_y + 40.0).clamp(0.0, content_h - APPROX_VIEWPORT_H);
    let thumb_ratio = APPROX_VIEWPORT_H / content_h;
    let thumb_h = (thumb_ratio * APPROX_VIEWPORT_H).max(28.0);
    let thumb_y = (scroll / content_h) * APPROX_VIEWPORT_H;

    // Track: rectángulo de SCROLLBAR_W de ancho pegado a la derecha.
    let track = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: auto(),
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size { width: length(SCROLLBAR_W), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg_panel);

    // Thumb: rectángulo más claro, draggable para scrollear.
    // Drag del thumb mueve el viewport proporcionalmente: cada px
    // visual = (content_h / viewport_h) px de contenido.
    let scale = content_h / APPROX_VIEWPORT_H;
    let thumb = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: auto(),
            top: length(thumb_y),
            right: length(1.0_f32),
            bottom: auto(),
        },
        size: Size { width: length(SCROLLBAR_W - 2.0), height: length(thumb_h) },
        ..Default::default()
    })
    .fill(palette.accent_stale)
    .hover_fill(palette.accent_fresh)
    .draggable(move |phase, _dx, dy| match phase {
        // dy del drag visual → -dy*scale en viewport.y (negativo porque
        // mover el thumb hacia abajo desplaza el contenido hacia arriba).
        DragPhase::Move => Some(Msg::PanBy(0.0, -dy * scale)),
        DragPhase::End => None,
    });

    Some(track.children(vec![thumb]))
}

/// Bounding box vertical de las celdas con posición (top, bottom).
/// `None` si no hay celdas posicionadas.
fn vertical_bounds(nb: &Notebook) -> Option<(f32, f32)> {
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for c in nb.cells() {
        if let Some(p) = c.position {
            if p.y < min_y {
                min_y = p.y;
            }
            if p.y + CANVAS_CARD_H > max_y {
                max_y = p.y + CANVAS_CARD_H;
            }
        }
    }
    if !min_y.is_finite() {
        None
    } else {
        Some((min_y, max_y))
    }
}

fn canvas_card_scaled(
    cell: &Cell,
    edit: Option<&EditState>,
    running: bool,
    palette: &Palette,
    x: f32,
    y: f32,
    scale: f32,
) -> View<Msg> {
    let id = cell.id;
    let editing = edit.is_some();
    let card_w = CANVAS_CARD_W * scale;
    let total_h = if editing { CANVAS_CARD_H_EDITING } else { CANVAS_CARD_H * scale };
    let body_h = if editing { CANVAS_EDITOR_H } else { CANVAS_BODY_H * scale };
    let (header, body) = card_header_body(cell, palette, body_h);
    // Mientras la celda se ejecuta, su footer es un shimmer (placeholder del
    // resultado). Al volver, el output entra con un pop-in suave.
    let footer = if running {
        skeleton_footer()
    } else {
        let f = output_footer(cell.last_output.as_ref(), palette);
        match cell.last_output.as_ref() {
            Some(o) => f.animated_enter(key_of(&format!("{id}:{}", format_output(o))), motion::NORMAL),
            None => f,
        }
    };
    let run_button = run_button_view(id, palette);
    let edit_button = edit_button_view(id, editing, palette);

    // En edición el body se reemplaza por el editor.
    let body = match edit {
        None => body,
        Some(es) => edit_input_view(&es.editor, body_h, language_of(cell)),
    };

    // En edición la card no debe ser draggable (interfiere con foco).
    let mut wrapper = View::new(Style {
        flex_direction: FlexDirection::Column,
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(card_w), height: length(total_h) },
        ..Default::default()
    })
    .fill(palette.bg_card)
    .clip(true);
    if !editing {
        wrapper = wrapper.draggable(move |phase, dx, dy| match phase {
            DragPhase::Move => Some(Msg::MoveCell { id, dx, dy }),
            DragPhase::End => None,
        });
    }
    wrapper.children(vec![header, body, footer, edit_button, run_button])
}

fn edit_input_view(editor: &EditorState, body_h: f32, language: Language) -> View<Msg> {
    let theme = Theme::dark();
    let ep = EditorPalette::from_theme(&theme);
    let metrics = EditorMetrics::for_font_size(12.0);
    let visible = (body_h / metrics.line_height).max(1.0) as usize;
    // En modo edición la card pierde su drag (en canvas_card), así que el
    // editor recibe los eventos del mouse para click-posiciona-caret y
    // drag-selecciona, como en nada.
    text_editor_view_highlighted(editor, &ep, metrics, visible, language, |ev| {
        Some(Msg::EditorPointer(ev))
    })
}

fn language_of(cell: &Cell) -> Language {
    match &cell.kind {
        CellKind::Code { language } => Language::from_cell_language(language),
        _ => Language::Plain,
    }
}

/// Footer en estado "ejecutando": una banda con shimmer que ocupa el lugar
/// del output que está por llegar. Requiere el tick de `Msg::Tick` para animar.
fn skeleton_footer() -> View<Msg> {
    let theme = Theme::dark();
    let sp = SkeletonPalette::from_theme(&theme);
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(CANVAS_FOOTER_H) },
        ..Default::default()
    })
    .clip(true)
    .children(vec![skeleton_view(&sp)])
}

fn output_footer(out: Option<&CellOutput>, palette: &Palette) -> View<Msg> {
    let (text, color) = match out {
        None => ("∅ sin output".to_string(), palette.fg_muted),
        Some(o) => (format_output(o), palette.accent_fresh),
    };
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(CANVAS_FOOTER_H) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .text_aligned(text, 10.0, color, Alignment::Start)
}

/// Una línea legible del output. Prioriza `value`; cae al primer renglón
/// de stdout si no hay value; muestra `[port_kind]` como prefijo del tipo.
fn format_output(o: &CellOutput) -> String {
    let port = o.payload.port_kind();
    if let Some(v) = &o.value {
        return format!("→[{}] {}", port, truncate_line(v, 28));
    }
    if !o.stdout.is_empty() {
        let line = o.stdout.lines().next().unwrap_or("");
        return format!("→[{}] {}", port, truncate_line(line, 28));
    }
    format!("→[{}]", port)
}

fn truncate_line(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max - 1).collect();
        format!("{cut}…")
    }
}

const RUN_BTN_SIZE: f32 = 18.0;

fn run_button_view(id: CellId, palette: &Palette) -> View<Msg> {
    // Esquina superior derecha.
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: auto(),
            top: length(2.0_f32),
            right: length(4.0_f32),
            bottom: auto(),
        },
        size: Size { width: length(RUN_BTN_SIZE), height: length(RUN_BTN_SIZE) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.edge)
    .hover_fill(palette.accent_fresh)
    .on_click(Msg::RunFrom(id))
    .text_aligned(">", 10.0, palette.bg, Alignment::Center)
}

fn edit_button_view(id: CellId, editing: bool, palette: &Palette) -> View<Msg> {
    // A la izquierda del botón ▶. Cuando hay edición activa, este
    // mismo botón pasa a representar "commit" (✓).
    let (glyph, msg) = if editing {
        ("OK", Msg::CommitEdit)
    } else {
        ("*", Msg::StartEdit(id))
    };
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: auto(),
            top: length(2.0_f32),
            right: length(26.0_f32),
            bottom: auto(),
        },
        size: Size { width: length(RUN_BTN_SIZE), height: length(RUN_BTN_SIZE) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .hover_fill(palette.accent_fresh)
    .on_click(msg)
    // Ícono del botón editar/commit: vector si el glifo está en el catálogo,
    // texto si no. El nodo ya tiene size fijo (RUN_BTN_SIZE), así que el View
    // absoluto 100% del helper se dimensiona correctamente.
    .children(vec![llimphi_icons::glyph_or_text_view(glyph, 10.0, palette.fg_text, 1.8)])
}

fn orphan_notice(n: usize, palette: &Palette) -> View<Msg> {
    let texto = format!(
        "{n} celda(s) sin posición — no se muestran en canvas. Asigná `Cell::position` para incluirlas."
    );
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(12.0_f32),
            top: length(8.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(560.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .text_aligned(texto, 10.0, palette.fg_muted, Alignment::Start)
}

// ---------------------------------------------------------------------
// Conectores S-codo — copia del patrón de pluma-editor-llimphi.
// ---------------------------------------------------------------------

fn edge_segments(x1: f32, y1: f32, x2: f32, y2: f32, color: Color) -> Vec<View<Msg>> {
    let stroke = 1.6f32;
    let half = stroke * 0.5;
    let mid_y = (y1 + y2) * 0.5;
    let mut out: Vec<View<Msg>> = Vec::with_capacity(3);

    out.push(line_view(x1 - half, y1, stroke, (mid_y - y1).abs().max(stroke), color));
    if (x2 - x1).abs() > stroke {
        let (xl, xr) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
        out.push(line_view(xl - half, mid_y - half, (xr - xl) + stroke, stroke, color));
    }
    out.push(line_view(x2 - half, mid_y, stroke, (y2 - mid_y).abs().max(stroke), color));
    out
}

fn line_view(x: f32, y: f32, w: f32, h: f32, color: Color) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(w), height: length(h) },
        ..Default::default()
    })
    .fill(color)
}

// ---------------------------------------------------------------------
// Card compartido — usado por ambos modos. El style del wrapper lo pone
// cada modo (lineal = flex column; canvas = absoluto en (x,y)).
// ---------------------------------------------------------------------

fn card_with_height(cell: &Cell, palette: &Palette, wrapper: Style, body_h: f32) -> View<Msg> {
    let (header, body) = card_header_body(cell, palette, body_h);
    View::new(wrapper).fill(palette.bg_card).clip(true).children(vec![header, body])
}

fn card_header_body(cell: &Cell, palette: &Palette, body_h: f32) -> (View<Msg>, View<Msg>) {
    let header_text = format!(
        "[{}] #{}  ·  {}",
        kind_label(&cell.kind),
        cell.id,
        state_label(cell.state)
    );
    let header_color = match cell.state {
        CellState::Fresh => palette.accent_fresh,
        CellState::Stale => palette.accent_stale,
        CellState::Failed => palette.accent_failed,
    };

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            // Espacio reservado para los botones ✎ y ▶ en modo canvas.
            right: length(50.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, header_color, Alignment::Start);

    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(body_h) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        truncate_source(&cell.source, CANVAS_BODY_LINES_VISIBLE.max(2)),
        12.0,
        palette.fg_text,
        Alignment::Start,
    );

    (header, body)
}

fn kind_label(k: &CellKind) -> String {
    match k {
        CellKind::Markdown => "markdown".into(),
        CellKind::Code { language } => format!("code:{language}"),
        CellKind::Embed { module } => format!("embed:{module}"),
    }
}

fn state_label(s: CellState) -> &'static str {
    match s {
        CellState::Fresh => "fresh",
        CellState::Stale => "stale",
        CellState::Failed => "failed",
    }
}

fn truncate_source(s: &str, max_lines: usize) -> String {
    let mut out = String::new();
    for (i, line) in s.lines().enumerate() {
        if i >= max_lines || out.len() + line.len() + 1 > LINEAR_MAX_BODY_CHARS {
            out.push_str("\n…");
            break;
        }
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    if out.is_empty() {
        out.push_str(s);
    }
    out
}

fn short_hex(d: &[u8; 32]) -> String {
    d[..6].iter().map(|b| format!("{b:02x}")).collect()
}

/// Notebook embebido — modo canvas: cuatro celdas con posición en (x, y)
/// para que el binario sin argumento muestre el modo espacial.
fn demo_notebook() -> Notebook {
    use pluma_notebook_core::Position as P;

    let mut nb = Notebook::new();
    let intro = nb.push(
        CellKind::Markdown,
        "Demo · ✎ edita (multilínea) · Ctrl+Enter commit · Esc cancela · ▶ corre run_from.",
    );
    let datos = nb.push(
        CellKind::Code { language: "wat".into() },
        "(module\n  (func (export \"main\") (result i32)\n    i32.const 21))",
    );
    let media = nb.push(
        CellKind::Code { language: "wat".into() },
        "(module\n  (func (export \"main\") (result i32)\n    i32.const 42))",
    );
    let py = nb.push(
        CellKind::Code { language: "python".into() },
        "n = 10\nprint(f\"suma 1..{n}\")\nsum(range(1, n + 1))",
    );
    let grafico = nb.push(
        CellKind::Embed { module: "pineal".into() },
        "barras: kilos por semana",
    );
    nb.add_dependency(media, datos);
    nb.add_dependency(grafico, datos);
    nb.add_dependency(grafico, media);
    nb.add_dependency(grafico, py);

    // Layout: intro arriba, datos al centro, media+python como hijos a
    // izquierda y centro, gráfico a la derecha como sink de los tres.
    nb.set_position(intro, Some(P::new(40.0, 40.0)));
    nb.set_position(datos, Some(P::new(40.0, 170.0)));
    nb.set_position(media, Some(P::new(40.0, 320.0)));
    nb.set_position(py, Some(P::new(310.0, 170.0)));
    nb.set_position(grafico, Some(P::new(310.0, 320.0)));

    nb
}

/// Calcula `(zoom, viewport)` para que TODO el bounding box entre en el
/// viewport visible aproximado. Margen de 40 px alrededor.
fn fit_all(nb: &Notebook) -> (f32, (f32, f32)) {
    let bounds = bounds_of_cells(nb);
    let Some((min_x, min_y, max_x, max_y)) = bounds else {
        return (1.0, (0.0, 0.0));
    };
    let content_w = (max_x - min_x).max(1.0);
    let content_h = (max_y - min_y).max(1.0);
    // Asumimos área visible ~ ventana 980 - scrollbar 10, 760 - header 28.
    let avail_w = 970.0 - 80.0;
    let avail_h = 732.0 - 80.0;
    let zoom = (avail_w / content_w).min(avail_h / content_h).clamp(0.25, 4.0);
    // Después de zoom, recentramos.
    let vx = 40.0 - min_x * zoom;
    let vy = 40.0 - min_y * zoom;
    (zoom, (vx, vy))
}

/// Bounding box (min_x, min_y, max_x, max_y) de las celdas posicionadas.
fn bounds_of_cells(nb: &Notebook) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for c in nb.cells() {
        if let Some(p) = c.position {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x + CANVAS_CARD_W);
            max_y = max_y.max(p.y + CANVAS_CARD_H);
        }
    }
    if !min_x.is_finite() {
        None
    } else {
        Some((min_x, min_y, max_x, max_y))
    }
}

/// Calcula un viewport que deje el bounding box de las celdas con
/// `position` visible en la esquina superior-izquierda (margen 40 px).
/// Si no hay celdas posicionadas, devuelve `(0, 0)`.
fn viewport_to_fit(nb: &Notebook) -> (f32, f32) {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    for c in nb.cells() {
        if let Some(p) = c.position {
            if p.x < min_x {
                min_x = p.x;
            }
            if p.y < min_y {
                min_y = p.y;
            }
        }
    }
    if !min_x.is_finite() || !min_y.is_finite() {
        return (0.0, 0.0);
    }
    (40.0 - min_x, 40.0 - min_y)
}

fn main() {
    rimay_localize::init();
    let wawa_cfg = wawa_config::WawaConfig::load();
    let _ = rimay_localize::set_locale(&wawa_cfg.lang);
    llimphi_ui::run::<Viewer>();
}
