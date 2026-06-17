//! **Showreel** de nakui — el shell unificado ERP / Hoja de cálculo / Grafo,
//! para el README del standalone. NO es eye-candy abstracto: cada frame monta
//! la **view real** del shell (`chrome::body` + toolbar + menubar, exactamente
//! los builders que pinta la app) con el `Model` real sembrado por el mismo
//! camino que el `init()` de la app, y deriva su **estado** del tiempo
//! normalizado `t∈[0,1]`:
//!
//!   1. cold-open: trazo bezier draw-on (firma).
//!   2. el área **Hoja** se llena celda a celda (la factura demo viva, con
//!      fórmulas reales `=B2*C2`, `=SUM(...)`), la selección recorre las
//!      celdas y la barra de fórmula refleja el `raw` de la activa.
//!   3. conmuta al área **ERP** (tablero meta-driven: stat cards + gráficos)
//!      vía el conmutador real de la toolbar.
//!   4. conmuta al área **Grafo** (DAG de morfismos del módulo activo).
//!   5. cierre: wordmark «nakui» + subtítulo, frame limpio para screenshot.
//!
//! Render headless y determinista (sin reloj, sin runtime, sin winit): frame
//! `i` de `N` → `t = i/(N-1)` → Model(t) → view → layout (taffy + parley) →
//! vello::Scene → wgpu → PNG. Idéntico al eventloop.
//!
//! ```text
//! cargo run -p nakui-ui-llimphi --example nakui_showreel --release -- \
//!     [out_dir] [n_frames] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames_nakui`, `n_frames=300`, `W=1600`, `H=900`.
#![allow(dead_code)]
#![allow(unused_imports)]

// La app es un crate binario sin lib: incluimos sus módulos reales por
// `#[path]` para llamar exactamente los mismos builders que pinta la app.
#[path = "../src/backend.rs"]
mod backend;
#[path = "../src/camera.rs"]
mod camera;
#[path = "../src/charts.rs"]
mod charts;
#[path = "../src/export.rs"]
mod export;
#[path = "../src/form.rs"]
mod form;
#[path = "../src/io.rs"]
mod io;
#[path = "../src/layout.rs"]
mod layout;
#[path = "../src/panels.rs"]
mod panels;
#[path = "../src/tablero.rs"]
mod tablero;
#[path = "../src/widgets.rs"]
mod widgets;
#[path = "../src/chrome.rs"]
mod chrome;
#[path = "../src/caja.rs"]
mod caja;
#[path = "../src/hoja.rs"]
mod hoja;

use chrome::{Area, DockPanel};
use form::*;
use hoja::SheetView;
use io::*;
use layout::*;

// ---------------------------------------------------------------------------
// Raíz del crate calcada de src/main.rs (imports, consts, Msg, Model y sus
// structs): los submódulos la consumen vía `use super::*`, así que tiene que
// existir idéntica acá. Sin el `impl App` (no hay eventloop en el showreel).
// ---------------------------------------------------------------------------

use crate::charts::*;
use crate::export::*;
use crate::panels::*;
use crate::tablero::*;
use crate::widgets::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use cards::CardBody;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Position, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle as KurboCircle, Point, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{self, Color, Fill, Gradient};
use llimphi_ui::llimphi_text::{draw_layout_brush_xf, measurement, Alignment, Typesetter};
use llimphi_ui::{
    App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, PaintRect, View,
    WheelDelta,
};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_field::{field_view, FieldPalette, FieldSpec as FieldWidgetSpec};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_motion::{animate, motion, Tween};
use llimphi_clipboard::SystemClipboard;
use llimphi_widget_nodegraph::{
    nodegraph_view_styled, NodeId, NodeSpec, NodeTint, NodegraphMetrics, NodegraphPalette, Wire,
};

use nahual_meta_runtime::{
    breakdown_to_csv, bucket_date, cmp_values, compute_clear_fields, compute_field_delta,
    compute_metric, cumulative_breakdown, format_value, human_label_for_record, limit_breakdown,
    parse_field_value,
    preview_value, record_matches, render_value, resolve_param_value, short_uuid,
    sort_breakdown_by_key, to_csv, validate_entity_refs, MetaBackend, MetricResult, WriteOutcome,
};
use nahual_meta_schema::{
    Action, CardFilter, ChartKind, Column, DashboardCard, DashboardView, DetailMetric, FieldKind,
    FieldSpec, FormView, GraphView, ListView, Module, RelatedList, ReportView, ValueFormat,
    View as ModuleView,
};
use nakui_core::executor::Executor;
use nakui_sheet::{CellRef, Workbook};
use serde_json::Value;
use uuid::Uuid;

use crate::backend::{MorphismGraphData, NakuiBackend};
use crate::camera::{
    canvas_rect_get, dentro_de_rect, fit_to_view, pan_para_zoom_a_cursor, ZOOM_BASE, ZOOM_MAX,
    ZOOM_MIN,
};

const SIDEBAR_WIDTH: f32 = 240.0;
const ROW_HEIGHT: f32 = 22.0;
const ENTITY_REF_LIMIT: usize = 50;
const LIST_PAGE_SIZE: usize = 20;

#[derive(Clone)]
enum Msg {
    SelectModule(usize),
    SelectMenu(usize),
    OpenForm { module_idx: usize, view_key: String },
    NewRecord { module_idx: usize, entity: String },
    EditRecord { module_idx: usize, entity: String, id: Uuid },
    DeleteRecord { entity: String, id: Uuid },
    FocusField(usize),
    FieldKey(KeyEvent),
    SetSelect(usize, String),
    ToggleBool(usize),
    SubmitForm,
    CancelForm,
    DismissToast,
    OpenDetail { module_idx: usize, view_key: String, entity: String, id: Uuid },
    CloseDetail,
    DetailEditField { field: String },
    DetailInlineKey(KeyEvent),
    DetailInlineFocus,
    DetailInlineSet(String),
    DetailInlineCommit,
    DetailInlineCancel,
    FocusListSearch,
    ListSearchKey(KeyEvent),
    SortBy(String),
    ListPagePrev,
    ListPageNext,
    ExportCsv { entity: String },
    ExportReport { module_idx: usize, view_key: String },
    ExportBreakdownCsv { module_idx: usize, view_key: String, card_idx: usize },
    ToggleReportFilter { view_key: String, idx: usize },
    DrillDown { entity: String, field: String, value: String, label: String, prefix: bool },
    ClearDrill,
    DragGraphNode { module_id: String, morphism: String, dx: f32, dy: f32, end: bool },
    SelectGraphNode { mod_idx: usize, id: NodeId },
    ZoomGraph { mult: f32, ancla: Option<(f32, f32)> },
    FitGraph,
    MenuOpen(Option<usize>),
    MenuCommand(String),
    EditMenuOpen(f32, f32),
    EditMenuAction(EditAction),
    CloseMenus,
    MenuNav(i32),
    MenuActivate,
    MenuTick,
    EditNav(i32),
    EditActivate,
    SwitchArea(Area),
    SetDockPanel(DockPanel),
    ToggleDock,
    SetDockWidth(f32),
    AreaTick,
    HojaSelectCell { col: u32, row: u32 },
    HojaMove { dcol: i32, drow: i32 },
    HojaFocusBar,
    HojaFormulaKey(KeyEvent),
    HojaEditWith(String),
    HojaEditStart,
    HojaCommit,
    HojaCancel,
    HojaClear,
    HojaUndo,
    HojaRedo,
    HojaScroll { dcol: i32, drow: i32 },
    HojaExportCsv,
    CajaAddProduct { id: Uuid, name: String, price: f64 },
    CajaInc(usize),
    CajaDec(usize),
    CajaClear,
    CajaCharge,
    CajaSetMethod(String),
}

struct FormState {
    module_idx: usize,
    entity: String,
    title: String,
    on_submit: Action,
    fields: Vec<FieldRuntime>,
    editing: Option<Uuid>,
    original: Option<Value>,
    focused: Option<usize>,
    error: Option<String>,
}

struct FieldRuntime {
    spec: FieldSpec,
    input: TextInputState,
}

impl FieldRuntime {
    fn raw(&self) -> String {
        self.input.text().to_string()
    }
}

struct Toast {
    kind: BannerKind,
    text: String,
}

struct DetailState {
    module_idx: usize,
    view_key: String,
    entity: String,
    id: Uuid,
}

struct Model {
    modules: Vec<Module>,
    backend: Arc<Mutex<NakuiBackend>>,
    initial_toast: Option<String>,
    load_error: Option<String>,
    selected_module: Option<usize>,
    selected_menu: Option<usize>,
    form: Option<FormState>,
    detail: Option<DetailState>,
    inline_edit: Option<FieldRuntime>,
    toast: Option<Toast>,
    list_search: TextInputState,
    list_search_focused: bool,
    list_sort: Option<(String, bool)>,
    list_page: usize,
    report_filters: BTreeSet<String>,
    drill: Option<DrillFilter>,
    graph_pos: BTreeMap<(String, String), (f32, f32)>,
    layout_path: PathBuf,
    graph_selected: Option<(usize, NodeId)>,
    graph_zoom: f32,
    graph_pan: (f32, f32),
    menu_open: Option<usize>,
    menu_active: usize,
    menu_anim: Tween<f32>,
    edit_menu: Option<(f32, f32)>,
    edit_active: usize,
    edit_anim: Tween<f32>,
    clipboard: SystemClipboard,
    area: Area,
    dock_left_active: DockPanel,
    dock_left_open: bool,
    area_anim: Tween<f32>,
    dock_w: f32,
    sheet: SheetView,
    cart: Vec<caja::CartLine>,
    caja_method: String,
}

#[derive(Clone)]
struct DrillFilter {
    entity: String,
    field: String,
    value: String,
    label: String,
    prefix: bool,
}

impl Model {
    fn reset_list_state(&mut self) {
        self.list_search.clear();
        self.list_search_focused = false;
        self.list_sort = None;
        self.list_page = 0;
    }

    fn focused_input(&self) -> Option<&TextInputState> {
        if let Some(fr) = &self.inline_edit {
            if is_text_field(fr.spec.kind) {
                return Some(&fr.input);
            }
        }
        if let Some(form) = &self.form {
            if let Some(i) = form.focused {
                return form.fields.get(i).map(|f| &f.input);
            }
        }
        if self.list_search_focused {
            return Some(&self.list_search);
        }
        None
    }

    fn focused_input_mut(&mut self) -> Option<&mut TextInputState> {
        if let Some(fr) = &mut self.inline_edit {
            if is_text_field(fr.spec.kind) {
                return Some(&mut fr.input);
            }
        }
        if let Some(form) = &mut self.form {
            if let Some(i) = form.focused {
                return form.fields.get_mut(i).map(|f| &mut f.input);
            }
        }
        if self.list_search_focused {
            return Some(&mut self.list_search);
        }
        None
    }
}

// --- Helpers raíz reales (menú principal + spec del menubar), calcados. ---

fn edit_flags(model: &Model) -> EditFlags {
    match model.focused_input() {
        Some(input) => EditFlags::from_editor(input.editor(), input.is_masked()),
        None => EditFlags::default(),
    }
}

fn menubar_spec<'a>(
    menu: &'a app_bus::AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
    let (w, h) = (W, H);
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

fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};

    let input = model.focused_input();
    let has_focus = input.is_some();
    let has_sel = input.map(|i| i.editor().has_selection()).unwrap_or(false);
    let can_undo = input.map(|i| i.editor().can_undo()).unwrap_or(false);
    let can_redo = input.map(|i| i.editor().can_redo()).unwrap_or(false);

    let mut undo = MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z");
    if !can_undo {
        undo = undo.disabled();
    }
    let mut redo = MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y");
    if !can_redo {
        redo = redo.disabled();
    }
    let mut cut = MenuItem::new("Cortar", "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new("Copiar", "edit.copy").shortcut("Ctrl+C");
    if !has_sel {
        cut = cut.disabled();
        copy = copy.disabled();
    }
    let mut paste = MenuItem::new("Pegar", "edit.paste").shortcut("Ctrl+V");
    let mut sel_all = MenuItem::new("Seleccionar todo", "edit.selectall")
        .shortcut("Ctrl+A")
        .separated();
    if !has_focus {
        paste = paste.disabled();
        sel_all = sel_all.disabled();
    }

    let active = active_view_info(model);
    let mut nuevo = MenuItem::new("Nuevo record", "file.new");
    if active.as_ref().and_then(|v| v.entity.as_ref()).is_none() {
        nuevo = nuevo.disabled();
    }
    let mut export_csv = MenuItem::new("Exportar lista (CSV)", "file.export_csv");
    if !active.as_ref().map(|v| v.is_list).unwrap_or(false) {
        export_csv = export_csv.disabled();
    }
    let mut export_md = MenuItem::new("Exportar reporte (.md)", "file.export_md").separated();
    if !active.as_ref().map(|v| v.is_report).unwrap_or(false) {
        export_md = export_md.disabled();
    }

    let mut clear_drill = MenuItem::new("Limpiar filtro drill-down", "view.clear_drill");
    if model.drill.is_none() {
        clear_drill = clear_drill.disabled();
    }
    let is_graph = active_graph_module(model).is_some();
    let mut fit = MenuItem::new("Ajustar grafo a la vista", "view.fit_graph");
    let mut zoom_in = MenuItem::new("Acercar grafo", "view.zoom_in");
    let mut zoom_out = MenuItem::new("Alejar grafo", "view.zoom_out");
    if !is_graph {
        fit = fit.disabled();
        zoom_in = zoom_in.disabled();
        zoom_out = zoom_out.disabled();
    }

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(nuevo)
                .item(export_csv)
                .item(export_md)
                .item(MenuItem::new("Cancelar formulario", "file.cancel_form")),
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
            Menu::new("Ver")
                .item(clear_drill)
                .item(fit)
                .item(zoom_in)
                .item(zoom_out),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de Nakui", "help.about")))
}

struct ActiveViewInfo {
    entity: Option<String>,
    is_list: bool,
    is_report: bool,
}

fn active_view_info(model: &Model) -> Option<ActiveViewInfo> {
    let mod_idx = model.selected_module?;
    let module = model.modules.get(mod_idx)?;
    let menu_idx = model.selected_menu?;
    let item = module.menu.get(menu_idx)?;
    match module.views.get(&item.view) {
        Some(ModuleView::List(lv)) => Some(ActiveViewInfo {
            entity: Some(lv.entity.clone()),
            is_list: true,
            is_report: false,
        }),
        Some(ModuleView::Report(_)) => Some(ActiveViewInfo {
            entity: None,
            is_list: false,
            is_report: true,
        }),
        Some(ModuleView::Form(fv)) => Some(ActiveViewInfo {
            entity: Some(fv.entity.clone()),
            is_list: false,
            is_report: false,
        }),
        _ => Some(ActiveViewInfo {
            entity: None,
            is_list: false,
            is_report: false,
        }),
    }
}

fn menu_command_to_msg(model: &Model, command: &str) -> Option<Msg> {
    let mod_idx = model.selected_module?;
    let view_key = model
        .selected_module
        .and_then(|i| model.modules.get(i))
        .and_then(|m| model.selected_menu.map(|j| (m, j)))
        .and_then(|(m, j)| m.menu.get(j))
        .map(|item| item.view.clone());
    match command {
        "edit.undo" => Some(Msg::EditMenuAction(EditAction::Undo)),
        "edit.redo" => Some(Msg::EditMenuAction(EditAction::Redo)),
        "edit.cut" => Some(Msg::EditMenuAction(EditAction::Cut)),
        "edit.copy" => Some(Msg::EditMenuAction(EditAction::Copy)),
        "edit.paste" => Some(Msg::EditMenuAction(EditAction::Paste)),
        "edit.selectall" => Some(Msg::EditMenuAction(EditAction::SelectAll)),
        "file.new" => active_view_info(model)
            .and_then(|v| v.entity)
            .map(|entity| Msg::NewRecord { module_idx: mod_idx, entity }),
        "file.export_csv" => active_view_info(model)
            .and_then(|v| v.entity)
            .map(|entity| Msg::ExportCsv { entity }),
        "file.export_md" => view_key.map(|view_key| Msg::ExportReport { module_idx: mod_idx, view_key }),
        "file.cancel_form" => Some(Msg::CancelForm),
        "view.clear_drill" => Some(Msg::ClearDrill),
        "view.fit_graph" => Some(Msg::FitGraph),
        "view.zoom_in" => Some(Msg::ZoomGraph { mult: ZOOM_BASE, ancla: None }),
        "view.zoom_out" => Some(Msg::ZoomGraph { mult: 1.0 / ZOOM_BASE, ancla: None }),
        _ => None,
    }
}

fn active_view_key(model: &Model) -> Option<String> {
    let module = model.modules.get(model.selected_module?)?;
    let item = module.menu.get(model.selected_menu?)?;
    Some(item.view.clone())
}

// ---------------------------------------------------------------------------
// Construcción del Model real (camino del init de la app) + vista real.
// ---------------------------------------------------------------------------

use std::fs::{create_dir_all, File};
use std::io::BufWriter;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::{measure_text_node, mount, paint};

const W: u32 = 1600;
const H: u32 = 900;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Lista de celdas sembradas de la factura demo, en el orden en que se van
/// "llenando" durante el beat de la Hoja (header → filas → total). Replica el
/// `seed()` real de `hoja.rs` pero exponiéndolo como secuencia animable.
const FACTURA: &[(&str, &str)] = &[
    ("A1", "Concepto"), ("B1", "Cant"), ("C1", "Unit"), ("D1", "Subtotal"), ("E1", "IVA"),
    ("A2", "Café"),     ("B2", "5"),    ("C2", "20"),   ("D2", "=B2*C2"),   ("E2", "=D2*16%"),
    ("A3", "Té"),       ("B3", "3"),    ("C3", "15"),   ("D3", "=B3*C3"),   ("E3", "=D3*16%"),
    ("A4", "Azúcar"),   ("B4", "2"),    ("C4", "10"),   ("D4", "=B4*C4"),   ("E4", "=D4*16%"),
    ("A6", "TOTAL"),    ("D6", "=SUM(D2:D4)"), ("E6", "=SUM(E2:E4)"),
];

/// Construye el `Model` real: los módulos demo del crate cargados por
/// `load_ui_modules`, executors Rhai, backend con event log efímero y la
/// siembra del `seed.json` de cada módulo — el mismo camino que el `init()`
/// de la app. Queda activo el módulo **Tesorería** con su **Tablero**.
fn modelo_demo() -> Model {
    let modules_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/nakui-modules");
    let (modules, _skipped) = load_ui_modules(&modules_dir).expect("módulos demo del crate");

    let mut executors: BTreeMap<String, Arc<Executor>> = BTreeMap::new();
    for m in &modules {
        if let Some(rel) = &m.nakui_module_dir {
            let nakui_dir = modules_dir.join(&m.id).join(rel);
            match Executor::load_module(&nakui_dir) {
                Ok(exec) => {
                    executors.insert(m.id.clone(), Arc::new(exec));
                }
                Err(e) => eprintln!("nakui_showreel: executor de {}: {e}", m.id),
            }
        }
    }

    let state_dir = std::env::temp_dir().join("nakui-showreel");
    let _ = std::fs::remove_dir_all(&state_dir);
    std::fs::create_dir_all(&state_dir).expect("dir de estado temporal");
    let log_path = state_dir.join("nakui-showreel.jsonl");
    let layout_path = log_path.with_extension("layout.json");
    let (mut backend, _status) = NakuiBackend::open(log_path, 50, executors);

    // Sembramos los datos demo (el camino real) pero descartamos el toast
    // informativo: el showreel arranca con el chrome limpio, sin banner.
    let _ = seed_demo_data(&mut backend, &modules, &modules_dir);
    let initial_toast: Option<String> = None;

    let selected_module = modules
        .iter()
        .position(|m| m.id == "tesoro")
        .or_else(|| (!modules.is_empty()).then_some(0));
    let selected_menu = selected_module.and_then(|i| {
        let m = &modules[i];
        m.menu
            .iter()
            .position(|it| matches!(m.views.get(&it.view), Some(ModuleView::Dashboard(_))))
            .or_else(|| (!m.menu.is_empty()).then_some(0))
    });

    Model {
        modules,
        backend: Arc::new(Mutex::new(backend)),
        initial_toast,
        load_error: None,
        selected_module,
        selected_menu,
        form: None,
        detail: None,
        inline_edit: None,
        // Sin toast: el showreel arranca limpio.
        toast: None,
        list_search: TextInputState::new(),
        list_search_focused: false,
        list_sort: None,
        list_page: 0,
        report_filters: BTreeSet::new(),
        drill: None,
        graph_pos: BTreeMap::new(),
        layout_path,
        graph_selected: None,
        graph_zoom: 1.0,
        graph_pan: (0.0, 0.0),
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        edit_menu: None,
        edit_active: usize::MAX,
        edit_anim: Tween::idle(1.0),
        clipboard: SystemClipboard::new(),
        area: Area::Hoja,
        dock_left_active: DockPanel::Nav,
        dock_left_open: true,
        area_anim: Tween::idle(1.0),
        dock_w: 230.0,
        sheet: SheetView::new(),
        cart: Vec::new(),
        caja_method: "efectivo".into(),
    }
}

/// Re-siembra la hoja del `Model` con sólo el prefijo `n` de las celdas de la
/// factura (efecto "llenándose"). `Workbook::new()` parte de cero; las
/// fórmulas recalculan reactivamente al setear sus dependencias.
fn seed_hoja_prefix(sheet: &mut SheetView, n: usize) {
    let mut wb = Workbook::new();
    for (cell, raw) in FACTURA.iter().take(n) {
        if let Ok(cr) = cell.parse::<CellRef>() {
            let _ = wb.set_cell(cr, raw);
        }
    }
    sheet.wb = wb;
    sheet.bar.set_text(sheet.wb.raw(sheet.sel).unwrap_or(""));
}

/// Misma composición que el `view()` real: menubar + toolbar (conmutador de
/// áreas + acciones) + banners + cuerpo (dientes + panel + área). Los mismos
/// builders reales de la app.
fn vista(model: &Model, theme: &Theme) -> View<Msg> {
    let menubar = menubar_view(&menubar_spec(&app_menu(model), model, theme));
    let toolbar = chrome::build_toolbar(model, theme);
    let banners = build_banners(model);
    let body = chrome::body(model, theme);

    let mut children: Vec<View<Msg>> = vec![menubar, toolbar];
    children.extend(banners);
    children.push(body);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}

// ───────────────────────── utilidades de timeline ─────────────────────────

fn with_alpha(c: Color, a: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, a.clamp(0.0, 1.0)])
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Reescala `t` desde el subintervalo `[lo,hi]` a `[0,1]`, clampado.
fn seg(t: f32, lo: f32, hi: f32) -> f32 {
    ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
}

#[derive(Clone)]
struct Skin {
    accent: Color,
    fg: Color,
    fg_muted: Color,
}

// ───────────────────────── la timeline: Model(t) ─────────────────────────

/// Aplica el estado animado al `Model` según `t`. Conmuta el área activa con
/// el conmutador real (mutando `model.area`), llena la Hoja celda a celda,
/// mueve la selección y refleja la barra de fórmula — todo estado real que la
/// view real consume.
fn aplicar_timeline(model: &mut Model, t: f32) {
    // Fade-in del contenido entre cambios de área: alto cuando estamos
    // asentados en un área, baja en la transición.
    let near_erp = seg(t, 0.46, 0.52);
    let near_grafo = seg(t, 0.70, 0.76);
    let trans = (near_erp * (1.0 - near_erp) + near_grafo * (1.0 - near_grafo)) * 4.0;
    model.area_anim = Tween::idle((1.0 - trans).clamp(0.35, 1.0));

    if t < 0.50 {
        // ── BEAT HOJA (≈8%–50%) ─────────────────────────────────────
        model.area = Area::Hoja;
        // Llenado celda a celda: de 0 a todas las celdas de la factura.
        let fill = seg(t, 0.10, 0.40);
        let n = (fill * FACTURA.len() as f32).round() as usize;
        seed_hoja_prefix(&mut model.sheet, n.min(FACTURA.len()));
        // La selección recorre las celdas recién llenadas (la "punta").
        let sel_cell = if n == 0 { "A1" } else { FACTURA[n.saturating_sub(1).min(FACTURA.len() - 1)].0 };
        if let Ok(cr) = sel_cell.parse::<CellRef>() {
            model.sheet.sel = cr;
            model.sheet.bar.set_text(model.sheet.wb.raw(cr).unwrap_or(""));
        }
    } else if t < 0.72 {
        // ── BEAT ERP (≈52%–72%) ─────────────────────────────────────
        // Hoja ya completa por si se vuelve; pero el área es ERP (tablero).
        seed_hoja_prefix(&mut model.sheet, FACTURA.len());
        model.area = Area::Erp;
    } else {
        // ── BEAT GRAFO (≈76%–92%) ───────────────────────────────────
        seed_hoja_prefix(&mut model.sheet, FACTURA.len());
        model.area = Area::Grafo;
        model.graph_zoom = 1.0;
        model.graph_pan = (0.0, 0.0);
    }
}

// ───────────────────────── overlays vector (cold-open + wordmark) ─────────────────────────

fn signature_path(cw: f64, ch: f64) -> BezPath {
    let cx = cw / 2.0;
    let cy = ch / 2.0;
    let mut p = BezPath::new();
    p.move_to((cx - 360.0, cy + 40.0));
    p.curve_to(
        (cx - 150.0, cy - 220.0),
        (cx + 150.0, cy + 220.0),
        (cx + 360.0, cy - 40.0),
    );
    p
}

fn trim_path(full: &BezPath, prog: f64) -> (BezPath, Point) {
    use vello::kurbo::ParamCurve;
    let prog = prog.clamp(0.0, 1.0);
    let mut cubic = None;
    let mut start = Point::ZERO;
    for el in full.elements() {
        match el {
            vello::kurbo::PathEl::MoveTo(p) => start = *p,
            vello::kurbo::PathEl::CurveTo(c1, c2, p) => {
                cubic = Some(vello::kurbo::CubicBez::new(start, *c1, *c2, *p));
            }
            _ => {}
        }
    }
    let mut out = BezPath::new();
    let mut head = start;
    if let Some(cb) = cubic {
        out.move_to(cb.p0);
        let steps = 96;
        for i in 1..=steps {
            let u = (i as f64 / steps as f64) * prog;
            let pt = cb.eval(u);
            out.line_to(pt);
            head = pt;
        }
    }
    (out, head)
}

fn draw_overlays(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64, s: &Skin) {
    // ── COLD OPEN (0–10%) ──────────────────────────────────────────
    let b1 = seg(t, 0.0, 0.10);
    let line_vis = 1.0 - seg(t, 0.10, 0.17);
    if line_vis > 0.001 {
        let path = signature_path(cw, ch);
        let draw_on = motion::ease_out_cubic(seg(t, 0.01, 0.11)) as f64;
        let (trimmed, head) = trim_path(&path, draw_on);
        let line_col = with_alpha(s.accent, 0.9 * line_vis);
        scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, line_col, None, &trimmed);
        let pop = motion::ease_out_back(b1);
        let r = (4.0 + 7.0 * pop as f64).max(0.0);
        let dot_a = (b1 * line_vis).clamp(0.0, 1.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.18 * dot_a),
            None,
            &KurboCircle::new(head, r * 3.2),
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, dot_a),
            None,
            &KurboCircle::new(head, r),
        );
    }

    // ── WORDMARK (88–100%) ─────────────────────────────────────────
    let word_in = seg(t, 0.90, 0.98);
    let word_a = motion::ease_out_cubic(word_in);
    if word_a > 0.001 {
        let size = 150.0_f32;
        let layout = ts.layout(
            "nakui", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false, 0.0, 0.0,
        );
        let m = measurement(&layout);
        let rise = lerp(24.0, 0.0, word_a as f64);
        let ox = (cw - m.width as f64) / 2.0;
        let oy = (ch - m.height as f64) / 2.0 - 18.0 + rise;
        let brush = peniko::Brush::Solid(with_alpha(s.fg, word_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));

        let sub_a = motion::ease_out_cubic(seg(t, 0.93, 1.0));
        if sub_a > 0.001 {
            let ssz = 26.0_f32;
            let sub = ts.layout(
                "ERP · spreadsheet · graph, in Rust", ssz, None, Alignment::Start, 1.0, false,
                None, 400.0, false, false, 0.0, 0.0,
            );
            let sm = measurement(&sub);
            let dot_r = 6.0;
            let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
            let sx = (cw - block_w) / 2.0;
            let sy = oy + m.height as f64 + 18.0;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(s.accent, sub_a),
                None,
                &KurboCircle::new(Point::new(sx + dot_r, sy + ssz as f64 * 0.42), dot_r as f64),
            );
            let sbrush = peniko::Brush::Solid(with_alpha(s.fg_muted, sub_a));
            draw_layout_brush_xf(
                scene,
                &sub,
                &sbrush,
                Affine::translate((sx + dot_r * 2.0 + 14.0, sy)),
            );
        }
    }

    // ── punto teal de firma (esquina inf-der) ───────
    let corner_a = seg(t, 0.04, 0.12) * (1.0 - seg(t, 0.86, 0.92));
    if corner_a > 0.001 {
        let cx = cw - 54.0;
        let cy = ch - 54.0;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.16 * corner_a),
            None,
            &KurboCircle::new(Point::new(cx, cy), 18.0),
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.9 * corner_a),
            None,
            &KurboCircle::new(Point::new(cx, cy), 6.0),
        );
    }
}

/// Árbol completo del frame: la view real del shell + overlay full-screen del
/// vector (cold-open / wordmark), con fade del shell durante el cold-open y el
/// cierre para que el vector quede solo.
fn build_view(model: &Model, theme: &Theme, t: f32, cw: f64, ch: f64, skin: &Skin) -> View<Msg> {
    // El shell aparece tras el cold-open y se desvanece antes del wordmark.
    let shell_in = motion::ease_out_cubic(seg(t, 0.07, 0.14));
    let shell_out = 1.0 - seg(t, 0.86, 0.92);
    let shell_a = (shell_in * shell_out).clamp(0.0, 1.0);

    let mut children: Vec<View<Msg>> = Vec::new();
    if shell_a > 0.001 {
        let shell = View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(0.0),
                top: length(0.0),
                right: length(0.0),
                bottom: length(0.0),
            },
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .alpha(shell_a)
        .children(vec![vista(model, theme)]);
        children.push(shell);
    }

    let overlay = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0),
            top: length(0.0),
            right: length(0.0),
            bottom: length(0.0),
        },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .paint_with({
        let skin = skin.clone();
        move |scene, ts, _rect: PaintRect| {
            draw_overlays(scene, ts, t, cw, ch, &skin);
        }
    });
    children.push(overlay);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        position: Position::Relative,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| "showreel_frames_nakui".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(W);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(H);
    create_dir_all(&out_dir).expect("mkdir out_dir");

    rimay_localize::init();
    let theme = Theme::dark();
    let skin = Skin {
        accent: theme.accent,
        fg: theme.fg_text,
        fg_muted: theme.fg_muted,
    };

    // Model real una sola vez; lo mutamos por frame con la timeline.
    let mut model = modelo_demo();

    let [br, bg, bb, _] = theme.bg_app.components;
    let base = Color::from_rgba8((br * 255.0) as u8, (bg * 255.0) as u8, (bb * 255.0) as u8, 255);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("showreel-nakui"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut ts = Typesetter::new();
    let cw = w as f64;
    let ch = h as f64;

    for i in 0..n {
        let t = if n <= 1 { 0.0 } else { i as f32 / (n as f32 - 1.0) };
        aplicar_timeline(&mut model, t);
        let root = build_view(&model, &theme, t, cw, ch, &skin);

        let mut layout_tree = LayoutTree::new();
        let mounted = mount(&mut layout_tree, root);
        let computed = {
            let tmap = &mounted.text_measures;
            layout_tree
                .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                        None => llimphi_ui::llimphi_layout::taffy::Size::ZERO,
                    }
                })
                .expect("layout")
        };
        let mut scene = vello::Scene::new();
        paint(&mut scene, &mounted, &computed, &mut ts, None, None);

        renderer
            .render_to_view(&hal, &scene, &view, w, h, base)
            .expect("render_to_view");
        let path = format!("{out_dir}/frame_{i:04}.png");
        write_png(&hal, &target, &path, w, h);
        if i % 30 == 0 || i == n - 1 {
            eprintln!("showreel-nakui: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("showreel-nakui: {n} frames en {out_dir}/ ({w}x{h})");
}

fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str, w: u32, h: u32) {
    let unpadded = (w * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * h as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        let sidx = row * padded;
        pixels.extend_from_slice(&data[sidx..sidx + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&pixels).unwrap();
}
