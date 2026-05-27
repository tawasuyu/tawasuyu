//! Validación del archivo `rules.example.json`: un JSON canónico de
//! Reglas de ejemplo que documenta el schema vivo (`arje-brain-rules`)
//! con tres casos típicos del fractal — log de muertes, log de
//! dispositivos añadidos, detección de tormenta de crashes.
//!
//! Si el schema cambia, este test cae y obliga a actualizar el ejemplo.

use std::path::PathBuf;

fn rules_example_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rules.example.json")
}

#[test]
fn rules_example_parsea_y_valida() {
    let path = rules_example_path();
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("no se pudo leer {}: {e}", path.display()));

    let rules = arje_brain::extract_rules_from_json(&raw)
        .expect("rules.example.json no parsea");

    // Tres reglas canónicas — si cambian, actualizar el archivo Y este
    // contador para que el test siga delatando regresiones silenciosas.
    assert_eq!(
        rules.len(),
        3,
        "rules.example.json debería traer exactamente 3 reglas",
    );

    // Cada regla debe `validate()` por sí sola (acciones no vacías,
    // patterns recursivos coherentes, etc.).
    for r in &rules {
        r.validate().unwrap_or_else(|e| {
            panic!("regla {} no valida: {e}", r.id)
        });
        assert!(
            !r.then.is_empty(),
            "regla {} no debe tener `then` vacío",
            r.id
        );
    }
}
