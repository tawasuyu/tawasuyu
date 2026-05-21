//! `nakui-ui` — binario shell de la metainterfaz Nakui.
//!
//! Compone:
//! - **Yahweh widget** [`nahual_widget_meta_form::MetaApp`] genérico
//!   sobre cualquier `MetaBackend` — toda la lógica de
//!   render/edit/delete/morphism vive ahí.
//! - **Backend** [`backend::NakuiBackend`] — implementa el trait
//!   wireado al stack nakui-core (event log + MemoryStore + Rhai
//!   executors).
//! - **Loader** [`load_ui_modules`] — usa `brahman_cards` para leer
//!   `card.{ncl,json}` / `module.{ncl,json}` desde
//!   `NAKUI_MODULES_DIR`, filtra a UiModule body, valida.
//!
//! ## Uso
//!
//! ```sh
//! NAKUI_MODULES_DIR=examples/nakui-modules cargo run -p nakui-ui
//! # default sin env: ./nakui-modules en pwd.
//! ```

mod backend;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    prelude::*, px, App, Application, Bounds, SharedString, TitlebarOptions, WindowBounds,
    WindowOptions,
};

use brahman_cards::CardBody;
use nahual_meta_schema::Module;
use nahual_theme::Theme;
use nahual_widget_meta_form::MetaApp;
use nakui_core::executor::Executor;

use crate::backend::NakuiBackend;

fn main() {
    Application::new().run(|cx: &mut App| {
        // El text input pide Theme::global; instalarlo antes de
        // crear el window evita que panicee.
        Theme::install_default(cx);

        // 1. Cargar módulos (Cards UiModule via brahman_cards).
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
        //    Path resuelve relativo al subdir del módulo.
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

        // 4. Abrir window con MetaApp<NakuiBackend> como root view.
        let bounds = Bounds::centered(None, gpui::size(px(1100.), px(720.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("Nakui")),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_w, cx| cx.new(|cx| MetaApp::new(modules, backend, initial_toast, load_error, cx)),
        )
        .expect("open window");
        cx.activate(true);
    });
}

/// Carga UiModules desde un directorio via el brazo unificado
/// `brahman_cards::load_cards_from_dir`. Aplica las reglas
/// específicas de la UI:
///  - Sólo `CardBody::UiModule` cuenta; otros body kinds
///    (Ente, Monad, ...) se reportan en el `skipped` para que el
///    runtime los muestre como banner informativo.
///  - Cada `Module` se valida via `Module::validate()`.
///  - Detecta `id` duplicados entre módulos UiModule (el runtime
///    los direcciona por id; duplicados serían ambiguos).
///
/// Devuelve `(modules, skipped_ids)` ordenados por id.
fn load_ui_modules(dir: &std::path::Path) -> Result<(Vec<Module>, Vec<String>), String> {
    let cards = brahman_cards::load_cards_from_dir(dir).map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    //! Tests del shell. Los tests del backend impl viven en
    //! `backend.rs`. Los tests del widget viven en
    //! `nahual-widget-meta-form`. Los helpers puros en
    //! `nahual-meta-runtime`.

    use super::*;
    use serde_json::json;

    /// E2E mínimo del WAL: armamos un log a mano con dos seeds,
    /// abrimos con `EventLog::open` + `replay_into`, y verificamos
    /// que el `MemoryStore` queda con esos records aplicados.
    /// Reproduce el flujo del startup de NakuiBackend sin GPUI.
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

    /// E2E del Action::Morphism: carga el módulo nakui-core real
    /// `sales`, arma store + log, y ejecuta el morphism `vender` vía
    /// `execute_and_log_with_recovery` (la función que usa
    /// `NakuiBackend::morphism` internamente). Verifica las
    /// post-condiciones esperadas del manifest sales.
    #[test]
    fn morphism_pipeline_executes_real_sales_vender() {
        use nakui_core::event_log::{execute_and_log_with_recovery, EventLog};
        use nakui_core::executor::Executor;
        use nakui_core::store::{MemoryStore, Store};
        use uuid::Uuid;

        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let sales_dir = here
            .join("../../..")
            .join("crates/modules/nakui/modules/sales");
        if !sales_dir.join("nsmc.json").exists() {
            eprintln!(
                "skip: sales module no encontrado en {}",
                sales_dir.display()
            );
            return;
        }

        let executor = Executor::load_module(&sales_dir).expect("cargar sales executor");

        let mut store = MemoryStore::new();
        let stock_id = Uuid::new_v4();
        let caja_id = Uuid::new_v4();
        store.seed(
            "Stock",
            stock_id,
            json!({
                "id": stock_id.to_string(),
                "sku_id": "test-sku",
                "ubicacion": "loc-1",
                "cantidad": 100_i64,
            }),
        );
        store.seed(
            "Caja",
            caja_id,
            json!({
                "id": caja_id.to_string(),
                "name": "Caja Test",
                "currency": "USD",
                "saldo": 1_000_000_i64,
            }),
        );

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let log_path = tmp.path().to_path_buf();
        drop(tmp);
        let mut log = EventLog::open(&log_path).unwrap();

        let venta_id = Uuid::new_v4();
        let inputs = vec![("stock", stock_id), ("caja", caja_id)];
        let params = json!({
            "venta_id": venta_id.to_string(),
            "cantidad": 5_i64,
            "precio_unitario": 200_i64,
            "timestamp": "2026-05-04T10:00:00Z",
        });

        let ops = execute_and_log_with_recovery(
            &executor, &mut store, &mut log, "vender", &inputs, params,
        )
        .expect("morphism vender debe ejecutar limpio");

        assert!(!ops.is_empty());
        let stock_after = store
            .load("Stock", stock_id)
            .and_then(|v| v.get("cantidad").and_then(|c| c.as_i64()))
            .expect("stock con cantidad");
        assert_eq!(stock_after, 95);
        let caja_after = store
            .load("Caja", caja_id)
            .and_then(|v| v.get("saldo").and_then(|s| s.as_i64()))
            .expect("caja con saldo");
        assert_eq!(caja_after, 1_001_000);

        let _ = std::fs::remove_file(&log_path);
    }

    #[test]
    fn load_ui_modules_via_brahman_cards_returns_ui_modules_and_skips_others() {
        let root = tempfile::tempdir().unwrap();

        let a = root.path().join("alpha");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(
            a.join("module.json"),
            serde_json::to_vec(&json!({
                "id": "alpha",
                "label": "Alpha",
                "entities": [],
                "menu": [],
                "views": {}
            }))
            .unwrap(),
        )
        .unwrap();

        let b = root.path().join("bravo");
        std::fs::create_dir(&b).unwrap();
        std::fs::write(
            b.join("card.json"),
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                "label": "ente-bravo",
                "payload": "Virtual",
                "supervision": "OneShot"
            }))
            .unwrap(),
        )
        .unwrap();

        let (modules, skipped) = load_ui_modules(root.path()).expect("load ok");
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].id, "alpha");
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("ente"));
    }

    #[test]
    fn load_ui_modules_via_brahman_cards_rejects_invalid_module() {
        let root = tempfile::tempdir().unwrap();
        let sub = root.path().join("broken");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(
            sub.join("module.json"),
            serde_json::to_vec(&json!({
                "id": "broken",
                "label": "Broken",
                "entities": [],
                "menu": [{ "label": "Phantom", "view": "ghost" }],
                "views": {}
            }))
            .unwrap(),
        )
        .unwrap();
        let err = load_ui_modules(root.path()).unwrap_err();
        assert!(err.contains("broken"), "msg debe nombrar el módulo: {err}");
    }

    #[test]
    fn load_ui_modules_detects_duplicate_id() {
        let root = tempfile::tempdir().unwrap();
        for name in ["dir_a", "dir_b"] {
            let sub = root.path().join(name);
            std::fs::create_dir(&sub).unwrap();
            std::fs::write(
                sub.join("module.json"),
                serde_json::to_vec(&json!({
                    "id": "dup",
                    "label": "Dup",
                    "entities": [], "menu": [], "views": {}
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let err = load_ui_modules(root.path()).unwrap_err();
        assert!(err.contains("duplicado"));
        assert!(err.contains("dup"));
    }

    /// El UiModule del CRM (`examples/nakui-modules/crm`) debe parsear
    /// como `Module` y pasar `validate()` — sino `nakui-ui` lo rechaza
    /// al arrancar. Cubre que las 7 vistas del ERP existan y que
    /// enganche el módulo-kernel.
    #[test]
    fn crm_example_module_parses_and_validates() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../examples/nakui-modules/crm/module.json");
        let m = Module::from_path(&path).expect("crm/module.json debe parsear");
        m.validate().expect("el módulo crm debe validar");

        assert_eq!(m.id, "crm");
        assert!(
            m.nakui_module_dir.is_some(),
            "el CRM debe enganchar el módulo-kernel"
        );
        for view in [
            "cliente_list",
            "cliente_form",
            "oportunidad_list",
            "abrir_form",
            "mover_form",
            "interaccion_list",
            "interaccion_form",
            "cliente_detail",
            "oportunidad_detail",
        ] {
            assert!(m.views.contains_key(view), "falta la vista «{view}»");
        }

        // Fase 2: la lista de oportunidades resuelve `cliente_id` al
        // label del cliente y formatea `monto` como moneda.
        let nahual_meta_schema::View::List(lv) = &m.views["oportunidad_list"] else {
            panic!("oportunidad_list debe ser una lista");
        };
        let cliente_col = lv
            .columns
            .iter()
            .find(|c| c.field == "cliente_id")
            .expect("columna cliente_id");
        assert_eq!(cliente_col.ref_entity.as_deref(), Some("Cliente"));
        let monto_col = lv
            .columns
            .iter()
            .find(|c| c.field == "monto")
            .expect("columna monto");
        assert!(
            matches!(
                monto_col.format,
                nahual_meta_schema::ValueFormat::Currency { .. }
            ),
            "monto debe formatearse como moneda",
        );
        assert_eq!(
            lv.row_detail.as_deref(),
            Some("oportunidad_detail"),
            "la fila de oportunidad debe abrir su ficha",
        );

        // Fase 3: la ficha del cliente lista sus oportunidades e
        // interacciones (back-references).
        let nahual_meta_schema::View::Detail(dv) = &m.views["cliente_detail"] else {
            panic!("cliente_detail debe ser una ficha (detail)");
        };
        assert_eq!(dv.entity, "Cliente");
        let related: Vec<&str> = dv.related.iter().map(|r| r.entity.as_str()).collect();
        assert!(
            related.contains(&"Oportunidad"),
            "ficha cliente: falta Oportunidad"
        );
        assert!(
            related.contains(&"Interaccion"),
            "ficha cliente: falta Interaccion"
        );
        for r in &dv.related {
            assert_eq!(r.via_field, "cliente_id", "back-ref por cliente_id");
        }
    }

    /// Carga el módulo crm por el mismo camino que usa `nakui-ui`
    /// (`load_ui_modules` → `brahman_cards::load_cards_from_dir`). Se
    /// aísla en un tempdir para no acoplar el test a los otros módulos
    /// de ejemplo.
    #[test]
    fn crm_module_loads_via_card_pipeline() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../examples/nakui-modules/crm/module.json");
        let root = tempfile::tempdir().unwrap();
        let crm_dir = root.path().join("crm");
        std::fs::create_dir(&crm_dir).unwrap();
        std::fs::copy(&src, crm_dir.join("module.json")).unwrap();

        let (modules, skipped) = load_ui_modules(root.path()).expect("el módulo crm debe cargar");
        assert!(skipped.is_empty(), "ninguna card debe saltarse");
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].id, "crm");
    }
}
