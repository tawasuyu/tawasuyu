//! `nakui-ui-llimphi` — binario shell de la metainterfaz Nakui sobre
//! Llimphi.
//!
//! ## Estado actual
//!
//! - Carga módulos UI desde `NAKUI_MODULES_DIR` (o `./nakui-modules`)
//!   vía `cards::load_cards_from_dir`.
//! - Crea `NakuiBackend` (event log persistente + replay + snapshot +
//!   auto-compact). El backend implementa `nahual_meta_runtime::MetaBackend`
//!   completo (seed/update/delete/morphism).
//! - Siembra datos de ejemplo desde un `seed.json` opcional por módulo
//!   (`seed_demo_data`), sólo para entities vacías — los tableros y
//!   gráficos se ven en vivo en el primer arranque sin pisar datos.
//! - Llimphi shell: sidebar de módulos (clickeable) + menú del módulo
//!   activo + área principal.
//! - **Meta-form Llimphi** (paralelo al `nahual-widget-meta-form` GPUI
//!   borrado): cinco vistas meta-driven.
//!   - `List`: filas reales con columnas del manifest (refs resueltas a
//!     su label legible), búsqueda por `search_in`, orden clickeando el
//!     header de columna (asc→desc→sin), paginación, botones editar/
//!     borrar por fila, `👁` cuando declara `row_detail`, `+ Nuevo` y
//!     export CSV de las filas filtradas/ordenadas.
//!   - `Form`: inputs por `FieldKind` (text/multiline/number/date/bool/
//!     select/entity_ref/auto_id), con foco de teclado y submit que
//!     dispara `SeedEntity`, edición (`update` con delta) o `Morphism`.
//!   - `Detail`: ficha de un record (← Volver / ✎ Editar), sus campos,
//!     KPIs scopeados al record (el "360": agregados sobre los records
//!     relacionados vía `via_field`, como stat cards) y las listas de
//!     records relacionados (back-references por `via_field`).
//!   - `Dashboard`: grilla de tarjetas de KPI vía `compute_metric`,
//!     con `ValueFormat` y filtros. Escalares `Count`/`Sum`/`Avg`/
//!     `Min`/`Max` y desgloses `GroupBy` (conteo) / `SumBy` / `AvgBy`
//!     (valor agregado por dimensión — el reporte ERP clásico). Las
//!     claves de un desglose con `group_ref` se resuelven al label del
//!     record referido (p.ej. "facturación por cliente" con nombres).
//!     Cada desglose tiene botón de export CSV. Los filtros aceptan
//!     operadores `eq`/`ne`/`gt`/`gte`/`lt`/`lte`/`between`/`non_empty`
//!     (numéricos o fechas ISO). Cada fila de un desglose es clickeable:
//!     drill-down a la lista de esa entity filtrada al grupo (por el
//!     valor real, aunque la fila muestre el label resuelto). El campo
//!     `chart` de la card elige cómo se pinta el desglose: barras ASCII
//!     (default), torta (`pie`) / dona (`donut`) —sectores proporcionales
//!     con leyenda de color + porcentaje—, o columnas (`columns`) / línea
//!     (`line`) —para series ordenadas, con eje cero y soporte de valores
//!     negativos—. La leyenda siempre es clickeable para drill-down. El
//!     campo `limit` recorta el desglose a las N filas de mayor valor y
//!     colapsa el resto en un bucket "Otros" (no-navegable) — mantiene
//!     legibles los gráficos sobre dimensiones de muchos grupos. El campo
//!     `bucket` (`year`/`month`/`day`) trunca una fecha de grupo ISO y
//!     convierte el desglose en una serie temporal: orden cronológico,
//!     sin recorte — el caso natural de `line`/`columns` (p.ej.
//!     "facturación por mes").
//!   - `Report`: los mismos agregados que un tablero, dispuestos como
//!     documento de una columna (título + subtítulo) con botón
//!     "Exportar (.md)" que vuelca el reporte completo a Markdown.
//!     Soporta `toggles`: controles de filtro interactivos que el
//!     usuario prende/apaga desde la UI y recortan los records de las
//!     cards (opcionalmente acotados a una `entity`) en vivo.
//!   El resultado (o el error de validación) se muestra como banner.
//!
//! El ciclo de escritura ya no pasa por CLI/tests: la UI crea, edita,
//! borra, corre morfismos y consulta tableros directamente sobre el
//! event log.
//!
//! ## Uso
//!
//! ```sh
//! NAKUI_MODULES_DIR=examples/nakui-modules cargo run -p nakui-ui-llimphi
//! # default sin env: ./nakui-modules en pwd.
//! ```
//!
//! ## Módulos
//!
//! El shell (App/Model/Msg/update + layout: sidebar/main/banners + carga
//! de módulos y siembra) vive acá. El resto se reparte:
//! - [`backend`] — `NakuiBackend` (event log + replay + snapshot).
//! - [`widgets`] — helpers de layout/estilo (celdas, líneas, botones).
//! - [`charts`] — render de gráficos (barras/torta/columnas/multi-serie).
//! - [`tablero`] — cómputo de métricas + vistas Dashboard/Report + Markdown.
//! - [`panels`] — vistas Graph/List/Detail/Form meta-driven.
//! - [`export`] — volcado a CSV/Markdown en el cwd.

mod backend;
mod charts;
mod export;
mod panels;
mod tablero;
mod widgets;

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
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, NamedKey, PaintRect, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_field::{field_view, FieldPalette, FieldSpec as FieldWidgetSpec};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_nodegraph::{
    nodegraph_view_styled, NodeId, NodeSpec, NodeTint, NodegraphMetrics, NodegraphPalette, Wire,
};

use nahual_meta_runtime::{
    breakdown_to_csv, bucket_date, cmp_values, compute_clear_fields, compute_field_delta,
    compute_metric, format_value, human_label_for_record, limit_breakdown, parse_field_value,
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
}

struct NakuiApp;

impl App for NakuiApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Nakui"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        // 1. Cargar módulos UI desde el directorio configurado.
        let modules_dir = std::env::var("NAKUI_MODULES_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("nakui-modules"));
        let (modules, mut load_error) = match load_ui_modules(&modules_dir) {
            Ok((mods, skipped)) => {
                let toast = if skipped.is_empty() {
                    None
                } else {
                    Some(format!(
                        "skipeé {} card(s) no-UiModule en {}: {:?}",
                        skipped.len(),
                        modules_dir.display(),
                        skipped
                    ))
                };
                (mods, toast)
            }
            Err(e) => (
                Vec::new(),
                Some(format!(
                    "no pude cargar módulos de {}: {e}",
                    modules_dir.display()
                )),
            ),
        };

        // 2. Cargar Executors para módulos con `nakui_module_dir`.
        let mut executors: BTreeMap<String, Arc<Executor>> = BTreeMap::new();
        for m in &modules {
            let Some(rel) = &m.nakui_module_dir else {
                continue;
            };
            let module_root = modules_dir.join(&m.id);
            let nakui_dir = if std::path::Path::new(rel).is_absolute() {
                PathBuf::from(rel)
            } else {
                module_root.join(rel)
            };
            match Executor::load_module(&nakui_dir) {
                Ok(exec) => {
                    executors.insert(m.id.clone(), Arc::new(exec));
                }
                Err(e) => {
                    let msg = format!(
                        "módulo {}: no pude cargar executor nakui en {}: {e}",
                        m.id,
                        nakui_dir.display()
                    );
                    load_error = Some(match load_error {
                        Some(prev) => format!("{prev}; {msg}"),
                        None => msg,
                    });
                }
            }
        }

        // 3. Construir el backend Nakui (abre log, replay, compact).
        let log_path = std::env::var("NAKUI_EVENT_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("nakui-ui-state.jsonl"));
        // Sidecar del layout del grafo (posiciones de nodos), junto al log.
        let layout_path = log_path.with_extension("layout.json");
        let snapshot_threshold: usize = std::env::var("NAKUI_SNAPSHOT_THRESHOLD")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50);
        let (mut backend, status) = NakuiBackend::open(log_path, snapshot_threshold, executors);
        let mut initial_toast = status.init_toast;
        if let Some(msg) = status.load_error {
            load_error = Some(match load_error {
                Some(prev) => format!("{prev}; {msg}"),
                None => msg,
            });
        }

        // 3.bis. Sembrar datos de ejemplo de cada módulo que traiga un
        // `seed.json`, sólo para las entities que estén vacías (no pisa
        // datos del usuario ni duplica entre arranques). Hace que los
        // tableros/gráficos se vean en vivo en el primer run.
        let seed_toast = seed_demo_data(&mut backend, &modules, &modules_dir);
        if let Some(msg) = seed_toast {
            initial_toast = Some(match initial_toast {
                Some(prev) => format!("{prev} · {msg}"),
                None => msg,
            });
        }

        let selected_module = (!modules.is_empty()).then_some(0);
        let selected_menu =
            selected_module.and_then(|i| (!modules[i].menu.is_empty()).then_some(0));

        Model {
            modules,
            backend: Arc::new(Mutex::new(backend)),
            initial_toast,
            load_error,
            selected_module,
            selected_menu,
            form: None,
            detail: None,
            toast: None,
            list_search: TextInputState::new(),
            list_search_focused: false,
            list_sort: None,
            list_page: 0,
            report_filters: BTreeSet::new(),
            drill: None,
            graph_pos: load_graph_layout(&layout_path),
            layout_path,
            graph_selected: None,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::SelectModule(i) => {
                if i < m.modules.len() {
                    m.selected_module = Some(i);
                    m.selected_menu = (!m.modules[i].menu.is_empty()).then_some(0);
                    m.form = None;
                    m.detail = None;
                    m.drill = None;
                    m.reset_list_state();
                    sync_form_to_menu(&mut m);
                }
            }
            Msg::SelectMenu(i) => {
                if let Some(mod_idx) = m.selected_module {
                    if i < m.modules[mod_idx].menu.len() {
                        m.selected_menu = Some(i);
                        m.form = None;
                        m.detail = None;
                        m.drill = None;
                        m.reset_list_state();
                        sync_form_to_menu(&mut m);
                    }
                }
            }
            Msg::OpenForm {
                module_idx,
                view_key,
            } => {
                if let Some(module) = m.modules.get(module_idx) {
                    if let Some(ModuleView::Form(fv)) = module.views.get(&view_key) {
                        m.form = Some(build_form(module_idx, fv, None));
                        m.toast = None;
                    }
                }
            }
            Msg::NewRecord { module_idx, entity } => {
                if let Some(module) = m.modules.get(module_idx) {
                    match find_form_view(module, &entity) {
                        Some(fv) => {
                            m.form = Some(build_form(module_idx, fv, None));
                            m.toast = None;
                        }
                        None => {
                            m.toast = Some(Toast {
                                kind: BannerKind::Warning,
                                text: format!(
                                    "el módulo no declara un Form para la entity '{entity}'"
                                ),
                            });
                        }
                    }
                }
            }
            Msg::EditRecord {
                module_idx,
                entity,
                id,
            } => {
                let record = m
                    .backend
                    .lock()
                    .ok()
                    .and_then(|b| b.load_record(&entity, id));
                match (m.modules.get(module_idx), record) {
                    (Some(module), Some(rec)) => match find_form_view(module, &entity) {
                        Some(fv) => {
                            m.form = Some(build_form(module_idx, fv, Some((id, rec))));
                            m.toast = None;
                        }
                        None => {
                            m.toast = Some(Toast {
                                kind: BannerKind::Warning,
                                text: format!(
                                    "el módulo no declara un Form para editar '{entity}'"
                                ),
                            });
                        }
                    },
                    _ => {
                        m.toast = Some(Toast {
                            kind: BannerKind::Error,
                            text: "no pude cargar el record a editar".into(),
                        });
                    }
                }
            }
            Msg::DeleteRecord { entity, id } => {
                let result = m
                    .backend
                    .lock()
                    .map_err(|_| "backend lock envenenado".to_string())
                    .and_then(|mut b| b.delete(&entity, id));
                m.toast = Some(match result {
                    Ok(_) => Toast {
                        kind: BannerKind::Success,
                        text: format!("borrado {} de {entity}", short_uuid(&id)),
                    },
                    Err(e) => Toast {
                        kind: BannerKind::Error,
                        text: format!("no pude borrar: {e}"),
                    },
                });
            }
            Msg::FocusField(i) => {
                if let Some(form) = &mut m.form {
                    if form
                        .fields
                        .get(i)
                        .map(|f| is_text_field(f.spec.kind))
                        .unwrap_or(false)
                    {
                        form.focused = Some(i);
                    }
                }
            }
            Msg::FieldKey(ev) => {
                if let Some(form) = &mut m.form {
                    if let Some(i) = form.focused {
                        if let Some(fr) = form.fields.get_mut(i) {
                            fr.input.apply_key(&ev);
                        }
                    }
                }
            }
            Msg::SetSelect(i, value) => {
                if let Some(form) = &mut m.form {
                    if let Some(fr) = form.fields.get_mut(i) {
                        fr.input.set_text(value);
                    }
                    form.focused = None;
                }
            }
            Msg::ToggleBool(i) => {
                if let Some(form) = &mut m.form {
                    if let Some(fr) = form.fields.get_mut(i) {
                        let now = fr.raw() == "true";
                        fr.input.set_text(if now { "false" } else { "true" });
                    }
                }
            }
            Msg::SubmitForm => {
                submit_form(&mut m);
            }
            Msg::CancelForm => {
                m.form = None;
                m.toast = None;
            }
            Msg::DismissToast => {
                m.toast = None;
            }
            Msg::OpenDetail {
                module_idx,
                view_key,
                entity,
                id,
            } => {
                m.detail = Some(DetailState {
                    module_idx,
                    view_key,
                    entity,
                    id,
                });
                m.form = None;
                m.toast = None;
            }
            Msg::CloseDetail => {
                m.detail = None;
            }
            Msg::FocusListSearch => {
                m.list_search_focused = true;
            }
            Msg::ListSearchKey(ev) => {
                if m.list_search_focused && m.list_search.apply_key(&ev) {
                    // La búsqueda cambió: volver a la primera página.
                    m.list_page = 0;
                }
            }
            Msg::SortBy(field) => {
                m.list_sort = next_sort(m.list_sort.take(), &field);
                m.list_page = 0;
            }
            Msg::ListPagePrev => {
                m.list_page = m.list_page.saturating_sub(1);
            }
            Msg::ListPageNext => {
                // El clamp real lo hace el render contra el total; acá
                // sólo avanzamos (el render no deja pasar de la última).
                m.list_page = m.list_page.saturating_add(1);
            }
            Msg::ExportCsv { entity } => {
                m.toast = Some(export_active_list_csv(&m, &entity));
            }
            Msg::ExportReport {
                module_idx,
                view_key,
            } => {
                m.toast = Some(export_report_md(&m, module_idx, &view_key));
            }
            Msg::ExportBreakdownCsv {
                module_idx,
                view_key,
                card_idx,
            } => {
                m.toast = Some(export_breakdown_csv(&m, module_idx, &view_key, card_idx));
            }
            Msg::ToggleReportFilter { view_key, idx } => {
                let key = report_filter_key(&view_key, idx);
                if !m.report_filters.remove(&key) {
                    m.report_filters.insert(key);
                }
            }
            Msg::DrillDown {
                entity,
                field,
                value,
                label,
                prefix,
            } => {
                // Buscar una vista List de esa entity en el módulo activo
                // y navegar a ella aplicando el filtro.
                if let Some(mod_idx) = m.selected_module {
                    let module = &m.modules[mod_idx];
                    let target = module.menu.iter().position(|item| {
                        matches!(
                            module.views.get(&item.view),
                            Some(ModuleView::List(lv)) if lv.entity == entity
                        )
                    });
                    match target {
                        Some(menu_idx) => {
                            m.selected_menu = Some(menu_idx);
                            m.form = None;
                            m.detail = None;
                            m.reset_list_state();
                            m.drill = Some(DrillFilter {
                                entity,
                                field,
                                value,
                                label,
                                prefix,
                            });
                        }
                        None => {
                            m.toast = Some(Toast {
                                kind: BannerKind::Error,
                                text: format!("no hay lista de '{entity}' para abrir"),
                            });
                        }
                    }
                }
            }
            Msg::ClearDrill => {
                m.drill = None;
            }
            Msg::DragGraphNode {
                module_id,
                morphism,
                dx,
                dy,
                end,
            } => {
                // El delta llega ya integrado por evento; partimos de la
                // posición actual (override previo o la base del layout)
                // y la desplazamos, clampeada a coordenadas no-negativas.
                let key = (module_id.clone(), morphism.clone());
                let base = m
                    .graph_pos
                    .get(&key)
                    .copied()
                    .unwrap_or_else(|| graph_base_pos(&m, &module_id, &morphism));
                m.graph_pos
                    .insert(key, ((base.0 + dx).max(0.0), (base.1 + dy).max(0.0)));
                // Al soltar, persistir el layout (no en cada delta).
                if end {
                    save_graph_layout(&m.graph_pos, &m.layout_path);
                }
            }
            Msg::SelectGraphNode { mod_idx, id } => {
                // Toggle: re-clickear el mismo nodo limpia la selección.
                m.graph_selected = if m.graph_selected == Some((mod_idx, id)) {
                    None
                } else {
                    Some((mod_idx, id))
                };
            }
        }
        m
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        // El form gana el teclado cuando tiene un field de texto activo.
        if let Some(form) = &model.form {
            form.focused?;
            if event.state == KeyState::Pressed {
                if let Key::Named(NamedKey::Escape) = &event.key {
                    return Some(Msg::CancelForm);
                }
            }
            return Some(Msg::FieldKey(event.clone()));
        }
        // Si no hay form, la caja de búsqueda de la lista puede tener foco.
        if model.list_search_focused {
            return Some(Msg::ListSearchKey(event.clone()));
        }
        None
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let header = app_header::<Msg>(
            rimay_localize::t_args(
                "nakui-header",
                &[("count", model.modules.len().to_string().into())],
            ),
            Vec::new(),
            &AppHeaderPalette::from_theme(&theme),
        );

        let banners = build_banners(model);
        let body = build_body(model, &theme);

        let mut children: Vec<View<Msg>> = vec![header];
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
}

/// Tras cambiar de módulo/menú: si la vista activa es un `Form`, abre el
/// form fresco (así clickear "Nuevo" en el menú muestra el formulario).
fn sync_form_to_menu(m: &mut Model) {
    let (Some(mod_idx), Some(menu_idx)) = (m.selected_module, m.selected_menu) else {
        return;
    };
    let Some(module) = m.modules.get(mod_idx) else {
        return;
    };
    let Some(item) = module.menu.get(menu_idx) else {
        return;
    };
    if let Some(ModuleView::Form(fv)) = module.views.get(&item.view) {
        m.form = Some(build_form(mod_idx, fv, None));
    }
}

/// Localiza el primer `Form` view de un módulo cuya entity coincide.
fn find_form_view<'a>(module: &'a Module, entity: &str) -> Option<&'a FormView> {
    module.views.values().find_map(|v| match v {
        ModuleView::Form(fv) if fv.entity == entity => Some(fv),
        _ => None,
    })
}

/// Construye un `FormState` desde un `FormView`. `editing` pre-rellena
/// los inputs desde un record existente; en alta, los `AutoId` se
/// rellenan con un UUID nuevo y el resto con su `default`.
fn build_form(module_idx: usize, fv: &FormView, editing: Option<(Uuid, Value)>) -> FormState {
    let fields = fv
        .fields
        .iter()
        .map(|fs| {
            let mut input = TextInputState::new();
            let raw = match &editing {
                Some((_, rec)) => rec
                    .get(&fs.name)
                    .map(value_to_raw)
                    .unwrap_or_default(),
                None => match fs.kind {
                    FieldKind::AutoId => Uuid::new_v4().to_string(),
                    FieldKind::Boolean => fs.default.clone().unwrap_or_else(|| "false".into()),
                    _ => fs.default.clone().unwrap_or_default(),
                },
            };
            input.set_text(raw);
            FieldRuntime {
                spec: fs.clone(),
                input,
            }
        })
        .collect();

    FormState {
        module_idx,
        entity: fv.entity.clone(),
        title: fv.title.clone(),
        on_submit: fv.on_submit.clone(),
        fields,
        editing: editing.as_ref().map(|(id, _)| *id),
        original: editing.map(|(_, v)| v),
        focused: None,
        error: None,
    }
}

/// Representación cruda (string) de un valor JSON para precargar un input.
fn value_to_raw(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn is_text_field(kind: FieldKind) -> bool {
    matches!(
        kind,
        FieldKind::Text | FieldKind::Multiline | FieldKind::Number | FieldKind::Date
    )
}

/// Ejecuta el submit del form activo contra el backend. Espeja
/// `commit_seed` / `commit_morphism` del meta-form GPUI borrado:
/// valida required, parsea por kind, valida `EntityRef`s, y ramifica en
/// edición (`update` con delta) vs alta (`seed`/`morphism`).
///
/// Saca el form del modelo con `take()` para no aliasar `m` mientras
/// tiene tomado el guard del backend; si algo falla, lo reinserta con el
/// error puesto para que la UI lo muestre.
fn submit_form(m: &mut Model) {
    let Some(mut form) = m.form.take() else {
        return;
    };

    // 1. Recolectar y parsear los fields.
    let mut obj = serde_json::Map::new();
    let mut to_clear: Vec<String> = Vec::new();
    let mut entity_refs: Vec<(String, String, Uuid)> = Vec::new();
    let mut by_name: BTreeMap<String, String> = BTreeMap::new();
    let mut parse_error: Option<String> = None;

    for fr in &form.fields {
        let raw = fr.raw();
        by_name.insert(fr.spec.name.clone(), raw.clone());

        if fr.spec.required && raw.trim().is_empty() && fr.spec.kind != FieldKind::AutoId {
            parse_error = Some(format!("campo '{}' es obligatorio", fr.spec.label));
            break;
        }
        if raw.is_empty() && !fr.spec.required {
            to_clear.push(fr.spec.name.clone());
            continue;
        }
        let value = match parse_field_value(fr.spec.kind, &raw) {
            Ok(v) => v,
            Err(e) => {
                parse_error = Some(format!("campo '{}': {e}", fr.spec.label));
                break;
            }
        };
        if fr.spec.kind == FieldKind::EntityRef {
            if let (Some(target), Some(uuid_str)) = (&fr.spec.ref_entity, value.as_str()) {
                if let Ok(id) = Uuid::parse_str(uuid_str) {
                    entity_refs.push((fr.spec.label.clone(), target.clone(), id));
                }
            }
        }
        obj.insert(fr.spec.name.clone(), value);
    }

    if let Some(e) = parse_error {
        form.error = Some(e);
        m.form = Some(form);
        return;
    }

    // 2. Datos derivados (sin tocar `form` durante el lock del backend).
    let module_id = m
        .modules
        .get(form.module_idx)
        .map(|md| md.id.clone())
        .unwrap_or_default();
    let entity = form.entity.clone();
    let editing = form.editing;
    let original = form.original.clone();
    let on_submit = form.on_submit.clone();
    let specs: BTreeMap<String, FieldSpec> = form
        .fields
        .iter()
        .map(|f| (f.spec.name.clone(), f.spec.clone()))
        .collect();

    // 3. Resolver contra el backend (lock una sola vez).
    let result: Result<WriteOutcome, String> = match m.backend.lock() {
        Ok(mut backend) => {
            let refs_ok: Result<(), String> = if entity_refs.is_empty() {
                Ok(())
            } else {
                validate_entity_refs(|e, id| backend.load_record(e, id), &entity_refs)
            };
            match refs_ok {
                Err(e) => Err(e),
                Ok(()) => {
                    if let Some(id) = editing {
                        let current = original.unwrap_or(Value::Null);
                        let set = compute_field_delta(&current, &obj);
                        let clear = compute_clear_fields(&current, &to_clear);
                        backend.update(&entity, id, set, clear)
                    } else {
                        match &on_submit {
                            Action::SeedEntity { entity: e, .. } => backend.seed(e, obj),
                            Action::Morphism {
                                name,
                                inputs,
                                params,
                                ..
                            } => commit_morphism(
                                &mut backend,
                                &module_id,
                                name,
                                inputs,
                                params,
                                &by_name,
                                &specs,
                            ),
                            Action::OpenView { .. } => {
                                Err("on_submit OpenView no crea ni edita records".into())
                            }
                        }
                    }
                }
            }
        }
        Err(_) => Err("backend lock envenenado".into()),
    };

    // 4. Toast + navegación.
    match result {
        Ok(outcome) => {
            let verb = if editing.is_some() { "guardado" } else { "creado" };
            let mut text = match outcome.changed {
                0 => format!("{entity}: sin cambios"),
                _ => format!("{entity} {verb} ✓"),
            };
            if let Some(post) = outcome.post_status {
                text = format!("{text} · {post}");
            }
            m.toast = Some(Toast {
                kind: BannerKind::Success,
                text,
            });
            // `form` queda consumido (no reinsertado): cerramos la sesión.
            navigate_next_view(m, &on_submit);
        }
        Err(e) => {
            form.error = Some(e);
            m.form = Some(form);
        }
    }
}

/// Resuelve inputs (role→field→UUID) y params (fields → JSON) y delega
/// al backend. Espejo de `commit_morphism` del widget GPUI.
fn commit_morphism(
    backend: &mut NakuiBackend,
    module_id: &str,
    name: &str,
    inputs_map: &BTreeMap<String, String>,
    params_fields: &[String],
    by_name: &BTreeMap<String, String>,
    specs: &BTreeMap<String, FieldSpec>,
) -> Result<WriteOutcome, String> {
    // Inputs: cada (role, field) → parsear el value del field como UUID.
    let mut inputs: BTreeMap<String, Uuid> = BTreeMap::new();
    for (role, field_name) in inputs_map {
        let raw = by_name
            .get(field_name)
            .ok_or_else(|| format!("input field '{field_name}' no existe en el form"))?;
        let id = Uuid::parse_str(raw.trim()).map_err(|_| {
            format!("input '{role}' (field '{field_name}'): '{raw}' no es UUID válido")
        })?;
        inputs.insert(role.clone(), id);
    }

    // Params: lista explícita, o todos los fields que no son inputs.
    let input_fields: BTreeSet<&String> = inputs_map.values().collect();
    let field_iter: Vec<String> = if params_fields.is_empty() {
        by_name
            .keys()
            .filter(|k| !input_fields.contains(*k))
            .cloned()
            .collect()
    } else {
        params_fields.to_vec()
    };

    let mut params_obj = serde_json::Map::new();
    for field_name in field_iter {
        let raw = by_name.get(&field_name).cloned().unwrap_or_default();
        let spec = specs.get(&field_name);
        let value = resolve_param_value(&field_name, &raw, spec)?;
        params_obj.insert(field_name, value);
    }

    backend.morphism(module_id, name, inputs, Value::Object(params_obj))
}

/// Tras un submit exitoso, salta al `next_view` declarado en la acción
/// (típicamente `"list"`), seleccionando ese ítem del menú del módulo.
fn navigate_next_view(m: &mut Model, action: &Action) {
    let next = match action {
        Action::SeedEntity { next_view, .. } => next_view.clone(),
        Action::Morphism { next_view, .. } => next_view.clone(),
        Action::OpenView { view, .. } => Some(view.clone()),
    };
    let Some(view_key) = next else {
        return;
    };
    let Some(mod_idx) = m.selected_module else {
        return;
    };
    if let Some(module) = m.modules.get(mod_idx) {
        if let Some(i) = module.menu.iter().position(|it| it.view == view_key) {
            m.selected_menu = Some(i);
        }
    }
}

fn build_banners(model: &Model) -> Vec<View<Msg>> {
    let mut out: Vec<View<Msg>> = Vec::new();
    if let Some(t) = &model.toast {
        out.push(
            banner_view::<Msg>(t.kind, t.text.clone()).on_click(Msg::DismissToast),
        );
    }
    if let Some(msg) = &model.initial_toast {
        out.push(banner_view::<Msg>(BannerKind::Info, msg.clone()));
    }
    if let Some(msg) = &model.load_error {
        out.push(banner_view::<Msg>(BannerKind::Error, msg.clone()));
    }
    out
}

fn build_body(model: &Model, theme: &Theme) -> View<Msg> {
    let sidebar = build_sidebar(model, theme);
    let main = build_main(model, theme);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![sidebar, main])
}

fn build_sidebar(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = ListPalette::from_theme(theme);

    // Sección 1: lista de módulos.
    let module_rows: Vec<ListRow<Msg>> = model
        .modules
        .iter()
        .enumerate()
        .map(|(i, m)| ListRow {
            label: m.label.clone(),
            selected: model.selected_module == Some(i),
            on_click: Msg::SelectModule(i),
        })
        .collect();

    let modules_panel = list_view(ListSpec {
        rows: module_rows,
        total: model.modules.len(),
        caption: Some(rimay_localize::t_args(
            "nakui-sidebar-modules",
            &[("count", model.modules.len().to_string().into())],
        )),
        truncated_hint: None,
        row_height: ROW_HEIGHT,
        palette,
    });

    // Sección 2: menú del módulo activo.
    let menu_panel = match model.selected_module {
        Some(mod_idx) => {
            let m = &model.modules[mod_idx];
            let rows: Vec<ListRow<Msg>> = m
                .menu
                .iter()
                .enumerate()
                .map(|(i, item)| ListRow {
                    label: match &item.icon {
                        Some(ic) => format!("{ic}  {}", item.label),
                        None => item.label.clone(),
                    },
                    selected: model.selected_menu == Some(i),
                    on_click: Msg::SelectMenu(i),
                })
                .collect();
            list_view(ListSpec {
                rows,
                total: m.menu.len(),
                caption: Some(rimay_localize::t("nakui-sidebar-menu")),
                truncated_hint: None,
                row_height: ROW_HEIGHT,
                palette,
            })
        }
        None => empty_panel(theme, &rimay_localize::t("nakui-empty-no-modules")),
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(SIDEBAR_WIDTH),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![modules_panel, menu_panel])
}

fn build_main(model: &Model, theme: &Theme) -> View<Msg> {
    // Prioridad del área principal: form > ficha de detalle > vista
    // seleccionada en el menú.
    let inner = if let Some(form) = &model.form {
        build_form_panel(model, form, theme)
    } else if let Some(detail) = &model.detail {
        build_detail_panel(model, detail, theme)
    } else {
        match (model.selected_module, model.selected_menu) {
            (Some(mod_idx), Some(menu_idx)) => {
                let m = &model.modules[mod_idx];
                let item = &m.menu[menu_idx];
                match m.views.get(&item.view) {
                    Some(view) => build_view_panel(model, mod_idx, &item.view, view, theme),
                    None => empty_panel(
                        theme,
                        &format!("vista '{}' no existe en el manifest del módulo", item.view),
                    ),
                }
            }
            (Some(_), None) => empty_panel(theme, &rimay_localize::t("nakui-empty-pick-menu")),
            _ => empty_panel(theme, &rimay_localize::t("nakui-empty-pick-module")),
        }
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![inner])
}

/// Clave del Form view dentro del módulo (para `Msg::OpenForm`).
fn form_view_key(module: &Module, fv: &FormView) -> String {
    module
        .views
        .iter()
        .find_map(|(k, v)| match v {
            ModuleView::Form(f) if f.entity == fv.entity && f.title == fv.title => {
                Some(k.clone())
            }
            _ => None,
        })
        .unwrap_or_default()
}

/// Carga UiModules desde un directorio via el brazo unificado
/// `cards::load_cards_from_dir`. Aplica las reglas específicas de la
/// UI: sólo `CardBody::UiModule` cuenta; otros body kinds se reportan
/// en el `skipped` para que el runtime los muestre como banner
/// informativo; cada `Module` se valida via `Module::validate()`;
/// detecta `id` duplicados entre módulos UiModule.
///
/// Devuelve `(modules, skipped_ids)` ordenados por id.
fn load_ui_modules(dir: &std::path::Path) -> Result<(Vec<Module>, Vec<String>), String> {
    let cards = cards::load_cards_from_dir(dir).map_err(|e| e.to_string())?;
    let mut modules: Vec<Module> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for c in cards {
        match c.body {
            CardBody::UiModule(m) => modules.push(m),
            other => skipped.push(format!("{}({})", c.id, other.kind_name())),
        }
    }
    for m in &modules {
        m.validate()
            .map_err(|e| format!("módulo '{}' inválido: {e}", m.id))?;
    }
    modules.sort_by(|a, b| a.id.cmp(&b.id));
    let mut prev: Option<&Module> = None;
    for cur in &modules {
        if let Some(p) = prev {
            if p.id == cur.id {
                return Err(format!(
                    "id de módulo duplicado: '{}' aparece más de una vez",
                    cur.id
                ));
            }
        }
        prev = Some(cur);
    }
    Ok((modules, skipped))
}

/// Siembra datos de ejemplo de cada módulo que traiga un `seed.json`
/// junto a su `module.json` (en `<modules_dir>/<module.id>/seed.json`),
/// **sólo** para las entities que estén vacías en el backend. Devuelve
/// un toast resumen si sembró algo.
///
/// Formato del `seed.json`:
/// ```json
/// { "seed": [
///     { "entity": "Customer", "records": [
///         { "handle": "acme", "data": { "name": "ACME", ... } } ] },
///     { "entity": "Order", "records": [
///         { "data": { "customer": "@acme", "monto": 1200 } } ] } ] }
/// ```
/// Los valores string que empiezan con `@` se resuelven al UUID del
/// record sembrado con ese `handle` (los bloques se procesan en orden,
/// así una entity puede referenciar a otra ya sembrada).
fn seed_demo_data(
    backend: &mut NakuiBackend,
    modules: &[Module],
    modules_dir: &std::path::Path,
) -> Option<String> {
    let mut total = 0usize;
    let mut entities_seeded: Vec<String> = Vec::new();
    for m in modules {
        let path = modules_dir.join(&m.id).join("seed.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(doc) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(blocks) = doc.get("seed").and_then(Value::as_array) else {
            continue;
        };
        // handle → UUID de los records ya sembrados (para resolver `@`).
        let mut handles: BTreeMap<String, String> = BTreeMap::new();
        for block in blocks {
            let Some(entity) = block.get("entity").and_then(Value::as_str) else {
                continue;
            };
            // Idempotencia: no sembrar si la entity ya tiene records.
            if !backend.list_records(entity).is_empty() {
                continue;
            }
            let Some(records) = block.get("records").and_then(Value::as_array) else {
                continue;
            };
            let mut count = 0usize;
            for rec in records {
                let Some(data) = rec.get("data").and_then(Value::as_object) else {
                    continue;
                };
                // Resolver refs `@handle` a UUIDs ya sembrados.
                let mut obj = data.clone();
                for v in obj.values_mut() {
                    if let Value::String(s) = v {
                        if let Some(key) = s.strip_prefix('@') {
                            if let Some(uuid) = handles.get(key) {
                                *v = Value::String(uuid.clone());
                            }
                        }
                    }
                }
                match backend.seed(entity, obj) {
                    Ok(outcome) => {
                        count += 1;
                        if let (Some(handle), Some(id)) =
                            (rec.get("handle").and_then(Value::as_str), outcome.id)
                        {
                            handles.insert(handle.to_string(), id.to_string());
                        }
                    }
                    Err(_) => continue,
                }
            }
            if count > 0 {
                entities_seeded.push(format!("{entity}×{count}"));
                total += count;
            }
        }
    }
    (total > 0).then(|| format!("sembré datos de ejemplo: {}", entities_seeded.join(", ")))
}

/// Carga el sidecar del layout del grafo (posiciones de nodos por
/// `(module_id, morfismo)`). Formato: array de `{module, morphism, x,
/// y}`. Ausente/ilegible → mapa vacío (layout automático).
fn load_graph_layout(path: &std::path::Path) -> BTreeMap<(String, String), (f32, f32)> {
    let mut out = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    let Ok(arr) = serde_json::from_str::<Vec<Value>>(&text) else {
        return out;
    };
    for e in arr {
        let (Some(m), Some(f), Some(x), Some(y)) = (
            e.get("module").and_then(Value::as_str),
            e.get("morphism").and_then(Value::as_str),
            e.get("x").and_then(Value::as_f64),
            e.get("y").and_then(Value::as_f64),
        ) else {
            continue;
        };
        out.insert((m.to_string(), f.to_string()), (x as f32, y as f32));
    }
    out
}

/// Persiste el layout del grafo al sidecar. Errores de IO se ignoran
/// (perder un layout no es fatal — se recae al automático).
fn save_graph_layout(pos: &BTreeMap<(String, String), (f32, f32)>, path: &std::path::Path) {
    let arr: Vec<Value> = pos
        .iter()
        .map(|((m, f), (x, y))| {
            serde_json::json!({ "module": m, "morphism": f, "x": x, "y": y })
        })
        .collect();
    if let Ok(text) = serde_json::to_string_pretty(&arr) {
        let _ = std::fs::write(path, text);
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<NakuiApp>();
}

#[cfg(test)]
mod tests {
    //! Tests del shell. Los tests del backend impl viven en `backend.rs`.
    //! Los helpers puros (preview_value/short_uuid/short_hash) en
    //! `nahual-meta-runtime`.

    use super::*;
    use serde_json::json;

    /// E2E mínimo del WAL: armamos un log a mano con dos seeds, abrimos
    /// con `EventLog::open` + `replay_into`, y verificamos que el
    /// `MemoryStore` queda con esos records aplicados. Reproduce el
    /// flujo del startup de NakuiBackend.
    #[test]
    fn event_log_replay_restores_memory_store() {
        use nakui_core::event_log::{replay_into, EventLog, LogEntry};
        use nakui_core::store::{MemoryStore, Store};
        use uuid::Uuid;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        {
            let mut log = EventLog::open(&path).unwrap();
            log.append(LogEntry::Seed {
                seq: 0,
                entity: "customer".into(),
                id: id_a,
                data: json!({"name": "Acme"}),
                schema_hash: None,
            })
            .unwrap();
            log.append(LogEntry::Seed {
                seq: 1,
                entity: "customer".into(),
                id: id_b,
                data: json!({"name": "Globex"}),
                schema_hash: None,
            })
            .unwrap();
        }

        let log = EventLog::open(&path).unwrap();
        assert_eq!(log.next_seq(), 2);
        let mut store = MemoryStore::new();
        replay_into(&log, &mut store).unwrap();

        assert_eq!(store.load("customer", id_a), Some(json!({"name": "Acme"})));
        assert_eq!(
            store.load("customer", id_b),
            Some(json!({"name": "Globex"}))
        );

        let _ = std::fs::remove_file(&path);
    }

    /// El layout del grafo round-trippea por el sidecar JSON (claves
    /// estables `(module_id, morfismo)`), y un archivo ausente da mapa
    /// vacío.
    #[test]
    fn graph_layout_round_trips_through_sidecar() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        // Archivo ausente → vacío.
        assert!(load_graph_layout(&path).is_empty());

        let mut pos: BTreeMap<(String, String), (f32, f32)> = BTreeMap::new();
        pos.insert(("ventas".into(), "calcular_total".into()), (120.0, 40.0));
        pos.insert(("ventas".into(), "marcar_pagado".into()), (300.5, 180.25));
        save_graph_layout(&pos, &path);

        let loaded = load_graph_layout(&path);
        assert_eq!(loaded, pos);

        let _ = std::fs::remove_file(&path);
    }

    /// El seeder de demo siembra el `seed.json` del módulo `ventas`,
    /// resuelve las refs `@handle` a UUIDs reales y es idempotente.
    #[test]
    fn seed_demo_data_seeds_ventas_and_is_idempotent() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let modules_dir = std::path::Path::new("examples/nakui-modules");
        let (modules, _) = load_ui_modules(modules_dir).unwrap();
        let (mut backend, _) = NakuiBackend::open(path.clone(), 1000, BTreeMap::new());

        // Primer sembrado: 9 clientes + 12 órdenes.
        let toast = seed_demo_data(&mut backend, &modules, modules_dir);
        assert!(toast.is_some(), "debió sembrar en el primer arranque");
        let customers = backend.list_records("Customer");
        let orders = backend.list_records("Order");
        assert_eq!(customers.len(), 9);
        assert_eq!(orders.len(), 12);

        // Las refs `@handle` se resolvieron a UUIDs reales de Customer.
        let customer_ids: std::collections::BTreeSet<String> = customers
            .iter()
            .map(|(id, _)| id.to_string())
            .collect();
        for (_, ord) in &orders {
            let cust = ord.get("customer").and_then(Value::as_str).unwrap();
            assert!(
                customer_ids.contains(cust),
                "la orden referencia un Customer inexistente: {cust}"
            );
        }

        // Segundo sembrado: idempotente (entities no vacías → no toca nada).
        let again = seed_demo_data(&mut backend, &modules, modules_dir);
        assert!(again.is_none(), "no debió re-sembrar entities ya pobladas");
        assert_eq!(backend.list_records("Customer").len(), 9);
        assert_eq!(backend.list_records("Order").len(), 12);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::backend::snapshot_path_for(&path));
    }

    /// Los KPIs de la ficha (`DetailMetric`) se scopean a los records
    /// relacionados: ACME tiene 2 órdenes (1200 + 800, ambas pagadas).
    #[test]
    fn detail_metric_scopes_to_related_records() {
        use nahual_meta_schema::{CardFilter, FilterOp, Metric};

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let modules_dir = std::path::Path::new("examples/nakui-modules");
        let (modules, _) = load_ui_modules(modules_dir).unwrap();
        let (mut backend, _) = NakuiBackend::open(path.clone(), 1000, BTreeMap::new());
        seed_demo_data(&mut backend, &modules, modules_dir);

        let acme = backend
            .list_records("Customer")
            .into_iter()
            .find(|(_, v)| v.get("name").and_then(Value::as_str) == Some("ACME Corp"))
            .map(|(id, _)| id)
            .unwrap();

        let dm = |metric, filter| DetailMetric {
            label: "x".into(),
            entity: "Order".into(),
            via_field: "customer".into(),
            metric,
            filter,
            format: ValueFormat::default(),
        };

        assert_eq!(
            compute_detail_metric(&backend, &dm(Metric::Count, None), acme),
            MetricResult::Scalar(2.0)
        );
        assert_eq!(
            compute_detail_metric(
                &backend,
                &dm(Metric::Sum { field: "monto".into() }, None),
                acme
            ),
            MetricResult::Scalar(2000.0)
        );
        // Cobrado (pagado=true) = mismas 2 órdenes.
        let pagado = CardFilter {
            field: "pagado".into(),
            op: FilterOp::Eq,
            value: Some("true".into()),
            min: None,
            max: None,
        };
        assert_eq!(
            compute_detail_metric(
                &backend,
                &dm(Metric::Sum { field: "monto".into() }, Some(pagado)),
                acme
            ),
            MetricResult::Scalar(2000.0)
        );
        assert_eq!(
            compute_detail_metric(
                &backend,
                &dm(Metric::Avg { field: "monto".into() }, None),
                acme
            ),
            MetricResult::Scalar(1000.0)
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::backend::snapshot_path_for(&path));
    }

    /// Las claves crudas de un desglose se muestran con su label: un
    /// `Select` resuelve a su `label` declarado, un booleano a Sí/No.
    #[test]
    fn humanize_relabels_select_and_boolean_keys() {
        use nahual_meta_schema::Metric;

        let modules_dir = std::path::Path::new("examples/nakui-modules");
        let (modules, _) = load_ui_modules(modules_dir).unwrap();
        let ventas = modules.iter().find(|m| m.id == "ventas").unwrap();

        // Select: tier → labels declarados; booleano → Sí/No; texto → sin mapa.
        let tier = field_label_map(ventas, "Customer", "tier").unwrap();
        assert_eq!(tier.get("pro").map(String::as_str), Some("Pro"));
        assert_eq!(tier.get("enterprise").map(String::as_str), Some("Enterprise"));
        let pagado = field_label_map(ventas, "Order", "pagado").unwrap();
        assert_eq!(pagado.get("true").map(String::as_str), Some("Sí"));
        assert_eq!(pagado.get("false").map(String::as_str), Some("No"));
        assert!(field_label_map(ventas, "Customer", "name").is_none());

        let card = |metric, group_ref: Option<&str>, bucket| DashboardCard {
            label: "x".into(),
            entity: "Customer".into(),
            metric,
            filter: None,
            format: ValueFormat::default(),
            group_ref: group_ref.map(Into::into),
            chart: ChartKind::Bars,
            limit: None,
            bucket,
        };

        // GroupBy de tier: claves crudas → labels.
        let mut r = MetricResult::Breakdown(vec![("pro".into(), 3), ("free".into(), 2)]);
        humanize_breakdown_labels(
            &mut r,
            ventas,
            &card(Metric::GroupBy { field: "tier".into() }, None, None),
        );
        assert_eq!(
            r,
            MetricResult::Breakdown(vec![("Pro".into(), 3), ("Free".into(), 2)])
        );

        // group_ref presente → NO humaniza la dimensión de grupo.
        let mut r2 = MetricResult::Breakdown(vec![("pro".into(), 3)]);
        humanize_breakdown_labels(
            &mut r2,
            ventas,
            &card(Metric::GroupBy { field: "tier".into() }, Some("Customer"), None),
        );
        assert_eq!(r2, MetricResult::Breakdown(vec![("pro".into(), 3)]));

        // SumBySeries: la dimensión de serie (pagado) se humaniza a Sí/No.
        let order_card = DashboardCard {
            label: "x".into(),
            entity: "Order".into(),
            metric: Metric::SumBySeries {
                group: "fecha".into(),
                series: "pagado".into(),
                value: "monto".into(),
            },
            filter: None,
            format: ValueFormat::default(),
            group_ref: None,
            chart: ChartKind::Line,
            limit: None,
            bucket: Some(nahual_meta_schema::DateBucket::Month),
        };
        let mut r3 = MetricResult::MultiBreakdown {
            groups: vec!["2026-01".into()],
            series: vec![("true".into(), vec![100.0]), ("false".into(), vec![50.0])],
        };
        humanize_breakdown_labels(&mut r3, ventas, &order_card);
        assert_eq!(
            r3,
            MetricResult::MultiBreakdown {
                // bucket activo → groups (fechas) intactos.
                groups: vec!["2026-01".into()],
                series: vec![("Sí".into(), vec![100.0]), ("No".into(), vec![50.0])],
            }
        );
    }

    /// El drill-down por prefijo (series temporales) recorta la lista al
    /// bucket: "2026-02" trae sólo las órdenes de febrero.
    #[test]
    fn drill_prefix_filters_list_to_month() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let modules_dir = std::path::Path::new("examples/nakui-modules");
        let (modules, _) = load_ui_modules(modules_dir).unwrap();
        let (mut backend, _) = NakuiBackend::open(path.clone(), 1000, BTreeMap::new());
        seed_demo_data(&mut backend, &modules, modules_dir);

        let lv = ListView {
            title: "Órdenes".into(),
            entity: "Order".into(),
            columns: Vec::new(),
            actions: Vec::new(),
            search_in: Vec::new(),
            row_detail: None,
        };
        let feb = DrillFilter {
            entity: "Order".into(),
            field: "fecha".into(),
            value: "2026-02".into(),
            label: "2026-02".into(),
            prefix: true,
        };
        let rows = list_filtered_sorted(&backend, &lv, "", &None, Some(&feb));
        assert_eq!(rows.len(), 4, "deberían ser las 4 órdenes de febrero");
        assert!(rows
            .iter()
            .all(|(_, v)| v.get("fecha").and_then(Value::as_str).unwrap().starts_with("2026-02")));

        // Sin prefijo, "2026-02" no matchea ninguna fecha completa.
        let exact = DrillFilter { prefix: false, ..feb.clone() };
        assert_eq!(
            list_filtered_sorted(&backend, &lv, "", &None, Some(&exact)).len(),
            0
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::backend::snapshot_path_for(&path));
    }

    /// `build_form` en alta: AutoId se rellena con un UUID, default
    /// puebla el resto, sin record original.
    #[test]
    fn build_form_fresh_fills_autoid_and_defaults() {
        let fv = FormView {
            title: "Nuevo".into(),
            entity: "Customer".into(),
            fields: vec![
                FieldSpec {
                    name: "id".into(),
                    label: "Id".into(),
                    kind: FieldKind::AutoId,
                    default: None,
                    required: false,
                    help: None,
                    ref_entity: None,
                    options: Vec::new(),
                    section: None,
                },
                FieldSpec {
                    name: "tier".into(),
                    label: "Tier".into(),
                    kind: FieldKind::Text,
                    default: Some("free".into()),
                    required: false,
                    help: None,
                    ref_entity: None,
                    options: Vec::new(),
                    section: None,
                },
            ],
            on_submit: Action::SeedEntity {
                entity: "Customer".into(),
                next_view: Some("list".into()),
            },
        };
        let form = build_form(0, &fv, None);
        assert!(form.editing.is_none());
        // AutoId parseable como UUID.
        assert!(Uuid::parse_str(&form.fields[0].raw()).is_ok());
        assert_eq!(form.fields[1].raw(), "free");
    }

    /// `build_form` en edición: pre-rellena desde el record original.
    #[test]
    fn build_form_editing_prefills_from_record() {
        let fv = FormView {
            title: "Editar".into(),
            entity: "Customer".into(),
            fields: vec![FieldSpec {
                name: "name".into(),
                label: "Nombre".into(),
                kind: FieldKind::Text,
                default: None,
                required: true,
                help: None,
                ref_entity: None,
                options: Vec::new(),
                section: None,
            }],
            on_submit: Action::SeedEntity {
                entity: "Customer".into(),
                next_view: None,
            },
        };
        let id = Uuid::new_v4();
        let form = build_form(0, &fv, Some((id, json!({"name": "Acme"}))));
        assert_eq!(form.editing, Some(id));
        assert_eq!(form.fields[0].raw(), "Acme");
    }

    /// El módulo demo (`examples/nakui-modules/ventas.json`) carga,
    /// valida y trae los Form views esperados — guarda el fixture que
    /// el binario abre por default.
    #[test]
    fn demo_module_loads_and_validates() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("nakui-modules");
        let (modules, skipped) = load_ui_modules(&dir).expect("el módulo demo carga");
        assert!(skipped.is_empty(), "no debería skipear cards: {skipped:?}");
        // Dos demos: 'ventas' (meta-form completo) y 'tesoro' (vista grafo).
        assert_eq!(modules.len(), 2);
        let tesoro = modules.iter().find(|m| m.id == "tesoro").expect("tesoro");
        assert!(
            matches!(tesoro.views.get("flujo"), Some(ModuleView::Graph(_))),
            "tesoro expone la vista grafo 'flujo'"
        );
        let m = modules.iter().find(|m| m.id == "ventas").expect("ventas");
        // Tiene un Form para cada entity (customers + orders).
        assert!(find_form_view(m, "Customer").is_some());
        assert!(find_form_view(m, "Order").is_some());
        // Y las cuatro clases de vista están presentes.
        assert!(matches!(m.views.get("tablero"), Some(ModuleView::Dashboard(_))));
        assert!(matches!(
            m.views.get("customer_detail"),
            Some(ModuleView::Detail(_))
        ));
        // La lista de clientes enlaza la ficha vía row_detail.
        if let Some(ModuleView::List(lv)) = m.views.get("customers_list") {
            assert_eq!(lv.row_detail.as_deref(), Some("customer_detail"));
        } else {
            panic!("customers_list debería ser una List");
        }
        // El form de cliente arma un FormState con AutoId pre-rellenado.
        let fv = find_form_view(m, "Customer").unwrap();
        let form = build_form(0, fv, None);
        let id_field = form
            .fields
            .iter()
            .find(|f| f.spec.kind == FieldKind::AutoId)
            .expect("el form tiene un AutoId");
        assert!(Uuid::parse_str(&id_field.raw()).is_ok());
    }

    #[test]
    fn next_sort_cycles_asc_desc_off() {
        // Columna nueva → ascendente.
        assert_eq!(next_sort(None, "name"), Some(("name".into(), true)));
        // Misma columna asc → desc.
        assert_eq!(
            next_sort(Some(("name".into(), true)), "name"),
            Some(("name".into(), false))
        );
        // Misma columna desc → sin orden.
        assert_eq!(next_sort(Some(("name".into(), false)), "name"), None);
        // Otra columna → arranca ascendente.
        assert_eq!(
            next_sort(Some(("name".into(), false)), "tier"),
            Some(("tier".into(), true))
        );
    }

    #[test]
    fn lookup_field_navigates_nested_paths() {
        let v = json!({"name": "Acme", "address": {"city": "Lima"}});
        assert_eq!(lookup_field(&v, "name"), Some(&json!("Acme")));
        assert_eq!(lookup_field(&v, "address.city"), Some(&json!("Lima")));
        assert_eq!(lookup_field(&v, "address.zip"), None);
        assert_eq!(lookup_field(&v, "missing"), None);
    }

    /// `cell_display` aplica el `ValueFormat` de la columna (sin
    /// ref_entity, no toca el backend).
    #[test]
    fn cell_display_formats_currency() {
        use nahual_meta_schema::Column;
        let col = Column {
            field: "monto".into(),
            label: "Monto".into(),
            weight: 1.0,
            ref_entity: None,
            format: ValueFormat::Currency { symbol: "$".into() },
        };
        let v = json!(12000);
        // No necesita backend porque la columna no es ref_entity; el
        // path de formato es puro.
        let out = format_value(Some(&v), &col.format);
        assert_eq!(out, "$12,000");
    }

    #[test]
    fn value_to_raw_covers_scalar_kinds() {
        assert_eq!(value_to_raw(&json!("hola")), "hola");
        assert_eq!(value_to_raw(&json!(true)), "true");
        assert_eq!(value_to_raw(&json!(42)), "42");
        assert_eq!(value_to_raw(&Value::Null), "");
    }

    #[test]
    fn graph_cone_separates_downstream_and_upstream() {
        // Topología del demo `tesoro`:
        //   1→2 (Movimiento), 2→3, 2→4 (Caja.saldo), 3→4 (Asiento).
        // Nodo 0 (abrir_caja) queda aislado.
        let w = |from_node: NodeId, to_node: NodeId| Wire {
            from_node,
            from_output: 0,
            to_node,
            to_input: 0,
        };
        let wires = vec![w(1, 2), w(2, 3), w(2, 4), w(3, 4)];

        // Cono de aplicar_movimiento (2): afecta a 3 y 4; depende de 1.
        let (down, up) = graph_cone(2, &wires, 5);
        assert_eq!(down.into_iter().collect::<Vec<_>>(), vec![3, 4]);
        assert_eq!(up.into_iter().collect::<Vec<_>>(), vec![1]);

        // Cono de cerrar_periodo (4): hoja, depende de 1,2,3; no afecta a nadie.
        let (down, up) = graph_cone(4, &wires, 5);
        assert!(down.is_empty());
        assert_eq!(up.into_iter().collect::<Vec<_>>(), vec![1, 2, 3]);

        // Nodo aislado (0): cono vacío en ambas direcciones.
        let (down, up) = graph_cone(0, &wires, 5);
        assert!(down.is_empty() && up.is_empty());
    }
}
