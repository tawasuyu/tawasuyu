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

// --- Más submódulos del bin: lógica de formularios/acciones, builders de
// layout y persistencia (carga/seed/graph). Tipos en root; free-fns
// pub(crate) re-exportadas para que impl App las llame bare. ---
mod form;
mod io;
mod layout;
#[cfg(test)]
mod tests;

use form::*;
use io::*;
use layout::*;

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<NakuiApp>();
}
