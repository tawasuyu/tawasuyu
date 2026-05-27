//! Integration tests del brazo brahman-cards.
//!
//! Cubre:
//! 1. Cada reader matchea sólo el shape correcto.
//! 2. El dispatcher (`load_card`/`dispatch`) elige el reader
//!    correcto sin ambigüedad.
//! 3. Round-trip: cada format JSON cargado produce el variant
//!    esperado del Card canónico con los campos del wrapper bien
//!    derivados.
//! 4. Rechazo gracioso de inputs no-matched + extensiones no
//!    soportadas.

use std::collections::BTreeMap;

use cards::{
    default_readers, load_card_with, Card, CardBody, CardLoadError, CardReader, EnteJsonReader,
    MonadJsonReader, UiModuleJsonReader,
};
use serde_json::{json, Value};

/// Helper: dispatch in-process desde un Value, sin tocar disco.
/// Reproduce la lógica interna del dispatcher para no exigir I/O en
/// los tests.
fn dispatch(input: Value, readers: &[Box<dyn CardReader>]) -> Result<Card, CardLoadError> {
    for r in readers {
        if r.can_read(&input) {
            return r.read(input);
        }
    }
    Err(CardLoadError::NoMatchingReader)
}

// ===========================================================================
// Reader detection (can_read)
// ===========================================================================

#[test]
fn ui_module_reader_detects_only_ui_module_shape() {
    let r = UiModuleJsonReader;
    let ui = json!({"id": "x", "label": "X", "menu": [], "views": {}, "entities": []});
    let ente = json!({"id": "x", "label": "X", "payload": "Virtual", "supervision": "OneShot"});
    let monad = json!({"id": "x", "label": "X", "members": [], "cardinality": 0});
    assert!(r.can_read(&ui), "UiModule reader debe matchear ui shape");
    assert!(!r.can_read(&ente), "no debe matchear Ente");
    assert!(!r.can_read(&monad), "no debe matchear Monad");
    assert!(!r.can_read(&Value::Null), "no debe matchear non-object");
}

#[test]
fn ente_reader_detects_only_ente_shape() {
    let r = EnteJsonReader;
    let ente = json!({"payload": "Virtual", "supervision": "OneShot"});
    let monad = json!({"members": [], "cardinality": 0});
    let ui = json!({"menu": [], "views": {}, "entities": []});
    assert!(r.can_read(&ente));
    assert!(!r.can_read(&monad));
    assert!(!r.can_read(&ui));
}

#[test]
fn monad_reader_detects_only_monad_shape() {
    let r = MonadJsonReader;
    let monad = json!({"members": [], "cardinality": 0});
    let ente = json!({"payload": "Virtual", "supervision": "OneShot"});
    let ui = json!({"menu": [], "views": {}, "entities": []});
    assert!(r.can_read(&monad));
    assert!(!r.can_read(&ente));
    assert!(!r.can_read(&ui));
}

// ===========================================================================
// Dispatch + variant projection
// ===========================================================================

#[test]
fn loads_ui_module_to_card_ui_module_variant() {
    let input = json!({
        "id": "sales_engine",
        "label": "Ventas",
        "description": "Demo",
        "entities": [],
        "menu": [{"label": "Stock", "view": "stock_list"}],
        "views": {
            "stock_list": {
                "kind": "list",
                "title": "Stock",
                "entity": "Stock",
                "columns": []
            }
        }
    });
    let card = dispatch(input, &default_readers()).expect("dispatch ok");
    assert_eq!(card.id, "sales_engine");
    assert_eq!(card.label, "Ventas");
    assert!(card.lineage.is_none(), "UiModule sin lineage");
    assert_eq!(card.body.kind_name(), "ui_module");
    match card.body {
        CardBody::UiModule(m) => {
            assert_eq!(m.id, "sales_engine");
            assert_eq!(m.menu.len(), 1);
        }
        other => panic!("variant inesperado: {:?}", other.kind_name()),
    }
}

#[test]
fn loads_ente_to_card_ente_variant() {
    // Ulid mínimo: 26 chars Crockford. Usamos uno conocido.
    let ulid = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let input = json!({
        "schema_version": 1,
        "id": ulid,
        "label": "test-ente",
        "payload": "Virtual",
        "supervision": "OneShot"
    });
    let card = dispatch(input, &default_readers()).expect("dispatch ok");
    assert_eq!(card.id, ulid);
    assert_eq!(card.label, "test-ente");
    assert_eq!(card.body.kind_name(), "ente");
    match card.body {
        CardBody::Ente(e) => {
            assert_eq!(e.label, "test-ente");
            assert_eq!(e.id.to_string(), ulid);
        }
        other => panic!("variant inesperado: {:?}", other.kind_name()),
    }
}

#[test]
fn loads_monad_to_card_monad_variant() {
    let ulid = "01ARZ3NDEKTSV4RRFFQ69G5FB1";
    let input = json!({
        "schema_version": 1,
        "id": ulid,
        "label": "test-monad",
        "members": [],
        "cardinality": 0,
        "created_at_ms": 0,
        "updated_at_ms": 0
    });
    let card = dispatch(input, &default_readers()).expect("dispatch ok");
    assert_eq!(card.id, ulid);
    assert_eq!(card.label, "test-monad");
    assert_eq!(card.body.kind_name(), "monad");
    match card.body {
        CardBody::Monad(m) => {
            assert_eq!(m.label, "test-monad");
            assert_eq!(m.cardinality, 0);
        }
        other => panic!("variant inesperado: {:?}", other.kind_name()),
    }
}

// ===========================================================================
// Negative cases
// ===========================================================================

#[test]
fn rejects_input_no_matching_reader() {
    let input = json!({"random": "shape", "without": "fingerprint"});
    let err = dispatch(input, &default_readers()).unwrap_err();
    assert!(
        matches!(err, CardLoadError::NoMatchingReader),
        "expected NoMatchingReader, got {err:?}"
    );
}

#[test]
fn rejects_non_object_input() {
    let input = json!([1, 2, 3]);
    let err = dispatch(input, &default_readers()).unwrap_err();
    assert!(matches!(err, CardLoadError::NoMatchingReader));
}

#[test]
fn ui_module_takes_priority_when_shape_overlaps_partial() {
    // Sanity del orden: si alguien armara un input híbrido con
    // `menu`+`views`+`entities` Y también `payload`+`supervision`,
    // el UiModuleReader (primero en orden) debería ganar. Esto no
    // debería ocurrir con inputs reales pero defendemos el contrato
    // de orden documentado.
    let input = json!({
        "id": "weird",
        "label": "Weird",
        "menu": [],
        "views": {},
        "entities": [],
        "payload": "Virtual",
        "supervision": "OneShot"
    });
    let card = dispatch(input, &default_readers()).expect("dispatch ok");
    assert_eq!(
        card.body.kind_name(),
        "ui_module",
        "el UiModuleReader debería ganar por orden"
    );
}

// ===========================================================================
// load_card desde disco (e2e fino)
// ===========================================================================

#[test]
fn load_card_from_disk_round_trip_ui_module() {
    let tmp = tempfile_path("ui_module.json");
    let input = json!({
        "id": "demo",
        "label": "Demo",
        "entities": [],
        "menu": [],
        "views": {}
    });
    std::fs::write(&tmp, serde_json::to_vec_pretty(&input).unwrap()).unwrap();

    let card = load_card_with(&tmp, &default_readers()).expect("load ok");
    assert_eq!(card.body.kind_name(), "ui_module");
    assert_eq!(card.id, "demo");

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn load_card_rejects_unsupported_extension() {
    let tmp = tempfile_path("foo.toml");
    std::fs::write(&tmp, b"[anything]\nx = 1").unwrap();
    let err = load_card_with(&tmp, &default_readers()).unwrap_err();
    match err {
        CardLoadError::UnsupportedExtension { ext, supported } => {
            assert_eq!(ext, "toml");
            assert!(supported.contains(&"json"));
        }
        other => panic!("expected UnsupportedExtension, got {other:?}"),
    }
    let _ = std::fs::remove_file(&tmp);
}

// ===========================================================================
// Custom reader sets
// ===========================================================================

#[test]
fn custom_reader_set_can_restrict_supported_formats() {
    // Sólo Ente: un input Monad debería rechazarse.
    let only_ente: Vec<Box<dyn CardReader>> = vec![Box::new(EnteJsonReader)];
    let monad_input = json!({"members": [], "cardinality": 0});
    let err = dispatch(monad_input, &only_ente).unwrap_err();
    assert!(matches!(err, CardLoadError::NoMatchingReader));
}

// ===========================================================================
// Wrapper field invariants
// ===========================================================================

#[test]
fn extensions_field_starts_empty_in_v1() {
    // Documented: V1 no mueve el "extras" del crate origen al
    // wrapper.extensions. Si esto cambia, este test se rompe como
    // signal para actualizar el doc de readers.rs.
    let input = json!({
        "id": "demo",
        "label": "Demo",
        "entities": [],
        "menu": [],
        "views": {}
    });
    let card = dispatch(input, &default_readers()).unwrap();
    assert_eq!(card.extensions, BTreeMap::new());
}

// ===========================================================================
// load_cards_from_dir (subdir walking)
// ===========================================================================

#[test]
fn load_cards_from_dir_walks_subdirs_and_finds_module_json() {
    let root = unique_dir("dir-walk");
    // Subdir A: tiene module.json (UiModule).
    let a = root.join("alpha");
    std::fs::create_dir(&a).unwrap();
    std::fs::write(
        a.join("module.json"),
        serde_json::to_vec_pretty(&json!({
            "id": "alpha",
            "label": "Alpha",
            "entities": [],
            "menu": [],
            "views": {}
        }))
        .unwrap(),
    )
    .unwrap();
    // Subdir B: tiene module.json (UiModule).
    let b = root.join("bravo");
    std::fs::create_dir(&b).unwrap();
    std::fs::write(
        b.join("module.json"),
        serde_json::to_vec_pretty(&json!({
            "id": "bravo",
            "label": "Bravo",
            "entities": [],
            "menu": [],
            "views": {}
        }))
        .unwrap(),
    )
    .unwrap();
    // Subdir C: NO tiene ninguno de los filenames convencionales —
    // se debe skipear sin error.
    let c = root.join("charlie");
    std::fs::create_dir(&c).unwrap();
    std::fs::write(c.join("readme.txt"), b"sin card aca").unwrap();

    let cards = cards::load_cards_from_dir(&root).expect("ok");
    let ids: Vec<&str> = cards.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["alpha", "bravo"],
        "orden lexicográfico por subdir name"
    );
    for c in &cards {
        assert_eq!(c.body.kind_name(), "ui_module");
    }

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn load_cards_from_dir_prefers_ncl_over_json_when_both_present() {
    let root = unique_dir("dir-prefer");
    let sub = root.join("only");
    std::fs::create_dir(&sub).unwrap();
    // Ambos archivos existen; el .ncl debería ganar.
    std::fs::write(
        sub.join("card.ncl"),
        r#"{ id = "from_ncl", label = "Ncl", entities = [], menu = [], views = {} }"#,
    )
    .unwrap();
    std::fs::write(
        sub.join("card.json"),
        serde_json::to_vec(&json!({
            "id": "from_json",
            "label": "Json",
            "entities": [], "menu": [], "views": {}
        }))
        .unwrap(),
    )
    .unwrap();

    let cards = cards::load_cards_from_dir(&root).expect("ok");
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].id, "from_ncl", "card.ncl tiene prioridad");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn load_cards_from_dir_propagates_per_file_errors_loud() {
    let root = unique_dir("dir-error-loud");
    let sub = root.join("broken");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("card.json"), b"{ this is not valid json").unwrap();

    let err = cards::load_cards_from_dir(&root).unwrap_err();
    assert!(
        matches!(err, CardLoadError::JsonParse(_)),
        "el error de un file roto debe propagar fail-loud, got {err:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn load_cards_from_dir_with_custom_filenames() {
    let root = unique_dir("dir-custom-fname");
    let sub = root.join("only");
    std::fs::create_dir(&sub).unwrap();
    // Filename custom que NO está en el default.
    std::fs::write(
        sub.join("manifest.json"),
        serde_json::to_vec(&json!({
            "id": "x",
            "label": "X",
            "entities": [], "menu": [], "views": {}
        }))
        .unwrap(),
    )
    .unwrap();

    // Default no encuentra nada (skipea):
    let with_default = cards::load_cards_from_dir(&root).unwrap();
    assert_eq!(with_default.len(), 0, "default filenames no incluye manifest.json");

    // Custom filename encuentra:
    let with_custom = cards::load_cards_from_dir_with(
        &root,
        &["manifest.json"],
        &cards::default_readers(),
    )
    .unwrap();
    assert_eq!(with_custom.len(), 1);
    assert_eq!(with_custom[0].id, "x");

    std::fs::remove_dir_all(&root).ok();
}

// ===========================================================================
// Helpers de tests
// ===========================================================================

fn tempfile_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "brahman-cards-test-{}-{}",
        std::process::id(),
        name
    ));
    p
}

fn unique_dir(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "brahman-cards-test-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        name
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}
