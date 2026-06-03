//! `chaka-app-llimphi` — interfaz gráfica del transpilador COBOL → Rust.
//!
//! Tres paneles sobre Llimphi:
//!
//! 1. **Corpus** (izquierda, 220 px): árbol con los `.cob` del directorio
//!    `corpus/` colocado al lado del binario o el primer argumento. Click
//!    carga el archivo al editor.
//! 2. **Editor COBOL** (centro): `text-editor` editable, con scroll + gutter
//!    + selección + undo/redo (todo lo que da el widget). Cada edición
//!    recorre el pipeline `lexer → parser → ir → codegen → shadow` y
//!    refresca los tabs de la derecha en vivo.
//! 3. **Tabs** (derecha): cuatro paneles read-only — salida del intérprete
//!    sombra (con comparación contra `<archivo>.expected` si existe), el
//!    Rust generado por `chaka-codegen`, el IR como JSON, y los
//!    diagnósticos (errores de léxico/parseo + verbos no transpilados).
//!
//! Atajos: Ctrl+S guarda el .cob al disco, Ctrl+R re-corre el pipeline,
//! Ctrl+1..4 cambian de tab, rueda hace scroll en el editor activo.
//!
//! La estética hereda el `Theme` de `llimphi-theme` (default Dark, ciclable
//! con el switcher en el header).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use chaka_codegen::Target;
use chaka_ir::{Ir, PerformTarget, Stmt};
use chaka_lexer::{lex, SourceFormat};
use chaka_parser::parse;
use chaka_shadow::{interpret, Halt, Outcome};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_tabs::{tabs_view, TabsPalette, TabsSpec};
use llimphi_widget_text_editor::{
    text_editor_view_highlighted, EditorMetrics, EditorPalette, EditorState, Language,
    PointerEvent,
};
use llimphi_widget_theme_switcher::theme_switcher_view;
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_motion::{animate, motion, Tween};
use llimphi_clipboard::SystemClipboard;

const TREE_WIDTH: f32 = 240.0;
const STATUS_H: f32 = 24.0;
const TAB_STRIP_H: f32 = 28.0;
const TREE_ROW_H: f32 = 22.0;
const TREE_INDENT: f32 = 16.0;
/// Cuántas líneas como máximo se rendean en cada editor por frame.
/// Con `line_height ≈ 18 px` cubre ~720 px de alto útil — más que el
/// viewport típico de la ventana.
const EDITOR_VISIBLE_LINES: usize = 60;
/// Tope para no congelar el pipeline ante un archivo gigante pegado por
/// accidente. El corpus chaka real no supera ~30 KB; cualquier cosa más
/// grande probablemente no es un programa COBOL legítimo.
const MAX_SOURCE_BYTES: usize = 256 * 1024;

// Colores de status — verde/ámbar para los chips de status cuando no
// hay banner canónico (`banner_view` cubre success/error/info).
const ACCENT_OK: Color = Color::from_rgba8(0xa3, 0xbe, 0x8c, 0xff);
const ACCENT_WARN: Color = Color::from_rgba8(0xeb, 0xcb, 0x8b, 0xff);

#[derive(Clone)]
enum Msg {
    /// Carga el archivo en el índice `i` del árbol.
    OpenFile(usize),
    /// Tecla a aplicar al editor del .cob.
    EditKey(KeyEvent),
    /// Click/drag sobre el área de texto del editor del .cob.
    EditorPointer(PointerEvent),
    /// Scroll de la rueda — aplicado al editor activo (.cob o un viewer
    /// según el tab seleccionado).
    Scroll(i32),
    /// Cambia de tab en el panel derecho.
    SelectTab(OutputTab),
    /// Re-corre el pipeline sobre el buffer actual (Ctrl+R o botón).
    Run,
    /// Guarda el buffer al disco (sobreescribiendo el .cob abierto).
    Save,
    /// Cicla el theme.
    CycleTheme(Theme),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Navegación ↑/↓ por la fila activa del menú principal.
    MenuNav(i32),
    /// Enter sobre la fila activa del menú principal.
    MenuActivate,
    /// Tick de animación de aparición/swap (re-render).
    MenuTick,
    /// Navegación ↑/↓ por la fila activa del menú de edición.
    EditNav(i32),
    /// Enter sobre la fila activa del menú de edición.
    EditActivate,
    /// Right-click en el área de trabajo → abre el menú de edición en
    /// `(x, y)` de ventana, operando sobre el editor del .cob.
    EditMenuOpen(f32, f32),
    /// Acción elegida en el menú de edición.
    EditMenuAction(EditAction),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputTab {
    Salida = 0,
    Rust = 1,
    Ir = 2,
    Diag = 3,
}

impl OutputTab {
    fn from_index(i: usize) -> Self {
        match i {
            0 => OutputTab::Salida,
            1 => OutputTab::Rust,
            2 => OutputTab::Ir,
            _ => OutputTab::Diag,
        }
    }
    fn index(self) -> usize {
        self as usize
    }
}

struct CorpusEntry {
    label: String,
    path: PathBuf,
}

/// Resultado de correr el pipeline completo sobre el buffer actual.
struct Pipeline {
    /// Salida del intérprete sombra (líneas + halt). `None` si hubo error
    /// antes de poder ejecutar (lex/parse).
    outcome: Option<Outcome>,
    /// Diff contra `<archivo>.expected` si existe — `Some((ok, total_lines))`
    /// donde `ok` indica si todas las líneas coinciden ignorando trailing
    /// whitespace.
    compare: Option<(bool, usize, String)>,
    /// Rust emitido por `chaka-codegen`.
    rust: String,
    /// IR serializado en JSON (con tolerancia: si la serialización falla,
    /// queda un mensaje explicativo).
    ir_json: String,
    /// Errores de léxico o parseo + lista de verbos COBOL no transpilados.
    diagnostics: String,
    /// Estado resumen para el banner / status bar.
    summary: PipelineSummary,
}

#[derive(Clone, Copy)]
enum PipelineSummary {
    /// No hay archivo cargado todavía.
    Idle,
    /// Pipeline OK; intérprete halt = Normal o StopRun. Si hay
    /// `.expected`: `match_ok` indica si coincide.
    Ok { lines: usize, match_ok: Option<bool> },
    /// Pipeline OK pero el intérprete pegó el tope de pasos.
    StepLimit,
    /// Falló lex/parse — `Run` no se llegó a ejecutar.
    PipelineError,
}

struct Model {
    entries: Vec<CorpusEntry>,
    /// Índice del archivo abierto en `entries`. `None` si nada se ha
    /// cargado todavía.
    open: Option<usize>,
    /// Editor principal — el .cob editable.
    cobol: EditorState,
    /// Si el buffer divergió del disco desde el último load/save.
    dirty: bool,
    /// Acumulado de drag en el editor del .cob.
    drag_accum: (f32, f32),
    /// Resultado del último pipeline corrido. Se refresca en cada
    /// `EditKey` (lazy: si el buffer no cambió, no se vuelve a parsear,
    /// pero el costo es despreciable para archivos del corpus).
    pipe: Pipeline,
    /// Viewers read-only para los outputs. Se reasignan en cada refresh.
    view_salida: EditorState,
    view_rust: EditorState,
    view_ir: EditorState,
    view_diag: EditorState,
    /// Tab actualmente visible en el panel derecho.
    active_tab: OutputTab,
    theme: Theme,
    /// Mensaje del status bar (último evento — save, theme, error, etc.).
    status: String,
    /// Portapapeles del sistema para cortar/copiar/pegar desde el menú.
    clipboard: SystemClipboard,
    /// Menú principal: índice del menú raíz abierto (`None` cerrado).
    menu_open: Option<usize>,
    /// Menú de edición contextual: ancla `(x, y)` en ventana (`None` cerrado).
    edit_menu: Option<(f32, f32)>,
    /// Fila resaltada por teclado en el menú principal (`usize::MAX` = ninguna).
    menu_active: usize,
    /// Animación de aparición/swap del dropdown principal.
    menu_anim: Tween<f32>,
    /// Fila resaltada por teclado en el menú de edición (`usize::MAX` = ninguna).
    edit_active: usize,
    /// Animación de aparición del menú de edición.
    edit_anim: Tween<f32>,
}

struct ChakaApp;

impl App for ChakaApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "chaka — COBOL → Rust"
    }

    fn initial_size() -> (u32, u32) {
        (1400, 860)
    }

    fn init(_handle: &Handle<Msg>) -> Model {
        let corpus_root = locate_corpus();
        let entries = scan_corpus(&corpus_root);
        let pipe = Pipeline::idle();
        let theme = Theme::dark();
        let status = format!(
            "{} programas en {}",
            entries.len(),
            corpus_root.display(),
        );
        let mut model = Model {
            entries,
            open: None,
            cobol: EditorState::new(),
            dirty: false,
            drag_accum: (0.0, 0.0),
            pipe,
            view_salida: EditorState::new(),
            view_rust: EditorState::new(),
            view_ir: EditorState::new(),
            view_diag: EditorState::new(),
            active_tab: OutputTab::Salida,
            theme,
            status,
            clipboard: SystemClipboard::new(),
            menu_open: None,
            edit_menu: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
        };
        // Si hay corpus, abrimos el primero — pantalla inicial poblada
        // en vez de placeholders vacíos.
        if !model.entries.is_empty() {
            model = open_entry(model, 0);
        }
        model
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::OpenFile(i) => open_entry(model, i),
            Msg::EditKey(ev) => apply_edit_key(model, ev),
            Msg::EditorPointer(ev) => apply_editor_pointer(model, ev),
            Msg::Scroll(delta) => {
                let mut m = model;
                m.cobol.scroll_by(delta);
                m
            }
            Msg::SelectTab(t) => Model {
                active_tab: t,
                ..model
            },
            Msg::Run => recompute(model),
            Msg::Save => save_open(model),
            Msg::CycleTheme(t) => {
                let mut m = model;
                m.theme = t;
                m.status = format!("✓ tema: {}", t.name);
                m
            }
            Msg::MenuOpen(i) => {
                let mut m = model;
                m.menu_open = i;
                m.edit_menu = None;
                m.menu_active = usize::MAX;
                if i.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
                m
            }
            Msg::MenuCommand(cmd) => handle_menu_command(model, cmd, handle),
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
            Msg::MenuTick => model,
            Msg::EditNav(dir) => {
                let mut m = model;
                let flags = EditFlags::from_editor(&m.cobol, false);
                m.edit_active = editmenu::edit_menu_step(flags, m.edit_active, dir);
                m
            }
            Msg::EditActivate => {
                let flags = EditFlags::from_editor(&model.cobol, false);
                if let Some(a) = editmenu::edit_menu_action_at(flags, model.edit_active) {
                    return apply_edit_menu_action(model, a);
                }
                model
            }
            Msg::EditMenuOpen(x, y) => {
                let mut m = model;
                m.edit_menu = Some((x, y));
                m.menu_open = None;
                m.edit_active = usize::MAX;
                m.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(handle, motion::FAST, || Msg::MenuTick);
                m
            }
            Msg::EditMenuAction(action) => apply_edit_menu_action(model, action),
            Msg::CloseMenus => {
                let mut m = model;
                m.menu_open = None;
                m.edit_menu = None;
                m.menu_active = usize::MAX;
                m.edit_active = usize::MAX;
                m
            }
        }
    }

    fn on_wheel(
        _model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Self::Msg> {
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
        // Menú de edición abierto: ↑/↓ navegan, Enter ejecuta, Esc cierra.
        if model.edit_menu.is_some() {
            match &event.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::EditNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::EditNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::EditActivate),
                _ => return None,
            }
        }
        // Atajos globales con Ctrl.
        if event.modifiers.ctrl {
            if matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("s")) {
                return Some(Msg::Save);
            }
            if matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("r")) {
                return Some(Msg::Run);
            }
            // Ctrl+1..4 — cambio de tab. `Key::Character` viene con el
            // dígito como '1'..'4' en la mayoría de layouts.
            if let Key::Character(s) = &event.key {
                if let Some(d) = s.chars().next().and_then(|c| c.to_digit(10)) {
                    if (1..=4).contains(&d) {
                        return Some(Msg::SelectTab(OutputTab::from_index(d as usize - 1)));
                    }
                }
            }
        }
        // Resto va al editor del .cob.
        Some(Msg::EditKey(event.clone()))
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = model.theme;
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let header = header_view(model, &theme);
        let body = body_view(model, &theme);
        let status = status_bar(model, &theme);

        // El right-click se engancha en la raíz (origen 0,0 → las coords
        // locales que llegan ya son de ventana) y abre el menú de edición
        // sobre el editor del .cob.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
        .children(vec![menubar, header, body, status])
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // El menú de edición tiene prioridad si está abierto.
        if let Some((x, y)) = model.edit_menu {
            let flags = EditFlags::from_editor(&model.cobol, false);
            let (w, h) = Self::initial_size();
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                (w as f32, h as f32),
                &model.theme,
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras { appear: model.edit_anim.value(), ..Default::default() },
            ));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &model.theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(
    menu: &'a app_bus::AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
    let (w, h) = ChakaApp::initial_size();
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: (w as f32, h as f32),
        height: MENU_H,
        on_open: std::sync::Arc::new(Msg::MenuOpen),
        on_command: std::sync::Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

// ── Menú principal + menú de edición contextual ───────────────────────────

/// Construye el menú principal reflejando el estado real del editor del
/// .cob (ítems de Editar grises cuando no hay selección / historial).
fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};
    let has_open = model.open.is_some();
    let has_sel = model.cobol.has_selection();
    let can_undo = model.cobol.can_undo();
    let can_redo = model.cobol.can_redo();
    let has_text = !model.cobol.is_empty();

    let mut guardar = MenuItem::new("Guardar", "file.save").shortcut("Ctrl+S");
    if !has_open { guardar = guardar.disabled(); }

    let mut undo = MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z");
    if !can_undo { undo = undo.disabled(); }
    let mut redo = MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y");
    if !can_redo { redo = redo.disabled(); }
    let mut cut = MenuItem::new("Cortar", "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new("Copiar", "edit.copy").shortcut("Ctrl+C");
    if !has_sel { cut = cut.disabled(); copy = copy.disabled(); }
    let paste = MenuItem::new("Pegar", "edit.paste").shortcut("Ctrl+V");
    let mut sel_all = MenuItem::new("Seleccionar todo", "edit.selectall")
        .shortcut("Ctrl+A")
        .separated();
    if !has_text { sel_all = sel_all.disabled(); }

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(guardar),
        )
        .menu(
            Menu::new("Editar")
                .item(undo)
                .item(redo)
                .item(cut)
                .item(copy)
                .item(paste)
                .item(sel_all),
        )
        .menu(
            Menu::new("Ejecutar")
                .item(MenuItem::new("Correr pipeline", "run.run").shortcut("Ctrl+R")),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Salida", "view.tab.salida").shortcut("Ctrl+1"))
                .item(MenuItem::new("Rust generado", "view.tab.rust").shortcut("Ctrl+2"))
                .item(MenuItem::new("IR", "view.tab.ir").shortcut("Ctrl+3"))
                .item(MenuItem::new("Diagnósticos", "view.tab.diag").shortcut("Ctrl+4"))
                .item(MenuItem::new("Cambiar tema", "view.theme").separated()),
        )
        .menu(
            Menu::new("Ayuda")
                .item(MenuItem::new("Acerca de chaka", "help.about")),
        )
}

/// Traduce el `command` del menú principal al `Msg` real y lo despacha.
/// Cierra el menú antes de actuar.
fn handle_menu_command(mut model: Model, command: String, handle: &Handle<Msg>) -> Model {
    model.menu_open = None;
    let target = match command.as_str() {
        "file.save" => Some(Msg::Save),
        "edit.undo" => Some(Msg::EditMenuAction(EditAction::Undo)),
        "edit.redo" => Some(Msg::EditMenuAction(EditAction::Redo)),
        "edit.cut" => Some(Msg::EditMenuAction(EditAction::Cut)),
        "edit.copy" => Some(Msg::EditMenuAction(EditAction::Copy)),
        "edit.paste" => Some(Msg::EditMenuAction(EditAction::Paste)),
        "edit.selectall" => Some(Msg::EditMenuAction(EditAction::SelectAll)),
        "run.run" => Some(Msg::Run),
        "view.tab.salida" => Some(Msg::SelectTab(OutputTab::Salida)),
        "view.tab.rust" => Some(Msg::SelectTab(OutputTab::Rust)),
        "view.tab.ir" => Some(Msg::SelectTab(OutputTab::Ir)),
        "view.tab.diag" => Some(Msg::SelectTab(OutputTab::Diag)),
        "view.theme" => Some(Msg::CycleTheme(Theme::next_after(model.theme.name))),
        "help.about" => {
            model.status =
                "chaka · transpilador COBOL → Rust · pipeline lex→parse→ir→codegen→shadow".into();
            None
        }
        _ => None,
    };
    match target {
        Some(msg) => ChakaApp::update(model, msg, handle),
        None => model,
    }
}

/// Aplica una acción del menú de edición al editor del .cob, replicando
/// el bookkeeping de `apply_edit_key` (dirty + recompute del pipeline +
/// auto-scroll). Cierra el menú de edición.
fn apply_edit_menu_action(mut model: Model, action: EditAction) -> Model {
    model.edit_menu = None;
    let before = model.cobol.text();
    let r = editmenu::apply(&mut model.cobol, action, &mut model.clipboard);
    if r.touched() {
        model.cobol.ensure_caret_visible(EDITOR_VISIBLE_LINES);
    }
    if r.changed() {
        let after = model.cobol.text();
        if after != before {
            model.dirty = true;
            model = recompute(model);
        }
    }
    model
}

// ── Composición de la vista ───────────────────────────────────────────────

fn header_view(model: &Model, theme: &Theme) -> View<Msg> {
    let open_label = match model.open.and_then(|i| model.entries.get(i)) {
        Some(e) => {
            let marker = if model.dirty { "● " } else { "  " };
            format!("{}{}", marker, e.label)
        }
        None => "(sin archivo)".to_string(),
    };
    let label = format!(
        "chaka · COBOL → Rust · {} programas · {}",
        model.entries.len(),
        open_label,
    );

    let btn_pal = ButtonPalette::from_theme(theme);
    let btn_style = Style {
        size: Size {
            width: length(96.0_f32),
            height: length(26.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    };
    let btn_run = button_styled::<Msg>(
        "▶ Correr",
        btn_style.clone(),
        Alignment::Center,
        &btn_pal,
        Msg::Run,
    );
    let btn_save = button_styled::<Msg>(
        "💾 Guardar",
        btn_style,
        Alignment::Center,
        &btn_pal,
        Msg::Save,
    );
    let switcher = theme_switcher_view::<Msg>(theme, Msg::CycleTheme);

    let actions = vec![btn_run, btn_save, switcher];
    app_header::<Msg>(label, actions, &AppHeaderPalette::from_theme(theme))
}

fn body_view(model: &Model, theme: &Theme) -> View<Msg> {
    let tree = corpus_tree(model, theme);
    let editor = cobol_editor(model, theme);
    let outputs = output_tabs(model, theme);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![tree, editor, outputs])
}

fn corpus_tree(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = TreePalette::from_theme(theme);
    let rows: Vec<TreeRow<Msg>> = model
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| TreeRow {
            label: e.label.clone(),
            depth: 0,
            has_children: false,
            expanded: false,
            selected: model.open == Some(i),
            on_toggle: Msg::OpenFile(i),
            on_select: Msg::OpenFile(i),
            icon: None,
            on_context: None,
            editor: None,
        })
        .collect();

    let tree = if rows.is_empty() {
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(12.0_f32),
                bottom: length(12.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .text_aligned(
            "corpus vacío",
            12.0,
            theme.fg_muted,
            Alignment::Start,
        )
    } else {
        tree_view::<Msg>(TreeSpec {
            rows,
            row_height: TREE_ROW_H,
            indent_px: TREE_INDENT,
            palette,
            guides: false,
        })
    };

    // Wrapper con encabezado "corpus".
    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(8.0_f32),
            top: length(4.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned("CORPUS", 10.0, theme.fg_muted, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(TREE_WIDTH),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .clip(true)
    .children(vec![header, tree])
}

fn cobol_editor(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = EditorPalette::from_theme(theme);
    let metrics = EditorMetrics::for_font_size(13.0);
    let pointer_handler = |ev: PointerEvent| Some(Msg::EditorPointer(ev));
    let editor = text_editor_view_highlighted::<Msg>(
        &model.cobol,
        &palette,
        metrics,
        EDITOR_VISIBLE_LINES,
        Language::Plain,
        pointer_handler,
    );

    // Header con el path (path completo si no hay archivo).
    let path_label = match model.open.and_then(|i| model.entries.get(i)) {
        Some(e) => e.path.display().to_string(),
        None => "(seleccioná un programa del corpus)".to_string(),
    };
    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(4.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(path_label, 10.0, theme.fg_muted, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        flex_shrink: 1.0,
        min_size: Size {
            width: length(200.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .clip(true)
    .children(vec![header, editor])
}

fn output_tabs(model: &Model, theme: &Theme) -> View<Msg> {
    let labels = vec![
        format!("Salida ({})", salida_short(&model.pipe)),
        "Rust".to_string(),
        "IR".to_string(),
        format!("Diag ({})", diag_short(&model.pipe)),
    ];
    let active = model.active_tab.index();
    let palette = TabsPalette::from_theme(theme);

    let content = match model.active_tab {
        OutputTab::Salida => salida_pane(model, theme),
        OutputTab::Rust => viewer_pane(&model.view_rust, theme, Language::Rust),
        OutputTab::Ir => viewer_pane(&model.view_ir, theme, Language::Plain),
        OutputTab::Diag => viewer_pane(&model.view_diag, theme, Language::Plain),
    };

    let tabs = tabs_view::<Msg, _>(TabsSpec {
        labels,
        active,
        on_select: |i: usize| Msg::SelectTab(OutputTab::from_index(i)),
        content,
        tab_height: TAB_STRIP_H,
        palette,
        tab_width: None,
    });

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(0.42_f32),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        min_size: Size {
            width: length(360.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .clip(true)
    .children(vec![tabs])
}

fn salida_pane(model: &Model, theme: &Theme) -> View<Msg> {
    let banner = match model.pipe.summary {
        PipelineSummary::Idle => banner_view::<Msg>(
            BannerKind::Info,
            "abrí un programa del corpus a la izquierda".to_string(),
        ),
        PipelineSummary::Ok { lines, match_ok } => match (match_ok, &model.pipe.compare) {
            (Some(true), _) => banner_view::<Msg>(
                BannerKind::Success,
                format!("✓ shadow OK · {lines} líneas · coincide con .expected"),
            ),
            (Some(false), Some((_, _, ref msg))) => banner_view::<Msg>(
                BannerKind::Error,
                format!("✗ shadow ≠ .expected — {msg}"),
            ),
            _ => status_pill(
                ACCENT_OK,
                format!("shadow ▸ {lines} líneas · halt: Normal"),
                theme,
            ),
        },
        PipelineSummary::StepLimit => status_pill(
            ACCENT_WARN,
            "shadow ⚠ se agotó el tope de pasos (¿bucle sin fin?)".to_string(),
            theme,
        ),
        PipelineSummary::PipelineError => banner_view::<Msg>(
            BannerKind::Error,
            "el pipeline falló — ver tab «Diag» para detalles".to_string(),
        ),
    };

    let viewer = viewer_pane(&model.view_salida, theme, Language::Plain);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![banner, viewer])
}

fn viewer_pane(state: &EditorState, theme: &Theme, lang: Language) -> View<Msg> {
    let palette = EditorPalette::from_theme(theme);
    let metrics = EditorMetrics::for_font_size(12.0);
    // Pointer handler que no produce mensajes — los viewers son read-only.
    let no_pointer = |_ev: PointerEvent| None::<Msg>;
    let editor = text_editor_view_highlighted::<Msg>(
        state,
        &palette,
        metrics,
        EDITOR_VISIBLE_LINES,
        lang,
        no_pointer,
    );
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .clip(true)
    .children(vec![editor])
}

/// Una "píldora" de status — fondo `accent` translúcido + texto a la
/// izquierda. Sustituto del `banner` cuando no es success/error/info
/// canónico (intermedio: shadow corrió, sin .expected para comparar).
fn status_pill(accent: Color, text: String, theme: &Theme) -> View<Msg> {
    let bg = with_alpha(accent, 0x22);
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .text_aligned(text, 11.0, theme.fg_text, Alignment::Start)
}

fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(STATUS_H),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(
        model.status.clone(),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    )
}

// ── Mutaciones de estado ──────────────────────────────────────────────────

fn open_entry(mut model: Model, i: usize) -> Model {
    let Some(entry) = model.entries.get(i) else {
        return model;
    };
    let path = entry.path.clone();
    match fs::read_to_string(&path) {
        Ok(text) => {
            if text.len() > MAX_SOURCE_BYTES {
                model.status = format!(
                    "✗ {} pesa {} B — tope: {} B",
                    path.display(),
                    text.len(),
                    MAX_SOURCE_BYTES,
                );
                return model;
            }
            model.cobol = EditorState::new();
            model.cobol.set_text(&text);
            model.open = Some(i);
            model.dirty = false;
            model.drag_accum = (0.0, 0.0);
            model.status = format!("abierto {}", path.display());
            model = recompute(model);
        }
        Err(e) => {
            model.status = format!("✗ no se pudo leer {}: {}", path.display(), e);
        }
    }
    model
}

fn apply_edit_key(mut model: Model, ev: KeyEvent) -> Model {
    let before = model.cobol.text();
    let _result = model.cobol.apply_key(&ev);
    model.cobol.ensure_caret_visible(EDITOR_VISIBLE_LINES);
    let after = model.cobol.text();
    if after != before {
        model.dirty = true;
        model = recompute(model);
    }
    model
}

fn apply_editor_pointer(mut model: Model, ev: PointerEvent) -> Model {
    let metrics = EditorMetrics::for_font_size(13.0);
    let scroll = model.cobol.scroll_offset;
    match ev {
        PointerEvent::Click { x, y } => {
            let (line, col) = metrics.screen_to_pos(x, y, scroll);
            model.cobol.set_caret_at(line, col);
            model.drag_accum = (0.0, 0.0);
        }
        PointerEvent::Drag {
            initial_x,
            initial_y,
            dx,
            dy,
        } => {
            model.drag_accum.0 += dx;
            model.drag_accum.1 += dy;
            let x = initial_x + model.drag_accum.0;
            let y = initial_y + model.drag_accum.1;
            let (line, col) = metrics.screen_to_pos(x, y, scroll);
            model.cobol.extend_selection_to(line, col);
        }
    }
    model
}

fn save_open(mut model: Model) -> Model {
    let Some(i) = model.open else {
        model.status = "no hay archivo abierto para guardar".into();
        return model;
    };
    let Some(entry) = model.entries.get(i) else {
        return model;
    };
    let content = model.cobol.text();
    match fs::write(&entry.path, &content) {
        Ok(()) => {
            model.dirty = false;
            model.status = format!("guardado {}", entry.path.display());
        }
        Err(e) => {
            model.status = format!("✗ no se pudo guardar: {e}");
        }
    }
    model
}

/// Re-ejecuta el pipeline sobre el buffer actual y refresca los viewers.
fn recompute(mut model: Model) -> Model {
    let source = model.cobol.text();
    model.pipe = run_pipeline(&source, model.open.and_then(|i| model.entries.get(i)));
    model.view_salida = read_only_state(&model.pipe.salida_text());
    model.view_rust = read_only_state(&model.pipe.rust);
    model.view_ir = read_only_state(&model.pipe.ir_json);
    model.view_diag = read_only_state(&model.pipe.diagnostics);
    model
}

fn read_only_state(text: &str) -> EditorState {
    let mut s = EditorState::new();
    s.set_text(text);
    s
}

// ── Pipeline (lex → parse → ir → codegen → shadow) ────────────────────────

impl Pipeline {
    fn idle() -> Self {
        Self {
            outcome: None,
            compare: None,
            rust: String::new(),
            ir_json: String::new(),
            diagnostics: String::new(),
            summary: PipelineSummary::Idle,
        }
    }

    fn salida_text(&self) -> String {
        match &self.outcome {
            None => String::from("(sin salida — el pipeline falló antes de ejecutar)\n"),
            Some(out) => {
                let mut buf = String::new();
                for line in &out.lines {
                    buf.push_str(line);
                    buf.push('\n');
                }
                if let Some((ok, total, msg)) = &self.compare {
                    buf.push_str("\n— vs .expected ");
                    if *ok {
                        buf.push_str(&format!("✓ {total} líneas coinciden\n"));
                    } else {
                        buf.push_str(&format!("✗\n{msg}\n"));
                    }
                }
                buf
            }
        }
    }
}

fn run_pipeline(source: &str, entry: Option<&CorpusEntry>) -> Pipeline {
    let mut diag = String::new();
    let tokens = match lex(source, SourceFormat::Free) {
        Ok(t) => t,
        Err(e) => {
            diag.push_str(&format!("error de léxico:\n  {e}\n"));
            return Pipeline {
                outcome: None,
                compare: None,
                rust: String::new(),
                ir_json: String::new(),
                diagnostics: diag,
                summary: PipelineSummary::PipelineError,
            };
        }
    };
    let program = match parse(&tokens) {
        Ok(p) => p,
        Err(e) => {
            diag.push_str(&format!("error de parseo:\n  {e}\n"));
            return Pipeline {
                outcome: None,
                compare: None,
                rust: String::new(),
                ir_json: String::new(),
                diagnostics: diag,
                summary: PipelineSummary::PipelineError,
            };
        }
    };
    let ir = chaka_ir::lower(&program);
    let rust = chaka_codegen::emit(&ir, Target::Rust);
    let ir_json = chaka_codegen::emit(&ir, Target::Json);

    // Verbos no transpilados — los recogemos como aviso, no como error.
    let unknowns = collect_unknowns(&ir);
    if !unknowns.is_empty() {
        diag.push_str("verbos no transpilados:\n");
        for v in &unknowns {
            diag.push_str(&format!("  · {v}\n"));
        }
    }

    let outcome = interpret(&ir);
    let halt = outcome.halt;
    let n = outcome.lines.len();

    let compare = entry.and_then(|e| {
        let expected_path = e.path.with_extension("expected");
        let got: Vec<String> = outcome
            .lines
            .iter()
            .map(|l| l.trim_end().to_string())
            .collect();
        match fs::read_to_string(&expected_path) {
            Ok(want_raw) => {
                let want: Vec<String> = want_raw.lines().map(|l| l.trim_end().to_string()).collect();
                if got == want {
                    Some((true, got.len(), String::new()))
                } else {
                    Some((false, got.len().max(want.len()), diff_text(&got, &want)))
                }
            }
            Err(_) => None,
        }
    });

    let summary = match (halt, compare.as_ref()) {
        (Halt::StepLimit, _) => PipelineSummary::StepLimit,
        (_, Some((ok, _, _))) => PipelineSummary::Ok {
            lines: n,
            match_ok: Some(*ok),
        },
        (_, None) => PipelineSummary::Ok {
            lines: n,
            match_ok: None,
        },
    };
    if diag.is_empty() {
        diag.push_str("(sin errores · sin verbos desconocidos)\n");
    }
    Pipeline {
        outcome: Some(outcome),
        compare,
        rust,
        ir_json,
        diagnostics: diag,
        summary,
    }
}

/// Texto del diff: línea por línea de divergencia, formato "obtenido /
/// esperado". Sólo se incluye la línea donde difieren.
fn diff_text(got: &[String], want: &[String]) -> String {
    let mut buf = String::new();
    let n = got.len().max(want.len());
    for i in 0..n {
        let g = got.get(i).map(|s| s.as_str()).unwrap_or("<falta>");
        let w = want.get(i).map(|s| s.as_str()).unwrap_or("<falta>");
        if g != w {
            buf.push_str(&format!("  línea {}:\n", i + 1));
            buf.push_str(&format!("    obtenido: {g}\n"));
            buf.push_str(&format!("    esperado: {w}\n"));
        }
    }
    buf
}

fn collect_unknowns(ir: &Ir) -> Vec<String> {
    let mut verbs = Vec::new();
    for proc in &ir.procedures {
        walk_stmts(&proc.body, &mut verbs);
    }
    verbs.sort();
    verbs.dedup();
    verbs
}

fn walk_stmts(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        match s {
            Stmt::Unknown { verb, .. } => out.push(verb.clone()),
            Stmt::If {
                then_branch,
                else_branch,
                ..
            } => {
                walk_stmts(then_branch, out);
                walk_stmts(else_branch, out);
            }
            Stmt::Evaluate { whens, other, .. } => {
                for w in whens {
                    walk_stmts(&w.body, out);
                }
                walk_stmts(other, out);
            }
            Stmt::Read {
                at_end, not_at_end, ..
            } => {
                walk_stmts(at_end, out);
                walk_stmts(not_at_end, out);
            }
            Stmt::Perform(p) => {
                if let PerformTarget::Inline(body) = &p.target {
                    walk_stmts(body, out);
                }
            }
            Stmt::Call {
                on_overflow,
                not_on_overflow,
                ..
            } => {
                walk_stmts(on_overflow, out);
                walk_stmts(not_on_overflow, out);
            }
            Stmt::Search { at_end, whens, .. } => {
                walk_stmts(at_end, out);
                for w in whens {
                    walk_stmts(&w.body, out);
                }
            }
            Stmt::Rewrite {
                invalid_key,
                not_invalid_key,
                ..
            }
            | Stmt::Delete {
                invalid_key,
                not_invalid_key,
                ..
            }
            | Stmt::Start {
                invalid_key,
                not_invalid_key,
                ..
            } => {
                walk_stmts(invalid_key, out);
                walk_stmts(not_invalid_key, out);
            }
            _ => {}
        }
    }
}

// ── Resumen para títulos de tab ───────────────────────────────────────────

fn salida_short(pipe: &Pipeline) -> String {
    match pipe.summary {
        PipelineSummary::Idle => "—".to_string(),
        PipelineSummary::Ok { lines, match_ok } => match match_ok {
            Some(true) => format!("{lines}✓"),
            Some(false) => format!("{lines}✗"),
            None => format!("{lines}"),
        },
        PipelineSummary::StepLimit => "⚠".to_string(),
        PipelineSummary::PipelineError => "✗".to_string(),
    }
}

fn diag_short(pipe: &Pipeline) -> String {
    match pipe.summary {
        PipelineSummary::PipelineError => "✗".to_string(),
        _ => {
            // Cuenta líneas no vacías del bloque de diagnóstico, descontando
            // la línea de encabezado "verbos no transpilados:".
            let count = pipe
                .diagnostics
                .lines()
                .filter(|l| l.trim_start().starts_with('·') || l.trim_start().starts_with('-'))
                .count();
            if count == 0 {
                "0".to_string()
            } else {
                format!("{count}")
            }
        }
    }
}

// ── Carga del corpus ──────────────────────────────────────────────────────

/// Resuelve el directorio del corpus. Orden de preferencia:
/// 1) primer argumento posicional (cualquier dir con `.cob` adentro);
/// 2) `corpus/` al lado del binario (caso `cargo run --release`);
/// 3) `00_unanchay/chaka/corpus` desde el CWD (caso `cargo run` en repo);
/// 4) `corpus/` en el CWD;
/// 5) CWD.
fn locate_corpus() -> PathBuf {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Some(arg) = args.iter().find(|a| !a.starts_with("--")) {
        let p = PathBuf::from(arg);
        if p.is_dir() {
            return p;
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let here_corpus = manifest.parent().map(|p| p.join("corpus"));
    if let Some(c) = here_corpus.as_ref() {
        if c.is_dir() {
            return c.clone();
        }
    }
    let repo_corpus = PathBuf::from("00_unanchay/chaka/corpus");
    if repo_corpus.is_dir() {
        return repo_corpus;
    }
    let cwd_corpus = PathBuf::from("corpus");
    if cwd_corpus.is_dir() {
        return cwd_corpus;
    }
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn scan_corpus(root: &Path) -> Vec<CorpusEntry> {
    let mut entries: Vec<CorpusEntry> = match fs::read_dir(root) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|d| {
                let path = d.path();
                let ext = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_ascii_lowercase());
                if matches!(ext.as_deref(), Some("cob") | Some("cbl")) {
                    let label = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("?")
                        .to_string();
                    Some(CorpusEntry { label, path })
                } else {
                    None
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    entries.sort_by(|a, b| a.label.cmp(&b.label));
    entries
}

// ── Color util ────────────────────────────────────────────────────────────

fn with_alpha(c: Color, alpha: u8) -> Color {
    let [r, g, b, _] = c.to_rgba8().to_u8_array();
    Color::from_rgba8(r, g, b, alpha)
}

fn main() {
    llimphi_ui::run::<ChakaApp>();
}

