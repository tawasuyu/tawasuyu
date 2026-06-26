//! El catálogo de plugins commiteado (`assets/`) debe cargar y **verificar
//! firma** entero: cada `.ron` (menos `trust.ron`) produce un plugin cargado.
//! Un plugin rechazado (firma inválida, caps que no casan con la firma, manifest
//! roto, import sin capacidad) NO entra en el vector — así este test cae si
//! alguien toca un .wasm/caps sin re-firmar con `build-mirada-plugins.sh`.

use std::path::Path;

use mirada_plugin_host::load_plugins_dir;

#[test]
fn el_catalogo_de_assets_carga_y_verifica() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");

    // Cuántos manifests hay (excluyendo el anillo de confianza).
    let manifests = std::fs::read_dir(&dir)
        .expect("assets/ existe")
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            p.extension().and_then(|x| x.to_str()) == Some("ron")
                && p.file_name().and_then(|x| x.to_str()) != Some("trust.ron")
        })
        .count();

    let plugins = load_plugins_dir(&dir);
    assert_eq!(
        plugins.len(),
        manifests,
        "algún plugin del catálogo no cargó/verificó ({} de {} manifests)",
        plugins.len(),
        manifests,
    );
    // El catálogo creció con los nuevos: scratchpads, los 3 layouts y los 4
    // reactores extra, más los 4 de base.
    assert!(manifests >= 12, "el catálogo tiene menos plugins de los esperados");
}
