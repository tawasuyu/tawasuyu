//! Tests E2E de los templates canónicos shipped con el crate.
//!
//! Cada test escribe un Card user-side en un tempdir, importa el
//! template canónico, override id/label/etc., y verifica que el
//! brazo lo dispatcha al variant correcto del CardBody con los
//! valores merged.
//!
//! `BRAHMAN_CARDS_TEMPLATES_DIR` se setea localmente en cada test.
//! Como Nickel también busca relativo al input file, usamos el env
//! para que `import "ente_basic.ncl"` (sin path) resuelva desde
//! cualquier ubicación del input.

use std::fs;
use std::path::PathBuf;

use brahman_cards::{
    canonical_templates_dir, load_card, CardBody, BRAHMAN_CARDS_TEMPLATES_ENV,
};

/// Helper: corre `f()` con `BRAHMAN_CARDS_TEMPLATES_ENV` set al
/// directorio de templates canónicos, restaurando el env al salir.
///
/// Tests no son thread-safe entre sí cuando comparten env. Por eso
/// quedan en serial via `nextest --test-threads=1` o `cargo test`
/// que paralelizara sólo entre `tests/*.rs` distintos. Como este
/// archivo encapsula todo el setup de env, aún en paralelo entre
/// archivos de tests no chocan (cada thread setea/restaura).
fn with_canonical_templates<F: FnOnce()>(f: F) {
    let prev = std::env::var(BRAHMAN_CARDS_TEMPLATES_ENV).ok();
    let dir = canonical_templates_dir();
    // SAFETY: env mutation single-threaded en este test.
    unsafe {
        std::env::set_var(BRAHMAN_CARDS_TEMPLATES_ENV, &dir);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    unsafe {
        match prev {
            Some(v) => std::env::set_var(BRAHMAN_CARDS_TEMPLATES_ENV, v),
            None => std::env::remove_var(BRAHMAN_CARDS_TEMPLATES_ENV),
        }
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

fn unique_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "brahman-cards-templates-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        name
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn ente_basic_template_overridden_loads_as_ente_card() {
    with_canonical_templates(|| {
        let dir = unique_dir("ente");
        let card_path = dir.join("my_ente.ncl");
        fs::write(
            &card_path,
            r#"
            let base = import "ente_basic.ncl" in
            base & {
              id = "01ARZ3NDEKTSV4RRFFQ69G5FAV",
              label = "mi-ente",
            }
            "#,
        )
        .unwrap();

        let card = load_card(&card_path).expect("load ente");
        assert_eq!(card.id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(card.label, "mi-ente");
        assert_eq!(card.body.kind_name(), "ente");
        match card.body {
            CardBody::Ente(e) => {
                assert_eq!(e.label, "mi-ente");
                // Defaults del template intactos.
                assert_eq!(e.schema_version, 1);
                // Payload es el "Virtual" del template default.
                assert!(
                    matches!(e.payload, brahman_card::Payload::Virtual),
                    "payload debería ser Virtual, got {:?}",
                    e.payload
                );
            }
            other => panic!("variant inesperado: {:?}", other.kind_name()),
        }

        fs::remove_dir_all(&dir).ok();
    });
}

#[test]
fn monad_basic_template_overridden_loads_as_monad_card() {
    with_canonical_templates(|| {
        let dir = unique_dir("monad");
        let card_path = dir.join("my_monad.ncl");
        fs::write(
            &card_path,
            r#"
            let base = import "monad_basic.ncl" in
            base & {
              id = "01ARZ3NDEKTSV4RRFFQ69G5FAW",
              label = "fotos-2026",
              cardinality = 5,
            }
            "#,
        )
        .unwrap();

        let card = load_card(&card_path).expect("load monad");
        assert_eq!(card.id, "01ARZ3NDEKTSV4RRFFQ69G5FAW");
        assert_eq!(card.label, "fotos-2026");
        assert_eq!(card.body.kind_name(), "monad");
        match card.body {
            CardBody::Monad(m) => {
                assert_eq!(m.label, "fotos-2026");
                assert_eq!(m.cardinality, 5);
                // Defaults del template intactos.
                assert_eq!(m.schema_version, 1);
                assert!(m.members.is_empty());
                assert!(m.summary.is_empty());
            }
            other => panic!("variant inesperado: {:?}", other.kind_name()),
        }

        fs::remove_dir_all(&dir).ok();
    });
}

#[test]
fn ui_module_basic_template_overridden_loads_as_ui_module_card() {
    with_canonical_templates(|| {
        let dir = unique_dir("ui");
        let card_path = dir.join("my_module.ncl");
        fs::write(
            &card_path,
            r#"
            let base = import "ui_module_basic.ncl" in
            base & {
              id = "customers",
              label = "Clientes",
              menu = [{ label = "Lista", view = "list" }],
              views = {
                list = {
                  kind = "list",
                  title = "Customers",
                  entity = "Customer",
                  columns = [],
                },
              },
            }
            "#,
        )
        .unwrap();

        let card = load_card(&card_path).expect("load ui_module");
        assert_eq!(card.id, "customers");
        assert_eq!(card.label, "Clientes");
        assert_eq!(card.body.kind_name(), "ui_module");
        match card.body {
            CardBody::UiModule(m) => {
                assert_eq!(m.id, "customers");
                assert_eq!(m.menu.len(), 1);
                assert!(m.views.contains_key("list"));
                // Defaults del template: entities vacío.
                assert!(m.entities.is_empty());
            }
            other => panic!("variant inesperado: {:?}", other.kind_name()),
        }

        fs::remove_dir_all(&dir).ok();
    });
}

#[test]
fn template_default_id_and_label_pass_through_when_not_overridden() {
    // Sanity: si el usuario importa el template SIN override de
    // id/label, los defaults `"TEMPLATE_ID"` y `"TEMPLATE_LABEL"`
    // pasan al wrapper Card.id/label. El brazo no falla — sólo
    // los muestra como están. Validar este flow garantiza que un
    // user "vacío" (importa y no override) carga sin error.
    with_canonical_templates(|| {
        let dir = unique_dir("defaults");
        let card_path = dir.join("noop.ncl");
        fs::write(&card_path, r#"import "ui_module_basic.ncl""#).unwrap();

        let card = load_card(&card_path).expect("load defaults");
        assert_eq!(card.id, "TEMPLATE_ID");
        assert_eq!(card.label, "TEMPLATE_LABEL");

        fs::remove_dir_all(&dir).ok();
    });
}

#[test]
fn canonical_templates_dir_actually_exists() {
    // Sanity: el path expuesto por canonical_templates_dir tiene
    // que apuntar a un directorio que existe físicamente, sino los
    // tests anteriores fallarían silenciosamente (Nickel reporta
    // import-not-found pero el test ya estaría roto).
    let d = canonical_templates_dir();
    assert!(d.is_dir(), "templates dir no existe: {}", d.display());
    for fname in ["ente_basic.ncl", "monad_basic.ncl", "ui_module_basic.ncl"] {
        let p = d.join(fname);
        assert!(p.is_file(), "template missing: {}", p.display());
    }
}
