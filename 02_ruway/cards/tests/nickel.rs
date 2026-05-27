//! Nickel reader + templates.
//!
//! V2 del brazo: la dispatcher acepta archivos `.ncl`. La evaluación
//! produce JSON intermedio que va a los readers estándar, así que un
//! `.ncl` puede generar cualquier `CardBody` siempre que su shape sea
//! reconocida.
//!
//! Templates: Nickel `import` + `&` merge nativos. El brazo no
//! inventa nada — sólo agrega el parent dir + el env
//! `BRAHMAN_CARDS_TEMPLATES_DIR` al import path.

use std::fs;
use std::path::PathBuf;

use cards::{
    eval_nickel_file, load_card, CardBody, CardLoadError, NickelEvalError,
    BRAHMAN_CARDS_TEMPLATES_ENV,
};
use serde_json::json;

// ===========================================================================
// Helpers
// ===========================================================================

fn unique_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "brahman-cards-nickel-{}-{}-{}",
        std::process::id(),
        nanos(),
        name
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn write_file(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, content).unwrap();
    p
}

// ===========================================================================
// 1. Evaluación directa: Nickel → Value
// ===========================================================================

#[test]
fn eval_nickel_file_returns_value_for_valid_input() {
    let dir = unique_dir("eval-basic");
    let p = write_file(
        &dir,
        "card.ncl",
        r#"
            {
              id = "demo",
              label = "Demo",
              entities = [],
              menu = [],
              views = {},
            }
        "#,
    );
    let v = eval_nickel_file(&p).expect("eval ok");
    assert_eq!(v.get("id"), Some(&json!("demo")));
    assert_eq!(v.get("label"), Some(&json!("Demo")));
    assert!(v.get("entities").is_some());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn eval_nickel_file_surfaces_evaluation_error() {
    let dir = unique_dir("eval-err");
    let p = write_file(
        &dir,
        "broken.ncl",
        r#"
            {
              id = "x",
              label = doesnotexist,
            }
        "#,
    );
    let err = eval_nickel_file(&p).unwrap_err();
    match err {
        NickelEvalError::Eval { path, message } => {
            assert!(path.contains("broken.ncl"));
            assert!(!message.is_empty(), "el msg debe traer info de Nickel");
        }
        other => panic!("expected Eval error, got {other:?}"),
    }
    fs::remove_dir_all(&dir).ok();
}

// ===========================================================================
// 2. load_card pipeline: .ncl → Card
// ===========================================================================

#[test]
fn load_card_dispatches_ncl_to_ui_module_variant() {
    let dir = unique_dir("dispatch-ui");
    let p = write_file(
        &dir,
        "module.ncl",
        r#"
            {
              id = "demo",
              label = "Demo",
              entities = [],
              menu = [{ label = "Stock", view = "stock_list" }],
              views = {
                stock_list = {
                  kind = "list",
                  title = "Stock",
                  entity = "Stock",
                  columns = [],
                },
              },
            }
        "#,
    );
    let card = load_card(&p).expect("load ok");
    assert_eq!(card.body.kind_name(), "ui_module");
    assert_eq!(card.id, "demo");
    assert_eq!(card.label, "Demo");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn load_card_dispatches_ncl_to_ente_variant() {
    let dir = unique_dir("dispatch-ente");
    let p = write_file(
        &dir,
        "ente.ncl",
        r#"
            {
              schema_version = 1,
              id = "01ARZ3NDEKTSV4RRFFQ69G5FAV",
              label = "test-ente",
              payload = "Virtual",
              supervision = "OneShot",
            }
        "#,
    );
    let card = load_card(&p).expect("load ok");
    assert_eq!(card.body.kind_name(), "ente");
    assert_eq!(card.id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    fs::remove_dir_all(&dir).ok();
}

// ===========================================================================
// 3. Templates: import + merge native de Nickel
// ===========================================================================

/// El caso de uso que el usuario describió: "un Card simple usa un
/// Card ya hecho cambiando sólo nombre y id". Template define la
/// shape full; el archivo concreto importa + override.
#[test]
fn template_merge_overrides_id_and_label_only() {
    let dir = unique_dir("template-merge");

    // Template con la shape full de un UiModule. Los campos
    // sobrescribibles se marcan `| default` — Nickel sólo permite
    // override en merge cuando hay diferencia de prioridad. Sin
    // `| default` los strings no-iguales fallan con "non mergeable".
    write_file(
        &dir,
        "ui_module_basic.ncl",
        r#"
            {
              id | String | default = "TEMPLATE_ID",
              label | String | default = "TEMPLATE_LABEL",
              description = "stock + form básico",
              entities = [
                { name = "Item", label = "Item", fields = [] },
              ],
              menu = [
                { label = "Items", view = "items_list" },
                { label = "+ Item", view = "items_form" },
              ],
              views = {
                items_list = {
                  kind = "list",
                  title = "Items",
                  entity = "Item",
                  columns = [],
                },
                items_form = {
                  kind = "form",
                  title = "Nuevo item",
                  entity = "Item",
                  fields = [],
                  on_submit = {
                    kind = "seed_entity",
                    entity = "Item",
                    next_view = "items_list",
                  },
                },
              },
            }
        "#,
    );

    // Card concreto: import + merge override.
    let p = write_file(
        &dir,
        "my_module.ncl",
        r#"
            let base = import "ui_module_basic.ncl" in
            base & {
              id = "my_module",
              label = "Mi Módulo",
            }
        "#,
    );

    let card = load_card(&p).expect("template merge ok");
    assert_eq!(card.id, "my_module", "el override del id se aplicó");
    assert_eq!(card.label, "Mi Módulo", "el override del label se aplicó");
    assert_eq!(card.body.kind_name(), "ui_module");
    match card.body {
        CardBody::UiModule(m) => {
            // El resto viene del template intacto.
            assert_eq!(m.menu.len(), 2);
            assert_eq!(m.entities.len(), 1);
            assert_eq!(m.entities[0].name, "Item");
        }
        other => panic!("variant inesperado: {:?}", other.kind_name()),
    }

    fs::remove_dir_all(&dir).ok();
}

/// El env `BRAHMAN_CARDS_TEMPLATES_DIR` permite tener un registry
/// global: el usuario importa por nombre desnudo desde cualquier
/// ubicación.
///
/// Este test setea/unset el env de forma local (no thread-safe en
/// tests paralelos contra el mismo env, pero usamos una key dedicada
/// y borramos después). Si se vuelve flaky, agregar mutex.
#[test]
fn template_resolves_via_env_registry() {
    let registry = unique_dir("template-registry");
    let inputs = unique_dir("template-input");

    write_file(
        &registry,
        "ui_module_minimal.ncl",
        r#"
            {
              id | String | default = "X",
              label | String | default = "X",
              entities = [],
              menu = [],
              views = {},
            }
        "#,
    );

    let p = write_file(
        &inputs,
        "from_registry.ncl",
        r#"
            let base = import "ui_module_minimal.ncl" in
            base & { id = "registry_user", label = "Usado del Registry" }
        "#,
    );

    // Set env, evaluar, restaurar.
    let prev = std::env::var(BRAHMAN_CARDS_TEMPLATES_ENV).ok();
    // SAFETY: nickel-lang tests modifican un env ad-hoc que no es
    // referenciado por nada externo y se restaura al salir. Ningún
    // otro test del crate lee este env.
    unsafe {
        std::env::set_var(BRAHMAN_CARDS_TEMPLATES_ENV, &registry);
    }

    let result = load_card(&p);

    unsafe {
        if let Some(v) = prev {
            std::env::set_var(BRAHMAN_CARDS_TEMPLATES_ENV, v);
        } else {
            std::env::remove_var(BRAHMAN_CARDS_TEMPLATES_ENV);
        }
    }

    let card = result.expect("template via registry ok");
    assert_eq!(card.id, "registry_user");
    assert_eq!(card.body.kind_name(), "ui_module");

    fs::remove_dir_all(&registry).ok();
    fs::remove_dir_all(&inputs).ok();
}

// ===========================================================================
// 4. Errores propagan limpios al CardLoadError
// ===========================================================================

#[test]
fn load_card_wraps_nickel_error_in_card_load_error() {
    let dir = unique_dir("wrap-err");
    let p = write_file(&dir, "bad.ncl", "let x = unknown in x");
    let err = load_card(&p).unwrap_err();
    match err {
        CardLoadError::Nickel(NickelEvalError::Eval { .. }) => {} // expected
        other => panic!("expected Nickel(Eval), got {other:?}"),
    }
    fs::remove_dir_all(&dir).ok();
}

/// El value-add concreto de Nickel sobre JSON: un contract
/// violation se captura en evaluación, ANTES de que el reader
/// JSON tenga oportunidad de aceptar un shape mal-tipado. Acá un
/// `id | String` con un value que no es String falla en eval-time
/// con un mensaje legible. JSON puro lo aceptaría y rompería más
/// tarde aguas abajo.
#[test]
fn nickel_contract_violation_caught_at_eval_time() {
    let dir = unique_dir("contract-violation");
    let p = write_file(
        &dir,
        "bad_id.ncl",
        r#"
            {
              id | String = 42,
              label = "X",
              entities = [],
              menu = [],
              views = {},
            }
        "#,
    );
    let err = load_card(&p).unwrap_err();
    match err {
        CardLoadError::Nickel(NickelEvalError::Eval { message, .. }) => {
            // Mensaje de contract violation legible.
            assert!(
                message.contains("contract") || message.contains("String"),
                "msg debe mencionar contract o String: {message}"
            );
        }
        other => panic!("expected Nickel(Eval), got {other:?}"),
    }
    fs::remove_dir_all(&dir).ok();
}

/// Sanity: un Nickel que evalúa a un shape NO-reconocible (no
/// matchea ningún reader) cae en `NoMatchingReader` — la cadena
/// Nickel + dispatcher se mantiene coherente.
#[test]
fn ncl_evaluating_to_unknown_shape_returns_no_matching_reader() {
    let dir = unique_dir("unknown-shape");
    let p = write_file(
        &dir,
        "weird.ncl",
        r#"{ random = "shape", without = "fingerprint" }"#,
    );
    let err = load_card(&p).unwrap_err();
    assert!(
        matches!(err, CardLoadError::NoMatchingReader),
        "expected NoMatchingReader, got {err:?}"
    );
    fs::remove_dir_all(&dir).ok();
}
