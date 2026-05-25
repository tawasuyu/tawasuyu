//! `nakui-ui-llimphi` — binario shell de la metainterfaz Nakui sobre
//! Llimphi.
//!
//! ## Estado actual (MVP)
//!
//! - Carga módulos UI desde `NAKUI_MODULES_DIR` (o `./nakui-modules`)
//!   vía `cards::load_cards_from_dir`.
//! - Crea `NakuiBackend` (event log persistente + replay + snapshot +
//!   auto-compact). El backend implementa `nahual_meta_runtime::MetaBackend`
//!   completo (seed/update/delete/morphism), igual que la versión GPUI.
//! - Llimphi shell: sidebar de módulos (clickeable) + menú del módulo
//!   activo + área principal con la vista seleccionada. Por ahora la
//!   vista muestra **read-only**: para una `List` se enumera la cantidad
//!   de records de la entity correspondiente; para `Form`/`Detail`/
//!   `Dashboard` se muestra el kind y el aviso de que el meta-form
//!   widget Llimphi todavía no existe.
//!
//! Falta: el widget Llimphi paralelo a `nahual-widget-meta-form` (2k
//! LOC) que renderea los formularios de seed/edit y dispara las
//! acciones `Morphism` / `OpenView` del manifest. Hasta entonces la
//! mutación pasa por los binarios CLI / tests del backend; la UI es
//! exploratoria.
//!
//! ## Uso
//!
//! ```sh
//! NAKUI_MODULES_DIR=examples/nakui-modules cargo run -p nakui-ui-llimphi
//! # default sin env: ./nakui-modules en pwd.
//! ```

mod backend;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use cards::CardBody;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};

use nahual_meta_runtime::MetaBackend;
use nahual_meta_schema::{Module, View as ModuleView};
use nakui_core::executor::Executor;

use crate::backend::NakuiBackend;

const SIDEBAR_WIDTH: f32 = 240.0;
const ROW_HEIGHT: f32 = 22.0;

#[derive(Clone)]
enum Msg {
    SelectModule(usize),
    SelectMenu(usize),
}

struct Model {
    modules: Vec<Module>,
    backend: Arc<Mutex<NakuiBackend>>,
    initial_toast: Option<String>,
    load_error: Option<String>,
    selected_module: Option<usize>,
    selected_menu: Option<usize>,
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
        let selected_menu = selected_module
            .and_then(|i| (!modules[i].menu.is_empty()).then_some(0));

        Model {
            modules,
            backend: Arc::new(Mutex::new(backend)),
            initial_toast,
            load_error,
            selected_module,
            selected_menu,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::SelectModule(i) => {
                if i < m.modules.len() {
                    m.selected_module = Some(i);
                    m.selected_menu = (!m.modules[i].menu.is_empty()).then_some(0);
                }
            }
            Msg::SelectMenu(i) => {
                if let Some(mod_idx) = m.selected_module {
                    if i < m.modules[mod_idx].menu.len() {
                        m.selected_menu = Some(i);
                    }
                }
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let header = app_header::<Msg>(
            format!("Nakui · {} módulo(s)", model.modules.len()),
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

fn build_banners(model: &Model) -> Vec<View<Msg>> {
    let mut out: Vec<View<Msg>> = Vec::new();
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
        caption: Some(format!("Módulos ({})", model.modules.len())),
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
                caption: Some("Menú".into()),
                truncated_hint: None,
                row_height: ROW_HEIGHT,
                palette,
            })
        }
        None => empty_panel(theme, "Sin módulos cargados"),
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
    let inner = match (model.selected_module, model.selected_menu) {
        (Some(mod_idx), Some(menu_idx)) => {
            let m = &model.modules[mod_idx];
            let item = &m.menu[menu_idx];
            match m.views.get(&item.view) {
                Some(view) => build_view_panel(model, m, view, theme),
                None => empty_panel(
                    theme,
                    &format!(
                        "vista '{}' no existe en el manifest del módulo",
                        item.view
                    ),
                ),
            }
        }
        (Some(_), None) => empty_panel(theme, "Elegí un menú en la barra lateral"),
        _ => empty_panel(theme, "Elegí un módulo en la barra lateral"),
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
    module: &Module,
    view: &ModuleView,
    theme: &Theme,
) -> View<Msg> {
    let (title, body_lines) = match view {
        ModuleView::List(lv) => {
            let count = model
                .backend
                .lock()
                .map(|b| b.list_records(&lv.entity).len())
                .unwrap_or(0);
            let lines = vec![
                format!("kind: list · entity: {}", lv.entity),
                format!("columns: {}", lv.columns.len()),
                format!("records en store: {count}"),
                format!("acciones: {}", lv.actions.len()),
            ];
            (lv.title.clone(), lines)
        }
        ModuleView::Form(fv) => {
            let lines = vec![
                format!("kind: form · entity: {}", fv.entity),
                format!("fields: {}", fv.fields.len()),
                "edición pendiente: requiere meta-form Llimphi".into(),
            ];
            (fv.title.clone(), lines)
        }
        ModuleView::Detail(dv) => {
            let lines = vec![
                format!("kind: detail · entity: {}", dv.entity),
                "render pendiente: requiere meta-form Llimphi".into(),
            ];
            (dv.title.clone(), lines)
        }
        ModuleView::Dashboard(d) => {
            let lines = vec![
                "kind: dashboard".into(),
                format!("cards: {}", d.cards.len()),
                "render pendiente: requiere dashboard Llimphi".into(),
            ];
            (d.title.clone(), lines)
        }
    };

    let header_line = text_line(
        format!("{} · {}", module.label, title),
        16.0,
        theme.fg_text,
    );
    let mut children: Vec<View<Msg>> = vec![header_line];
    if let Some(desc) = &module.description {
        children.push(text_line(desc.clone(), 11.0, theme.fg_muted));
    }
    for line in body_lines {
        children.push(text_line(line, 12.0, theme.fg_text));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .children(children)
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
}
