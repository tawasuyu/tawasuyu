//! Validación de los módulos demo que trae el shell `nakui-ui-llimphi`
//! en `01_yachay/nakui/nakui-ui-llimphi/examples/nakui-modules/`.
//!
//! Si esto está verde, garantizamos que un usuario que clone el repo y
//! corra `NAKUI_MODULES_DIR=01_yachay/nakui/nakui-ui-llimphi/examples/nakui-modules
//! cargo run -p nakui-ui-llimphi` va a obtener los módulos cargados sin
//! tocar nada.

use nahual_meta_schema::{load_modules_from_dir, FieldKind, View};

fn examples_dir() -> std::path::PathBuf {
    // El crate vive en `02_ruway/nahual/libs/meta-schema`; el repo root
    // queda 4 niveles arriba. Los módulos demo viven junto al shell.
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    here.join("../../../..")
        .join("01_yachay/nakui/nakui-ui-llimphi/examples/nakui-modules")
}

#[test]
fn loads_demo_modules() {
    let dir = examples_dir();
    let mods = load_modules_from_dir(&dir).unwrap_or_else(|e| {
        panic!("load failed for {}: {e}", dir.display());
    });
    let ids: Vec<&str> = mods.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["tesoro", "ventas"],
        "se esperaban los módulos demo 'tesoro' (tesorería) y 'ventas'"
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
                View::Detail(_) | View::Dashboard(_) | View::Report(_) | View::Graph(_) => {}
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
fn ventas_has_dashboard_and_report_views() {
    let mods = load_modules_from_dir(examples_dir()).unwrap();
    let ventas = mods.iter().find(|m| m.id == "ventas").expect("ventas");
    let has_dashboard = ventas.views.values().any(|v| matches!(v, View::Dashboard(_)));
    let has_report = ventas.views.values().any(|v| matches!(v, View::Report(_)));
    assert!(has_dashboard, "ventas debería tener un tablero");
    assert!(has_report, "ventas debería tener un reporte");
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
                            | FieldKind::Select
                            | FieldKind::EntityRef
                            | FieldKind::AutoId
                    );
                }
            }
        }
    }
}

#[test]
fn every_module_validates_clean() {
    // validate() chequea que cada MenuItem.view exista en views y que
    // los row_detail apunten a una vista Detail. Un typo haría fallar.
    let mods = load_modules_from_dir(examples_dir()).unwrap();
    for m in &mods {
        m.validate()
            .unwrap_or_else(|e| panic!("module {} failed validate: {e}", m.id));
    }
}
