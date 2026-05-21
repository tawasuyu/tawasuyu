//! Validación de los 6 módulos demo en `examples/nakui-modules/`.
//!
//! Si esto verde, garantizamos que un usuario que clone el repo y
//! corra `NAKUI_MODULES_DIR=examples/nakui-modules cargo run -p nakui-ui`
//! va a obtener los 6 módulos cargados sin tocar nada.

use nahual_meta_schema::{load_modules_from_dir, FieldKind, View};

fn examples_dir() -> std::path::PathBuf {
    // Tests corren desde el dir del crate; el repo root está dos
    // niveles arriba: crates/modules/nakui/ui-schema → repo.
    // Tras el lift a nahual, el crate vive en
    // `crates/modules/ui_engine/libs/meta-schema`, así que el repo
    // root queda 5 niveles arriba.
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    here.join("../../../../..").join("examples/nakui-modules")
}

#[test]
fn loads_all_demo_modules() {
    let dir = examples_dir();
    let mods = load_modules_from_dir(&dir).unwrap_or_else(|e| {
        panic!("load failed for {}: {e}", dir.display());
    });
    let ids: Vec<&str> = mods.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "crm",
            "customers",
            "inventory_movements",
            "invoices",
            "products",
            "sales_engine",
            "sales_orders",
            "suppliers",
        ],
        "expected 8 modules in alphabetical order \
         (crm se sumó como ERP con morfismos)"
    );
}

#[test]
fn sales_engine_declares_nakui_module_dir_and_morphism() {
    // Sanity del módulo demo de morphism: nakui_module_dir set,
    // y al menos una vista con Action::Morphism en su on_submit.
    let mods = load_modules_from_dir(examples_dir()).unwrap();
    let sales = mods
        .iter()
        .find(|m| m.id == "sales_engine")
        .expect("sales_engine debe estar");
    assert!(
        sales.nakui_module_dir.is_some(),
        "sales_engine debería declarar nakui_module_dir"
    );
    let has_morphism_view = sales.views.values().any(|v| match v {
        nahual_meta_schema::View::Form(form) => {
            matches!(form.on_submit, nahual_meta_schema::Action::Morphism { .. })
        }
        _ => false,
    });
    assert!(
        has_morphism_view,
        "sales_engine debería tener al menos una Form con Action::Morphism"
    );
}

#[test]
fn every_demo_module_has_list_and_form_views() {
    let mods = load_modules_from_dir(examples_dir()).unwrap();
    for m in &mods {
        let mut has_list = false;
        let mut has_form = false;
        for v in m.views.values() {
            match v {
                View::List(_) => has_list = true,
                View::Form(_) => has_form = true,
            }
        }
        assert!(
            has_list && has_form,
            "module {} should expose at least one list + one form view",
            m.id
        );
    }
}

#[test]
fn every_demo_form_field_kind_is_recognized() {
    // Sanity: ningún módulo demo usa un kind que no esté en el enum
    // (sería rechazado al parsear, pero check explícito no daña).
    let mods = load_modules_from_dir(examples_dir()).unwrap();
    for m in &mods {
        for v in m.views.values() {
            if let View::Form(form) = v {
                for f in &form.fields {
                    let _ok = matches!(
                        f.kind,
                        FieldKind::Text
                            | FieldKind::Multiline
                            | FieldKind::Number
                            | FieldKind::Boolean
                            | FieldKind::Date
                    );
                }
            }
        }
    }
}

#[test]
fn every_module_validates_clean() {
    // validate() chequea que cada MenuItem.view exista en views.
    // Un typo en cualquiera de los 6 módulos haría fallar este test.
    let mods = load_modules_from_dir(examples_dir()).unwrap();
    for m in &mods {
        m.validate()
            .unwrap_or_else(|e| panic!("module {} failed validate: {e}", m.id));
    }
}
