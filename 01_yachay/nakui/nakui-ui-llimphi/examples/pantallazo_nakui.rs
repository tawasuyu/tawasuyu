//! Pantallazo headless de `nakui-ui-llimphi` — la metainterfaz ERP de nakui.
//!
//! Monta la **view real** del shell (menubar + header + sidebar de módulos
//! + área principal) con el módulo demo de **Tesorería** activo y su vista
//! `Dashboard` ("Tablero"): stat cards (movimientos, saldo neto, ingresos,
//! egresos), flujo mensual en columnas, saldo acumulado en línea, dona de
//! movimientos por tipo y columnas multi-serie de ingresos/egresos por mes.
//! Los datos salen del `seed.json` real de cada módulo (fechas fijas →
//! pantallazo estable), sembrados sobre un event log efímero por el mismo
//! `seed_demo_data` que corre la app en su primer arranque.
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `agora-app/examples/pantallazo_agora.rs`).
//!
//! `cargo run -p nakui-ui-llimphi --example pantallazo_nakui --release -- [out.png]`
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

use form::*;
use io::*;
use layout::*;

// ---------------------------------------------------------------------------
// Raíz del crate calcada de src/main.rs (imports, consts, Msg, Model y sus
// structs): los submódulos la consumen vía `use super::*`, así que tiene que
// existir idéntica acá. Sin el `impl App` (no hay eventloop en el pantallazo).
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
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle as KurboCircle, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::Alignment;
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
use serde_json::Value;
use uuid::Uuid;

use crate::backend::{MorphismGraphData, NakuiBackend};
use crate::camera::{
    canvas_rect_get, dentro_de_rect, fit_to_view, pan_para_zoom_a_cursor, ZOOM_BASE, ZOOM_MAX,
    ZOOM_MIN,
};

const SIDEBAR_WIDTH: f32 = 240.0;
const ROW_HEIGHT: f32 = 22.0;
/// Tope de records ofrecidos en un selector `EntityRef` (evita pintar
/// miles de botones). Si la entity tiene más, se avisa al usuario.
const ENTITY_REF_LIMIT: usize = 50;
/// Filas por página en las listas.
const LIST_PAGE_SIZE: usize = 20;
#[derive(Clone)]
enum Msg {
    SelectModule(usize),
    SelectMenu(usize),
    /// Abre un form fresco para la vista `view_key` del módulo.
    OpenForm {
        module_idx: usize,
        view_key: String,
    },
    /// `+ Nuevo` desde una lista: busca el Form view de la entity.
    NewRecord {
        module_idx: usize,
        entity: String,
    },
    /// Editar una fila: abre el Form view pre-rellenado con el record.
    EditRecord {
        module_idx: usize,
        entity: String,
        id: Uuid,
    },
    DeleteRecord {
        entity: String,
        id: Uuid,
    },
    /// Foco a un field de texto (text/multiline/number/date).
    FocusField(usize),
    /// Tecla ruteada al field con foco.
    FieldKey(KeyEvent),
    /// Elección de un `Select` o `EntityRef` (guarda el value crudo).
    SetSelect(usize, String),
    /// Toggle de un `Boolean`.
    ToggleBool(usize),
    SubmitForm,
    CancelForm,
    DismissToast,
    /// Abre la ficha de detalle de un record (desde el 👁 de una fila).
    OpenDetail {
        module_idx: usize,
        view_key: String,
        entity: String,
        id: Uuid,
    },
    CloseDetail,
    /// Edición in-situ: click en el valor de un campo de la ficha de
    /// detalle abre el editor en el lugar (sin form aparte). `field` es
    /// el nombre del campo (== `Column.field` == `FieldSpec.name`).
    DetailEditField {
        field: String,
    },
    /// Tecla ruteada al campo en edición in-situ (kinds de texto).
    DetailInlineKey(KeyEvent),
    /// Click en el editor in-situ (mantiene el foco; no-op).
    DetailInlineFocus,
    /// Setea el value crudo del campo in-situ (chips de select/ref/bool).
    DetailInlineSet(String),
    /// Confirma la edición in-situ: persiste sólo ese campo vía `update`.
    DetailInlineCommit,
    /// Descarta la edición in-situ.
    DetailInlineCancel,
    /// Foco a la caja de búsqueda de la lista activa.
    FocusListSearch,
    /// Tecla ruteada a la caja de búsqueda.
    ListSearchKey(KeyEvent),
    /// Click en un header de columna: cicla orden asc → desc → sin.
    SortBy(String),
    /// Paginación de la lista activa.
    ListPagePrev,
    ListPageNext,
    /// Exporta la lista activa (filas filtradas/ordenadas) a un CSV.
    ExportCsv {
        entity: String,
    },
    /// Exporta un reporte (`View::Report`) completo a Markdown.
    ExportReport {
        module_idx: usize,
        view_key: String,
    },
    /// Exporta el desglose de una card (tablero o reporte) a CSV.
    ExportBreakdownCsv {
        module_idx: usize,
        view_key: String,
        card_idx: usize,
    },
    /// Prende/apaga un toggle de filtro de un reporte.
    ToggleReportFilter {
        view_key: String,
        idx: usize,
    },
    /// Drill-down: navega a la lista de `entity` filtrada a `field ==
    /// value` (o `field` empieza con `value` si `prefix` — buckets de
    /// fecha). Click en una fila de un desglose.
    DrillDown {
        entity: String,
        field: String,
        value: String,
        label: String,
        prefix: bool,
    },
    /// Limpia el filtro de drill-down activo.
    ClearDrill,
    /// Arrastre de un nodo en la vista grafo: integra el delta del cursor
    /// sobre la posición acumulada del morfismo. La clave es estable
    /// (`module_id` + nombre del morfismo) para que la posición sobreviva
    /// reordenamientos y reinicios; `end` marca el fin del arrastre (se
    /// persiste el layout al soltar).
    DragGraphNode {
        module_id: String,
        morphism: String,
        dx: f32,
        dy: f32,
        end: bool,
    },
    /// Click-derecho sobre un morfismo en la vista grafo: selecciona/
    /// deselecciona para resaltar su cono de dependencias.
    SelectGraphNode {
        mod_idx: usize,
        id: NodeId,
    },
    /// Zoom de la vista grafo. `mult` multiplica el zoom actual; `ancla` =
    /// cursor en coords de ventana para fijar el punto bajo él (zoom-a-
    /// cursor de la rueda). `None` ⇒ zoom hacia el centro del lienzo
    /// (botones +/−).
    ZoomGraph {
        mult: f32,
        ancla: Option<(f32, f32)>,
    },
    /// Encuadra todo el grafo en el lienzo (fit-to-view) y resetea el pan.
    FitGraph,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Right-click en el área de trabajo → abre el menú de edición en
    /// `(x, y)` de ventana, operando sobre el campo de texto con foco
    /// (field del form o caja de búsqueda de la lista).
    EditMenuOpen(f32, f32),
    /// Acción elegida en el menú de edición contextual.
    EditMenuAction(EditAction),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Navegación por teclado en el dropdown del menú principal.
    MenuNav(i32),
    /// Ejecuta la fila activa del menú principal (Enter).
    MenuActivate,
    /// Tick de animación de los dropdowns (sólo re-render).
    MenuTick,
    /// Navegación por teclado en el menú de edición contextual.
    EditNav(i32),
    /// Ejecuta la fila activa del menú de edición (Enter).
    EditActivate,
}

/// Sesión de edición de un formulario. Vive en el `Model` porque cada
/// input mantiene su `TextInputState` (cursor + buffer) entre frames.
struct FormState {
    module_idx: usize,
    entity: String,
    title: String,
    on_submit: Action,
    fields: Vec<FieldRuntime>,
    /// `Some(id)` = edición de un record existente; `None` = alta nueva.
    editing: Option<Uuid>,
    /// Estado original del record en edición (para computar el delta).
    original: Option<Value>,
    /// Índice del field con foco de teclado (sólo fields de texto).
    focused: Option<usize>,
    /// Error de validación / del backend tras un submit fallido.
    error: Option<String>,
}

/// Un field vivo del form: su spec del manifest + el buffer editable.
/// Para TODOS los kinds el value crudo vive como string en `input`
/// (text/multiline/number/date se teclean; select/entityref/bool/autoid
/// se setean por click), y `parse_field_value` lo convierte al submit.
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

/// Ficha de detalle activa: el record `id` de `entity`, renderizado con
/// la vista `view_key` (un `View::Detail`) del módulo `module_idx`.
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
    /// Sesión de edición in-situ de un único campo de la ficha de detalle
    /// activa (el record vive en `detail`). `spec` + buffer; confirmar
    /// persiste sólo ese campo. Mutuamente excluyente con `form`.
    inline_edit: Option<FieldRuntime>,
    toast: Option<Toast>,
    /// Estado de la lista activa (se resetea al cambiar de vista).
    list_search: TextInputState,
    list_search_focused: bool,
    /// Columna de orden + dirección (`true` = ascendente).
    list_sort: Option<(String, bool)>,
    list_page: usize,
    /// Toggles de filtro de reporte activos, por clave `"viewkey#idx"`.
    /// Persisten entre frames y entre cambios de vista (un reporte
    /// recuerda sus filtros si volvés a él).
    report_filters: BTreeSet<String>,
    /// Drill-down activo: cuando hacés click en una fila de un desglose,
    /// se navega a la lista de esa entity filtrada a ese grupo. La lista
    /// aplica el filtro y muestra un chip para limpiarlo.
    drill: Option<DrillFilter>,
    /// Posiciones override de los nodos de la vista grafo, por clave
    /// estable `(module_id, nombre_morfismo)`. Vacío = layout automático
    /// por rango topológico; al arrastrar un nodo se fija su `(x, y)` acá
    /// y se persiste a `layout_path` al soltar.
    graph_pos: BTreeMap<(String, String), (f32, f32)>,
    /// Sidecar JSON donde persiste `graph_pos` entre arranques (junto al
    /// event log: `<log>.layout.json`).
    layout_path: PathBuf,
    /// Morfismo seleccionado en la vista grafo (`mod_idx`, `node_id`).
    /// Click-derecho lo fija y resalta su cono (aguas arriba + abajo);
    /// volver a clickearlo lo limpia.
    graph_selected: Option<(usize, NodeId)>,
    /// Cámara de la vista grafo: factor de zoom (1.0 = tamaño base) y pan
    /// en coords locales al lienzo. `pantalla = mundo · zoom + pan`. La
    /// rueda hace zoom-a-cursor; los botones +/− y «ajustar» lo recentran.
    graph_zoom: f32,
    graph_pan: (f32, f32),
    /// Menú principal: índice del menú raíz abierto (`None` cerrado).
    menu_open: Option<usize>,
    /// Fila activa (teclado) del dropdown principal. `usize::MAX` = ninguna.
    menu_active: usize,
    /// Animación de aparición/swap del dropdown principal.
    menu_anim: Tween<f32>,
    /// Menú de edición contextual: ancla `(x, y)` en ventana (`None` cerrado).
    edit_menu: Option<(f32, f32)>,
    /// Fila activa (teclado) del menú de edición. `usize::MAX` = ninguna.
    edit_active: usize,
    /// Animación de aparición del menú de edición.
    edit_anim: Tween<f32>,
    /// Clipboard del sistema para el menú de edición (cut/copy/paste).
    clipboard: SystemClipboard,
}

/// Filtro de drill-down: la lista de `entity` se recorta a los records
/// cuyo `field` (como texto) es igual a `value` —o **empieza con**
/// `value` si `prefix` (para series temporales: el bucket "2026-02"
/// recorta a las fechas de febrero)—. `label` es el texto legible que
/// se muestra en el chip (puede diferir de `value` cuando el grupo era
/// una ref resuelta a un nombre).
#[derive(Clone)]
struct DrillFilter {
    entity: String,
    field: String,
    value: String,
    label: String,
    prefix: bool,
}

impl Model {
    /// Resetea el estado efímero de la lista (búsqueda/orden/página) al
    /// navegar a otra vista.
    fn reset_list_state(&mut self) {
        self.list_search.clear();
        self.list_search_focused = false;
        self.list_sort = None;
        self.list_page = 0;
    }

    /// Campo de texto con foco activo: el field del form (si hay uno
    /// focuseado) o, en su defecto, la caja de búsqueda de la lista.
    /// Es sobre éste que opera el menú de edición contextual.
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

/// Banderas del menú de edición derivadas del campo con foco. Sin foco,
/// banderas por defecto (todo deshabilitado salvo Pegar).
fn edit_flags(model: &Model) -> EditFlags {
    match model.focused_input() {
        Some(input) => EditFlags::from_editor(input.editor(), input.is_masked()),
        None => EditFlags::default(),
    }
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
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

/// Menú principal de Nakui. Refleja el estado real: el submenú "Editar"
/// se atenúa cuando no hay campo de texto con foco / sin selección /
/// historial; "Ver" y "Archivo" mapean a las acciones reales de la vista
/// activa (export CSV/MD, nuevo record, limpiar drill, ajustar grafo).
fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};

    // --- Editar: estado del campo de texto con foco. ---
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

    // --- Archivo: depende de la vista activa. ---
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

    // --- Ver: navegación del módulo / grafo / drill. ---
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
        .menu(
            Menu::new("Ayuda")
                .item(MenuItem::new("Acerca de Nakui", "help.about")),
        )
}

/// Datos de la vista activa que el menú "Archivo" necesita: la entity
/// asociada (para "Nuevo record") y si es lista/reporte (para los export).
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

/// Traduce el `command` del menú principal al `Msg` real de la app. Sólo
/// mapea comandos cuya acción ya existe; `None` para los sin efecto
/// (p.ej. "Acerca de", que no muta estado, o un export sin vista válida).
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
        "file.export_md" => view_key.map(|view_key| Msg::ExportReport {
            module_idx: mod_idx,
            view_key,
        }),
        "file.cancel_form" => Some(Msg::CancelForm),
        "view.clear_drill" => Some(Msg::ClearDrill),
        "view.fit_graph" => Some(Msg::FitGraph),
        "view.zoom_in" => Some(Msg::ZoomGraph { mult: ZOOM_BASE, ancla: None }),
        "view.zoom_out" => Some(Msg::ZoomGraph { mult: 1.0 / ZOOM_BASE, ancla: None }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Pantallazo headless: Model sembrado por el camino real + view → mount →
// layout → paint a vello::Scene → textura wgpu → readback → PNG.
// ---------------------------------------------------------------------------

use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint};

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Construye el `Model` real: los módulos demo del crate (tesorería +
/// ventas) cargados por `load_ui_modules`, executors Rhai, backend con
/// event log efímero y siembra del `seed.json` de cada módulo — el mismo
/// camino que el `init()` de la app. Queda activo el módulo **Tesorería**
/// con su **Tablero** (la vista más densa del shell).
fn modelo_demo() -> Model {
    let modules_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/nakui-modules");
    let (modules, _skipped) = load_ui_modules(&modules_dir).expect("módulos demo del crate");

    // Executors Rhai de los módulos que declaran `nakui_module_dir`.
    let mut executors: BTreeMap<String, Arc<Executor>> = BTreeMap::new();
    for m in &modules {
        if let Some(rel) = &m.nakui_module_dir {
            let nakui_dir = modules_dir.join(&m.id).join(rel);
            match Executor::load_module(&nakui_dir) {
                Ok(exec) => {
                    executors.insert(m.id.clone(), Arc::new(exec));
                }
                Err(e) => eprintln!("pantallazo_nakui: executor de {}: {e}", m.id),
            }
        }
    }

    // Backend con estado efímero: log fresco → la siembra corre completa.
    let state_dir = std::env::temp_dir().join("nakui-pantallazo");
    let _ = std::fs::remove_dir_all(&state_dir);
    std::fs::create_dir_all(&state_dir).expect("dir de estado temporal");
    let log_path = state_dir.join("nakui-pantallazo.jsonl");
    let layout_path = log_path.with_extension("layout.json");
    let (mut backend, _status) = NakuiBackend::open(log_path, 50, executors);

    // Siembra de datos creíbles (cajas + movimientos fechados, clientes +
    // órdenes) — vía el mismo `seed_demo_data` que usa la app al arrancar.
    let initial_toast = seed_demo_data(&mut backend, &modules, &modules_dir);

    // Módulo Tesorería activo, con el primer Dashboard de su menú.
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
    }
}

/// Misma composición que el `view()` de `NakuiApp`: menubar + header +
/// banners + cuerpo (sidebar de módulos + área principal) — los mismos
/// builders reales (`build_banners` / `build_body` de src/layout.rs).
fn vista(model: &Model, theme: &Theme) -> View<Msg> {
    let menubar = menubar_view(&menubar_spec(&app_menu(model), model, theme));
    let header = app_header::<Msg>(
        rimay_localize::t_args(
            "nakui-header",
            &[("count", model.modules.len().to_string().into())],
        ),
        Vec::new(),
        &AppHeaderPalette::from_theme(theme),
    );
    let banners = build_banners(model);
    let body = build_body(model, theme);

    let mut children: Vec<View<Msg>> = vec![menubar, header];
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

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/nakui.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    rimay_localize::init();
    let theme = Theme::dark();
    let model = modelo_demo();
    let root = vista(&model, &theme);

    // view → layout → scene (misma secuencia que el eventloop real).
    let mut layout_tree = LayoutTree::new();
    let mounted = mount(&mut layout_tree, root);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout_tree
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => llimphi_ui::llimphi_layout::taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-nakui"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
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
    let [r, g, b, _] = theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_nakui: escrito {out} ({W}x{H})");
}

/// Lee la textura a CPU y la vuelca como PNG RGBA8.
fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * H as usize) as u64,
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
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
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
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
