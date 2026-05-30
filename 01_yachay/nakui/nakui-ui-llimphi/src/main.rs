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
//!   - `Detail`: ficha de un record (← Volver / ✎ Editar), sus campos y
//!     las listas de records relacionados (back-references por
//!     `via_field`).
//!   - `Dashboard`: grilla de tarjetas de KPI vía `compute_metric`,
//!     con `ValueFormat` y filtros. Escalares `Count`/`Sum`/`Avg`/
//!     `Min`/`Max` y desgloses `GroupBy` (conteo) / `SumBy` / `AvgBy`
//!     (valor agregado por dimensión — el reporte ERP clásico). Las
//!     claves de un desglose con `group_ref` se resuelven al label del
//!     record referido (p.ej. "facturación por cliente" con nombres).
//!     Cada desglose tiene botón de export CSV. Los filtros aceptan
//!     operadores `eq`/`ne`/`gt`/`gte`/`lt`/`lte`/`between`/`non_empty`
//!     (numéricos o fechas ISO).
//!   - `Report`: los mismos agregados que un tablero, dispuestos como
//!     documento de una columna (título + subtítulo) con botón
//!     "Exportar (.md)" que vuelca el reporte completo a Markdown.
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

mod backend;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use cards::CardBody;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_field::{field_view, FieldPalette, FieldSpec as FieldWidgetSpec};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

use nahual_meta_runtime::{
    breakdown_to_csv, cmp_values, compute_clear_fields, compute_field_delta, compute_metric,
    format_value, human_label_for_record, parse_field_value, preview_value, render_value,
    resolve_param_value, short_uuid, to_csv, validate_entity_refs, MetaBackend, MetricResult,
    WriteOutcome,
};
use nahual_meta_schema::{
    Action, Column, DashboardCard, DashboardView, FieldKind, FieldSpec, FormView, ListView, Module,
    RelatedList, ReportView, ValueFormat, View as ModuleView,
};
use nakui_core::executor::Executor;
use serde_json::Value;
use uuid::Uuid;

use crate::backend::NakuiBackend;

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
        let snapshot_threshold: usize = std::env::var("NAKUI_SNAPSHOT_THRESHOLD")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50);
        let (backend, status) = NakuiBackend::open(log_path, snapshot_threshold, executors);
        let initial_toast = status.init_toast;
        if let Some(msg) = status.load_error {
            load_error = Some(match load_error {
                Some(prev) => format!("{prev}; {msg}"),
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

fn build_view_panel(
    model: &Model,
    mod_idx: usize,
    view_key: &str,
    view: &ModuleView,
    theme: &Theme,
) -> View<Msg> {
    let module = &model.modules[mod_idx];
    match view {
        ModuleView::List(lv) => build_list_panel(model, mod_idx, lv, theme),
        ModuleView::Form(fv) => {
            // Form alcanzado sin sesión activa (p.ej. tras cancelar):
            // ofrecer reabrirlo.
            let title = text_line(
                format!("{} · {}", module.label, fv.title),
                16.0,
                theme.fg_text,
            );
            let open = button_styled(
                "+ Abrir formulario",
                btn_style(200.0),
                Alignment::Center,
                &accent_btn(theme),
                Msg::OpenForm {
                    module_idx: mod_idx,
                    view_key: form_view_key(module, fv),
                },
            );
            column(vec![title, open], 8.0)
        }
        ModuleView::Detail(dv) => {
            // Una Detail seleccionada desde el menú no tiene record
            // objetivo: se llega con el 👁 de una fila de lista.
            let lines = vec![format!(
                "elegí un record desde una lista (botón 👁) para ver su ficha de '{}'.",
                dv.entity
            )];
            placeholder_panel(module, &dv.title, lines, theme)
        }
        ModuleView::Dashboard(dv) => {
            build_dashboard_panel(model, mod_idx, view_key, dv, theme)
        }
        ModuleView::Report(rv) => {
            build_report_panel(model, mod_idx, view_key, rv, theme)
        }
    }
}

/// Vista `List`: filas reales del store con columnas del manifest,
/// búsqueda (`search_in`), orden por columna, paginación, botones
/// editar/borrar/👁 por fila, `+ Nuevo` y export CSV.
fn build_list_panel(model: &Model, mod_idx: usize, lv: &ListView, theme: &Theme) -> View<Msg> {
    let module = &model.modules[mod_idx];
    // Sostenemos el guard durante el armado para resolver las columnas
    // `ref_entity` a su label legible sin re-lockear por celda.
    let guard = model.backend.lock().ok();
    let records = match guard.as_ref() {
        Some(b) => list_filtered_sorted(b, lv, &model.list_search.text(), &model.list_sort),
        None => Vec::new(),
    };

    let total = records.len();
    let has_form = find_form_view(module, &lv.entity).is_some();
    let can_search = !lv.search_in.is_empty();

    // Paginación: clamp de la página contra el total filtrado.
    let pages = total.div_ceil(LIST_PAGE_SIZE).max(1);
    let page = model.list_page.min(pages - 1);

    // --- Fila 1: título + contador + Export + Nuevo. ---
    let title = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("{} · {} ({total})", module.label, lv.title),
        16.0,
        theme.fg_text,
        Alignment::Start,
    );
    let mut header_children = vec![title];
    if total > 0 {
        header_children.push(button_styled(
            "exportar CSV",
            btn_style(120.0),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            Msg::ExportCsv {
                entity: lv.entity.clone(),
            },
        ));
    }
    if has_form {
        header_children.push(button_styled(
            "+ Nuevo",
            btn_style(110.0),
            Alignment::Center,
            &accent_btn(theme),
            Msg::NewRecord {
                module_idx: mod_idx,
                entity: lv.entity.clone(),
            },
        ));
    }
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0),
            height: length(0.0),
        },
        ..Default::default()
    })
    .children(header_children);

    let mut rows: Vec<View<Msg>> = vec![header];

    // --- Caja de búsqueda (sólo si la lista declara search_in). ---
    if can_search {
        rows.push(text_input_view(
            &model.list_search,
            &format!("buscar en {}…", lv.search_in.join(", ")),
            model.list_search_focused,
            &TextInputPalette::from_theme(theme),
            Msg::FocusListSearch,
        ));
    }

    // --- Fila de headers de columna (clickeables para ordenar). ---
    let mut head_cells: Vec<View<Msg>> = vec![cell_text("id".into(), 90.0, theme.fg_muted)];
    for col in &lv.columns {
        let arrow = match &model.list_sort {
            Some((f, true)) if *f == col.field => " ▲",
            Some((f, false)) if *f == col.field => " ▼",
            _ => "",
        };
        head_cells.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(22.0),
                },
                flex_grow: 1.0,
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(
                format!("{}{arrow}", col.label),
                12.0,
                theme.fg_muted,
                Alignment::Start,
            )
            .on_click(Msg::SortBy(col.field.clone())),
        );
    }
    rows.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0),
            },
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(8.0),
                height: length(0.0),
            },
            ..Default::default()
        })
        .children(head_cells),
    );

    if total == 0 {
        let msg = if model.list_search.text().trim().is_empty() {
            "(sin records — usá + Nuevo)"
        } else {
            "(ningún record coincide con la búsqueda)"
        };
        rows.push(text_line(msg.into(), 12.0, theme.fg_muted));
    }

    // --- Filas de la página actual. ---
    for (id, rec) in records
        .iter()
        .skip(page * LIST_PAGE_SIZE)
        .take(LIST_PAGE_SIZE)
    {
        let mut cells: Vec<View<Msg>> = vec![cell_text(short_uuid(id), 90.0, theme.fg_muted)];
        for col in &lv.columns {
            let disp = match guard.as_ref() {
                Some(b) => cell_display(b, col, lookup_field(rec, &col.field)),
                None => render_value(lookup_field(rec, &col.field)),
            };
            cells.push(cell_flex(disp, theme.fg_text));
        }
        if let Some(detail_vk) = &lv.row_detail {
            cells.push(button_styled(
                "👁",
                btn_style(44.0),
                Alignment::Center,
                &ButtonPalette::from_theme(theme),
                Msg::OpenDetail {
                    module_idx: mod_idx,
                    view_key: detail_vk.clone(),
                    entity: lv.entity.clone(),
                    id: *id,
                },
            ));
        }
        if has_form {
            cells.push(button_styled(
                "editar",
                btn_style(70.0),
                Alignment::Center,
                &ButtonPalette::from_theme(theme),
                Msg::EditRecord {
                    module_idx: mod_idx,
                    entity: lv.entity.clone(),
                    id: *id,
                },
            ));
        }
        cells.push(button_styled(
            "borrar",
            btn_style(70.0),
            Alignment::Center,
            &danger_btn(theme),
            Msg::DeleteRecord {
                entity: lv.entity.clone(),
                id: *id,
            },
        ));

        rows.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(30.0),
                },
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(8.0),
                    height: length(0.0),
                },
                ..Default::default()
            })
            .children(cells),
        );
    }

    // --- Controles de paginación (sólo si hay más de una página). ---
    if pages > 1 {
        let prev = button_styled(
            "‹ anterior",
            btn_style(100.0),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            Msg::ListPagePrev,
        );
        let indicator = View::new(Style {
            size: Size {
                width: length(140.0),
                height: length(30.0),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned(
            format!("página {} de {pages}", page + 1),
            12.0,
            theme.fg_muted,
            Alignment::Center,
        );
        let next = button_styled(
            "siguiente ›",
            btn_style(100.0),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            Msg::ListPageNext,
        );
        rows.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(38.0),
                },
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(8.0),
                    height: length(0.0),
                },
                ..Default::default()
            })
            .children(vec![prev, indicator, next]),
        );
    }

    column(rows, 6.0)
}

/// Próximo estado de orden al clickear el header `field`: la misma
/// columna cicla ascendente → descendente → sin orden; otra arranca asc.
fn next_sort(current: Option<(String, bool)>, field: &str) -> Option<(String, bool)> {
    match current {
        Some((f, true)) if f == field => Some((f, false)),
        Some((f, false)) if f == field => None,
        _ => Some((field.to_string(), true)),
    }
}

/// Filas de una lista tras aplicar búsqueda (`search_in`) y orden.
/// Compartido por el render y el export CSV. La búsqueda compara el
/// valor crudo (`render_value`) de cada `search_in` field, sin distinguir
/// mayúsculas.
fn list_filtered_sorted(
    backend: &NakuiBackend,
    lv: &ListView,
    query: &str,
    sort: &Option<(String, bool)>,
) -> Vec<(Uuid, Value)> {
    let mut rows = backend.list_records(&lv.entity);
    let q = query.trim().to_lowercase();
    if !q.is_empty() && !lv.search_in.is_empty() {
        rows.retain(|(_, v)| {
            lv.search_in.iter().any(|field| {
                lookup_field(v, field)
                    .map(|c| render_value(Some(c)).to_lowercase().contains(&q))
                    .unwrap_or(false)
            })
        });
    }
    if let Some((field, asc)) = sort {
        rows.sort_by(|(_, a), (_, b)| {
            let ord = cmp_values(lookup_field(a, field), lookup_field(b, field));
            if *asc {
                ord
            } else {
                ord.reverse()
            }
        });
    }
    rows
}

/// El `ListView` de la vista seleccionada cuya entity coincide.
fn active_list_view<'a>(m: &'a Model, entity: &str) -> Option<&'a ListView> {
    let module = m.modules.get(m.selected_module?)?;
    let item = module.menu.get(m.selected_menu?)?;
    match module.views.get(&item.view) {
        Some(ModuleView::List(lv)) if lv.entity == entity => Some(lv),
        _ => None,
    }
}

/// Ruta destino de un export CSV: `<entity>-<unix-secs>.csv` en el cwd.
/// Exporta un `View::Report` completo a Markdown en el cwd.
fn export_report_md(m: &Model, module_idx: usize, view_key: &str) -> Toast {
    let Some(module) = m.modules.get(module_idx) else {
        return err_toast("módulo fuera de rango");
    };
    let Some(ModuleView::Report(rv)) = module.views.get(view_key) else {
        return err_toast("no encontré el reporte a exportar");
    };
    let md = report_markdown(m, module, rv);
    let path = export_path_ext(&rv.title, "md");
    match std::fs::write(&path, md) {
        Ok(()) => Toast {
            kind: BannerKind::Success,
            text: format!("exporté el reporte a {}", path.display()),
        },
        Err(e) => err_toast(&format!("no pude exportar el reporte: {e}")),
    }
}

/// Exporta el desglose de una card (de un tablero o reporte) a CSV.
fn export_breakdown_csv(
    m: &Model,
    module_idx: usize,
    view_key: &str,
    card_idx: usize,
) -> Toast {
    let Some(module) = m.modules.get(module_idx) else {
        return err_toast("módulo fuera de rango");
    };
    let cards = match module.views.get(view_key) {
        Some(ModuleView::Dashboard(dv)) => &dv.cards,
        Some(ModuleView::Report(rv)) => &rv.cards,
        _ => return err_toast("la vista no tiene tarjetas"),
    };
    let Some(card) = cards.get(card_idx) else {
        return err_toast("tarjeta fuera de rango");
    };
    let result = compute_card_result(m, card);
    let (gh, vh) = breakdown_headers(card);
    let Some(csv) = breakdown_to_csv(&result, &gh, &vh) else {
        return err_toast("esta tarjeta no es un desglose");
    };
    let path = export_path_ext(&card.label, "csv");
    match std::fs::write(&path, csv) {
        Ok(()) => Toast {
            kind: BannerKind::Success,
            text: format!("exporté «{}» a {}", card.label, path.display()),
        },
        Err(e) => err_toast(&format!("no pude exportar CSV: {e}")),
    }
}

/// Encabezados (grupo, valor) del CSV de un desglose, derivados de la
/// métrica de la card.
fn breakdown_headers(card: &DashboardCard) -> (String, String) {
    use nahual_meta_schema::Metric;
    match &card.metric {
        Metric::GroupBy { field } => (field.clone(), "Cantidad".to_string()),
        Metric::SumBy { group, value } => (group.clone(), format!("Suma de {value}")),
        Metric::AvgBy { group, value } => (group.clone(), format!("Promedio de {value}")),
        _ => ("Grupo".to_string(), "Valor".to_string()),
    }
}

fn err_toast(text: &str) -> Toast {
    Toast {
        kind: BannerKind::Error,
        text: text.to_string(),
    }
}

fn export_path(entity: &str) -> std::path::PathBuf {
    export_path_ext(entity, "csv")
}

/// Como [`export_path`] pero con extensión arbitraria. El `stem` se
/// normaliza a kebab seguro para el filesystem.
fn export_path_ext(stem: &str, ext: &str) -> std::path::PathBuf {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let safe: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let name = format!("{safe}-{secs}.{ext}");
    std::env::current_dir()
        .map(|d| d.join(&name))
        .unwrap_or_else(|_| std::path::PathBuf::from(name))
}

/// Exporta la lista activa (filas filtradas/ordenadas, todas las
/// columnas con sus valores renderizados) a un CSV en el cwd; devuelve
/// un toast con el resultado.
fn export_active_list_csv(m: &Model, entity: &str) -> Toast {
    let Some(lv) = active_list_view(m, entity) else {
        return Toast {
            kind: BannerKind::Error,
            text: "no encontré la lista activa para exportar".into(),
        };
    };
    let Ok(backend) = m.backend.lock() else {
        return Toast {
            kind: BannerKind::Error,
            text: "backend lock envenenado".into(),
        };
    };
    let rows = list_filtered_sorted(&backend, lv, &m.list_search.text(), &m.list_sort);
    let headers: Vec<String> = lv.columns.iter().map(|c| c.label.clone()).collect();
    let data: Vec<Vec<String>> = rows
        .iter()
        .map(|(_, v)| {
            lv.columns
                .iter()
                .map(|c| cell_display(&backend, c, lookup_field(v, &c.field)))
                .collect()
        })
        .collect();
    drop(backend);

    let csv = to_csv(&headers, &data);
    let path = export_path(entity);
    match std::fs::write(&path, csv) {
        Ok(()) => Toast {
            kind: BannerKind::Success,
            text: format!("exporté {} fila(s) a {}", rows.len(), path.display()),
        },
        Err(e) => Toast {
            kind: BannerKind::Error,
            text: format!("no pude exportar CSV: {e}"),
        },
    }
}

/// Vista `Detail`: ficha de un record. Header con `← Volver` + `✎
/// Editar`, los campos declarados (label · valor, refs resueltas) y las
/// listas de records relacionados (back-references).
fn build_detail_panel(model: &Model, detail: &DetailState, theme: &Theme) -> View<Msg> {
    let Some(module) = model.modules.get(detail.module_idx) else {
        return empty_panel(theme, "módulo inválido");
    };
    let Some(ModuleView::Detail(dv)) = module.views.get(&detail.view_key) else {
        return empty_panel(theme, "la vista de detalle ya no existe en el manifest");
    };

    // Header: título + Volver + Editar.
    let title = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("{} · {}", module.label, dv.title),
        16.0,
        theme.fg_text,
        Alignment::Start,
    );
    let mut header_children = vec![
        title,
        button_styled(
            "← Volver",
            btn_style(100.0),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            Msg::CloseDetail,
        ),
    ];
    if find_form_view(module, &detail.entity).is_some() {
        header_children.push(button_styled(
            "✎ Editar",
            btn_style(100.0),
            Alignment::Center,
            &accent_btn(theme),
            Msg::EditRecord {
                module_idx: detail.module_idx,
                entity: detail.entity.clone(),
                id: detail.id,
            },
        ));
    }
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(10.0),
            height: length(0.0),
        },
        ..Default::default()
    })
    .children(header_children);

    let mut children: Vec<View<Msg>> = vec![header];

    // El cuerpo necesita el backend; lo sostenemos para el armado.
    let guard = model.backend.lock().ok();
    let record = guard
        .as_ref()
        .and_then(|b| b.load_record(&detail.entity, detail.id));

    let Some(record) = record else {
        children.push(text_line(
            format!("el record {} ya no existe.", short_uuid(&detail.id)),
            12.0,
            theme.fg_muted,
        ));
        return column(children, 8.0);
    };

    // Campos del record (label fijo a la izquierda · valor).
    for col in &dv.fields {
        let value = match guard.as_ref() {
            Some(b) => cell_display(b, col, lookup_field(&record, &col.field)),
            None => render_value(lookup_field(&record, &col.field)),
        };
        let label = cell_text(col.label.clone(), 160.0, theme.fg_muted);
        let val = cell_flex(value, theme.fg_text);
        children.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(26.0),
                },
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(12.0),
                    height: length(0.0),
                },
                ..Default::default()
            })
            .children(vec![label, val]),
        );
    }

    // Listas de records relacionados.
    for rl in &dv.related {
        if let Some(b) = guard.as_ref() {
            children.push(build_related_list(b, rl, detail.id, theme));
        }
    }

    column(children, 8.0)
}

/// Una lista de back-references dentro de una ficha: los records de
/// `rl.entity` cuyo `rl.via_field` apunta al record `target_id`.
fn build_related_list(
    backend: &NakuiBackend,
    rl: &RelatedList,
    target_id: Uuid,
    theme: &Theme,
) -> View<Msg> {
    let id_str = target_id.to_string();
    let rows: Vec<(Uuid, Value)> = backend
        .list_records(&rl.entity)
        .into_iter()
        .filter(|(_, v)| v.get(&rl.via_field).and_then(Value::as_str) == Some(id_str.as_str()))
        .collect();

    let mut children: Vec<View<Msg>> = vec![text_line(
        format!("{} ({})", rl.title, rows.len()),
        13.0,
        theme.fg_text,
    )];

    if rows.is_empty() {
        children.push(text_line("(ninguno)".into(), 11.0, theme.fg_muted));
    } else {
        // Header de columnas.
        let head_cells: Vec<View<Msg>> = rl
            .columns
            .iter()
            .map(|c| cell_flex(c.label.clone(), theme.fg_muted))
            .collect();
        children.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0),
                },
                gap: Size {
                    width: length(8.0),
                    height: length(0.0),
                },
                ..Default::default()
            })
            .children(head_cells),
        );

        for (_, v) in &rows {
            let cells: Vec<View<Msg>> = rl
                .columns
                .iter()
                .map(|c| {
                    cell_flex(cell_display(backend, c, lookup_field(v, &c.field)), theme.fg_text)
                })
                .collect();
            children.push(
                View::new(Style {
                    flex_direction: FlexDirection::Row,
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(22.0),
                    },
                    gap: Size {
                        width: length(8.0),
                        height: length(0.0),
                    },
                    ..Default::default()
                })
                .children(cells),
            );
        }
    }

    // Bloque que se ajusta al contenido, con un poco de aire arriba.
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_shrink: 0.0,
        margin: Rect {
            left: length(0.0),
            right: length(0.0),
            top: length(10.0),
            bottom: length(0.0),
        },
        gap: Size {
            width: length(0.0),
            height: length(4.0),
        },
        ..Default::default()
    })
    .children(children)
}

/// Resuelve las claves de un desglose (UUIDs) al label legible del
/// record referido en `ref_entity`. Las claves que no son UUID se
/// dejan tal cual; los records borrados se marcan como tales. Mismo
/// criterio que [`cell_display`] para columnas `ref_entity`.
fn resolve_breakdown_keys(
    result: &mut MetricResult,
    backend: &NakuiBackend,
    ref_entity: &str,
) {
    let resolve = |key: &str| -> String {
        match Uuid::parse_str(key) {
            Ok(uuid) => backend
                .load_record(ref_entity, uuid)
                .map(|rec| human_label_for_record(&rec, &uuid))
                .unwrap_or_else(|| format!("(borrado · {})", short_uuid(&uuid))),
            Err(_) => key.to_string(),
        }
    };
    match result {
        MetricResult::Breakdown(rows) => {
            for (k, _) in rows.iter_mut() {
                *k = resolve(k);
            }
        }
        MetricResult::ValueBreakdown(rows) => {
            for (k, _) in rows.iter_mut() {
                *k = resolve(k);
            }
        }
        MetricResult::Scalar(_) => {}
    }
}

/// Computa el agregado de una card resolviendo `group_ref` si lo hay.
/// Toma el lock del backend por card — el tablero no es ruta caliente.
fn compute_card_result(model: &Model, card: &DashboardCard) -> MetricResult {
    let guard = model.backend.lock().ok();
    let records = guard
        .as_ref()
        .map(|b| b.list_records(&card.entity))
        .unwrap_or_default();
    let mut result = compute_metric(&card.metric, card.filter.as_ref(), &records);
    if let (Some(ref_entity), Some(backend)) = (&card.group_ref, guard.as_ref()) {
        resolve_breakdown_keys(&mut result, backend, ref_entity);
    }
    result
}

/// `true` si el resultado es un desglose (exportable a CSV).
fn is_breakdown(r: &MetricResult) -> bool {
    matches!(
        r,
        MetricResult::Breakdown(_) | MetricResult::ValueBreakdown(_)
    )
}

/// Vista `Dashboard`: una grilla de tarjetas de KPI, cada una con su
/// agregado (`Count`/`Sum`/`Avg`/`Min`/`Max`/`GroupBy`/`SumBy`/`AvgBy`)
/// computado sobre los records de su entity.
fn build_dashboard_panel(
    model: &Model,
    mod_idx: usize,
    view_key: &str,
    dv: &DashboardView,
    theme: &Theme,
) -> View<Msg> {
    let module = &model.modules[mod_idx];
    let title = text_line(
        format!("{} · {}", module.label, dv.title),
        16.0,
        theme.fg_text,
    );

    let mut cards: Vec<View<Msg>> = Vec::new();
    for (i, card) in dv.cards.iter().enumerate() {
        let result = compute_card_result(model, card);
        // Las cards con desglose ganan un botón de export CSV.
        let on_export = if is_breakdown(&result) {
            Some(Msg::ExportBreakdownCsv {
                module_idx: mod_idx,
                view_key: view_key.to_string(),
                card_idx: i,
            })
        } else {
            None
        };
        cards.push(dashboard_card(&card.label, &result, &card.format, on_export, theme));
    }

    let grid = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        align_content: Some(llimphi_ui::llimphi_layout::taffy::AlignContent::Start),
        gap: Size {
            width: length(12.0),
            height: length(12.0),
        },
        ..Default::default()
    })
    .children(cards);

    column(vec![title, grid], 12.0)
}

/// Una tarjeta del tablero: label + número grande (Scalar) o barras de
/// breakdown (GroupBy).
fn dashboard_card(
    label: &str,
    result: &MetricResult,
    fmt: &ValueFormat,
    on_export: Option<Msg>,
    theme: &Theme,
) -> View<Msg> {
    let mut children: Vec<View<Msg>> = vec![text_line(label.to_string(), 11.0, theme.fg_muted)];

    match result {
        MetricResult::Scalar(s) => {
            // Entero si no tiene parte decimal (Count / sumas enteras).
            let value = if s.fract() == 0.0 {
                Value::from(*s as i64)
            } else {
                Value::from(*s)
            };
            children.push(
                View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(34.0),
                    },
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                })
                .text_aligned(
                    format_value(Some(&value), fmt),
                    26.0,
                    theme.accent,
                    Alignment::Start,
                ),
            );
        }
        MetricResult::Breakdown(rows) => {
            if rows.is_empty() {
                children.push(text_line("(sin datos)".into(), 11.0, theme.fg_muted));
            }
            let max = rows.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1);
            for (key, n) in rows {
                let bar = "█".repeat((n * 12 / max).max(1));
                let row = View::new(Style {
                    flex_direction: FlexDirection::Row,
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(18.0),
                    },
                    align_items: Some(AlignItems::Center),
                    gap: Size {
                        width: length(6.0),
                        height: length(0.0),
                    },
                    ..Default::default()
                })
                .children(vec![
                    cell_text(key.clone(), 96.0, theme.fg_text),
                    cell_flex(bar, theme.accent),
                    cell_text(n.to_string(), 32.0, theme.fg_muted),
                ]);
                children.push(row);
            }
        }
        MetricResult::ValueBreakdown(rows) => {
            if rows.is_empty() {
                children.push(text_line("(sin datos)".into(), 11.0, theme.fg_muted));
            }
            // La barra escala contra el mayor valor absoluto; el número
            // se formatea con el `ValueFormat` de la tarjeta (moneda).
            let max = rows
                .iter()
                .map(|(_, v)| v.abs())
                .fold(0.0_f64, f64::max)
                .max(1.0);
            for (key, v) in rows {
                let filled = ((v.abs() / max) * 12.0).round() as usize;
                let bar = "█".repeat(filled.max(1));
                let value = if v.fract() == 0.0 {
                    Value::from(*v as i64)
                } else {
                    Value::from(*v)
                };
                let row = View::new(Style {
                    flex_direction: FlexDirection::Row,
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(18.0),
                    },
                    align_items: Some(AlignItems::Center),
                    gap: Size {
                        width: length(6.0),
                        height: length(0.0),
                    },
                    ..Default::default()
                })
                .children(vec![
                    cell_text(key.clone(), 96.0, theme.fg_text),
                    cell_flex(bar, theme.accent),
                    cell_text(format_value(Some(&value), fmt), 64.0, theme.fg_muted),
                ]);
                children.push(row);
            }
        }
    }

    // Botón de export CSV para los desgloses.
    if let Some(msg) = on_export {
        children.push(button_styled(
            "⤓ CSV",
            btn_style_auto(),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            msg,
        ));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(220.0),
            height: auto(),
        },
        flex_grow: 0.0,
        flex_shrink: 0.0,
        padding: Rect {
            left: length(14.0),
            right: length(14.0),
            top: length(12.0),
            bottom: length(12.0),
        },
        gap: Size {
            width: length(0.0),
            height: length(6.0),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(8.0)
    .children(children)
}

/// Vista `Report`: los mismos agregados que un tablero, dispuestos
/// como documento de una columna (título + subtítulo) con un botón
/// "Exportar (.md)" que vuelca el reporte completo a Markdown.
fn build_report_panel(
    model: &Model,
    mod_idx: usize,
    view_key: &str,
    rv: &ReportView,
    theme: &Theme,
) -> View<Msg> {
    let module = &model.modules[mod_idx];
    let mut children: Vec<View<Msg>> = Vec::new();

    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        ..Default::default()
    })
    .children(vec![
        text_line(format!("{} · {}", module.label, rv.title), 16.0, theme.fg_text),
        button_styled(
            "⤓ Exportar (.md)",
            btn_style(150.0),
            Alignment::Center,
            &accent_btn(theme),
            Msg::ExportReport {
                module_idx: mod_idx,
                view_key: view_key.to_string(),
            },
        ),
    ]);
    children.push(header);
    if let Some(sub) = &rv.subtitle {
        children.push(text_line(sub.clone(), 12.0, theme.fg_muted));
    }

    // Una card por agregado, apiladas en columna (documento).
    for (i, card) in rv.cards.iter().enumerate() {
        let result = compute_card_result(model, card);
        let on_export = if is_breakdown(&result) {
            Some(Msg::ExportBreakdownCsv {
                module_idx: mod_idx,
                view_key: view_key.to_string(),
                card_idx: i,
            })
        } else {
            None
        };
        children.push(dashboard_card(&card.label, &result, &card.format, on_export, theme));
    }

    column(children, 12.0)
}

/// Serializa un reporte completo a Markdown: título, subtítulo, y una
/// sección por card (escalar en negrita o tabla de desglose).
fn report_markdown(model: &Model, module: &Module, rv: &ReportView) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {} · {}\n\n", module.label, rv.title));
    if let Some(sub) = &rv.subtitle {
        out.push_str(&format!("_{sub}_\n\n"));
    }
    out.push_str("Generado por nakui.\n\n");
    for card in &rv.cards {
        let result = compute_card_result(model, card);
        out.push_str(&format!("## {}\n\n", card.label));
        match &result {
            MetricResult::Scalar(s) => {
                let value = if s.fract() == 0.0 {
                    Value::from(*s as i64)
                } else {
                    Value::from(*s)
                };
                out.push_str(&format!("**{}**\n\n", format_value(Some(&value), &card.format)));
            }
            MetricResult::Breakdown(rows) => {
                out.push_str("| Grupo | Cantidad |\n|---|---:|\n");
                for (k, n) in rows {
                    out.push_str(&format!("| {} | {} |\n", md_escape(k), n));
                }
                out.push('\n');
            }
            MetricResult::ValueBreakdown(rows) => {
                out.push_str("| Grupo | Valor |\n|---|---:|\n");
                for (k, v) in rows {
                    let value = if v.fract() == 0.0 {
                        Value::from(*v as i64)
                    } else {
                        Value::from(*v)
                    };
                    out.push_str(&format!(
                        "| {} | {} |\n",
                        md_escape(k),
                        format_value(Some(&value), &card.format)
                    ));
                }
                out.push('\n');
            }
        }
    }
    out
}

/// Escapa los `|` de una celda de tabla Markdown.
fn md_escape(s: &str) -> String {
    s.replace('|', "\\|")
}

/// Render del valor de una celda. Una columna con `ref_entity` resuelve
/// su UUID al label del record referido; el resto aplica el
/// `ValueFormat` de la columna. Espejo del `render_cell` GPUI.
fn cell_display(backend: &NakuiBackend, col: &Column, v: Option<&Value>) -> String {
    if let Some(ref_entity) = &col.ref_entity {
        return match v {
            Some(Value::String(s)) => match Uuid::parse_str(s) {
                Ok(uuid) => backend
                    .load_record(ref_entity, uuid)
                    .map(|rec| human_label_for_record(&rec, &uuid))
                    .unwrap_or_else(|| format!("(borrado · {})", short_uuid(&uuid))),
                Err(_) => render_value(v),
            },
            _ => render_value(v),
        };
    }
    format_value(v, &col.format)
}

/// Navega un path con puntos (`address.city`) dentro de un `Value`.
fn lookup_field<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// Panel del formulario activo: un `field_view` por field + fila de
/// acciones (Cancelar / Guardar) + banner de error.
fn build_form_panel(model: &Model, form: &FormState, theme: &Theme) -> View<Msg> {
    let module = model.modules.get(form.module_idx);
    let module_label = module.map(|m| m.label.as_str()).unwrap_or("");
    let mode = if form.editing.is_some() {
        "editar"
    } else {
        "nuevo"
    };
    let title = text_line(
        format!("{module_label} · {} ({mode})", form.title),
        16.0,
        theme.fg_text,
    );

    let field_palette = FieldPalette::from_theme(theme);
    let input_palette = TextInputPalette::from_theme(theme);

    let mut children: Vec<View<Msg>> = vec![title];

    for (i, fr) in form.fields.iter().enumerate() {
        let focused = form.focused == Some(i);
        let control = build_field_control(model, fr, i, focused, &input_palette, theme);
        children.push(field_view(FieldWidgetSpec {
            label: fr.spec.label.clone(),
            control,
            required: fr.spec.required,
            helper: fr.spec.help.clone(),
            error: None,
            palette: field_palette,
        }));
    }

    if let Some(err) = &form.error {
        children.push(banner_view::<Msg>(BannerKind::Error, err.clone()));
    }

    // Fila de acciones.
    let actions = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(38.0),
        },
        gap: Size {
            width: length(10.0),
            height: length(0.0),
        },
        ..Default::default()
    })
    .children(vec![
        button_styled(
            "Cancelar",
            btn_style(120.0),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            Msg::CancelForm,
        ),
        button_styled(
            if form.editing.is_some() {
                "Guardar"
            } else {
                "Crear"
            },
            btn_style(120.0),
            Alignment::Center,
            &accent_btn(theme),
            Msg::SubmitForm,
        ),
    ]);
    children.push(actions);

    column(children, 10.0)
}

/// Renderea el control de un field según su `FieldKind`.
fn build_field_control(
    model: &Model,
    fr: &FieldRuntime,
    i: usize,
    focused: bool,
    input_palette: &TextInputPalette,
    theme: &Theme,
) -> View<Msg> {
    match fr.spec.kind {
        FieldKind::Text | FieldKind::Multiline | FieldKind::Number | FieldKind::Date => {
            let placeholder = fr.spec.help.clone().unwrap_or_default();
            text_input_view(
                &fr.input,
                &placeholder,
                focused,
                input_palette,
                Msg::FocusField(i),
            )
        }
        FieldKind::Boolean => {
            let on = fr.raw() == "true";
            let pal = if on {
                accent_btn(theme)
            } else {
                ButtonPalette::from_theme(theme)
            };
            button_styled(
                if on { "Sí" } else { "No" },
                btn_style(80.0),
                Alignment::Center,
                &pal,
                Msg::ToggleBool(i),
            )
        }
        FieldKind::AutoId => {
            // Read-only: el UUID autogenerado, sin foco.
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(28.0),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(fr.raw(), 12.0, theme.fg_muted, Alignment::Start)
        }
        FieldKind::Select => {
            let current = fr.raw();
            let chips: Vec<View<Msg>> = fr
                .spec
                .options
                .iter()
                .map(|opt| {
                    let selected = current == opt.value;
                    let pal = if selected {
                        accent_btn(theme)
                    } else {
                        ButtonPalette::from_theme(theme)
                    };
                    button_styled(
                        opt.display().to_string(),
                        btn_style_auto(),
                        Alignment::Center,
                        &pal,
                        Msg::SetSelect(i, opt.value.clone()),
                    )
                })
                .collect();
            chip_row(chips)
        }
        FieldKind::EntityRef => {
            let target = fr.spec.ref_entity.clone().unwrap_or_default();
            let current = fr.raw();
            let records = model
                .backend
                .lock()
                .map(|b| b.list_records(&target))
                .unwrap_or_default();
            let total = records.len();
            let mut chips: Vec<View<Msg>> = records
                .iter()
                .take(ENTITY_REF_LIMIT)
                .map(|(id, rec)| {
                    let id_str = id.to_string();
                    let selected = current == id_str;
                    let label = entity_ref_label(id, rec);
                    let pal = if selected {
                        accent_btn(theme)
                    } else {
                        ButtonPalette::from_theme(theme)
                    };
                    button_styled(
                        label,
                        btn_style_auto(),
                        Alignment::Center,
                        &pal,
                        Msg::SetSelect(i, id_str),
                    )
                })
                .collect();
            if total == 0 {
                chips.push(cell_text(
                    format!("(sin records en '{target}')"),
                    240.0,
                    theme.fg_muted,
                ));
            } else if total > ENTITY_REF_LIMIT {
                chips.push(cell_text(
                    format!("… +{} más", total - ENTITY_REF_LIMIT),
                    120.0,
                    theme.fg_muted,
                ));
            }
            chip_row(chips)
        }
    }
}

/// Etiqueta de un record en un selector EntityRef: id corto + preview
/// del primer campo string del record (si lo hay).
fn entity_ref_label(id: &Uuid, rec: &Value) -> String {
    let preview = rec.as_object().and_then(|m| {
        m.values()
            .find_map(|v| v.as_str().map(|s| s.to_string()))
    });
    match preview {
        Some(name) => format!("{} · {}", short_uuid(id), preview_value(&Value::String(name), 24)),
        None => short_uuid(id),
    }
}

// ----- helpers de layout -----

fn column(children: Vec<View<Msg>>, gap: f32) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(gap),
        },
        ..Default::default()
    })
    .children(children)
}

fn chip_row(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0),
        },
        gap: Size {
            width: length(6.0),
            height: length(6.0),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(children)
}

fn placeholder_panel(
    module: &Module,
    title: &str,
    body_lines: Vec<String>,
    theme: &Theme,
) -> View<Msg> {
    let mut children: Vec<View<Msg>> = vec![text_line(
        format!("{} · {}", module.label, title),
        16.0,
        theme.fg_text,
    )];
    if let Some(desc) = &module.description {
        children.push(text_line(desc.clone(), 11.0, theme.fg_muted));
    }
    for line in body_lines {
        children.push(text_line(line, 12.0, theme.fg_text));
    }
    column(children, 6.0)
}

fn empty_panel(theme: &Theme, msg: &str) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(msg.to_string(), 12.0, theme.fg_muted, Alignment::Start)
}

fn text_line(content: String, size_px: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(size_px + 8.0),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(content, size_px, color, Alignment::Start)
}

/// Celda de ancho fijo (px) para columnas tipo id/acción.
fn cell_text(content: String, width: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(width),
            height: length(24.0),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(content, 12.0, color, Alignment::Start)
}

/// Celda elástica para columnas de datos.
fn cell_flex(content: String, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(content, 12.0, color, Alignment::Start)
}

/// Style de botón de ancho fijo.
fn btn_style(width: f32) -> Style {
    Style {
        size: Size {
            width: length(width),
            height: length(30.0),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(10.0),
            right: length(10.0),
            top: length(4.0),
            bottom: length(4.0),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    }
}

/// Style de botón que se ajusta al contenido (chips de select/ref).
fn btn_style_auto() -> Style {
    Style {
        size: Size {
            width: length(140.0),
            height: length(26.0),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(8.0),
            right: length(8.0),
            top: length(2.0),
            bottom: length(2.0),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    }
}

/// Paleta de botón con acento (acción primaria / selección activa).
fn accent_btn(theme: &Theme) -> ButtonPalette {
    let mut p = ButtonPalette::from_theme(theme);
    p.bg = theme.accent;
    p.bg_hover = theme.accent;
    p.fg = theme.bg_app;
    p
}

/// Paleta de botón destructivo (borrar).
fn danger_btn(theme: &Theme) -> ButtonPalette {
    let mut p = ButtonPalette::from_theme(theme);
    p.bg = theme.fg_destructive;
    p.bg_hover = theme.fg_destructive;
    p.fg = theme.bg_app;
    p
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
        assert_eq!(modules.len(), 1);
        let m = &modules[0];
        assert_eq!(m.id, "ventas");
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
}
