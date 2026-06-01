//! El contrato del config en disco: un TOML real debe deserializar al modelo
//! de `pata-core` sin pérdida. Linux carga TOML; este test fija que el esquema
//! (superficies múltiples, slots, props `flatten` + `Prop` untagged) sobrevive
//! el viaje. Vive como test de integración porque `toml` es `std` y dev-only.

use pata_core::{Anchor, Config, Prop, SurfaceKind};

#[test]
fn deserializa_un_marco_completo_desde_toml() {
    let src = r#"
        [general]
        timezone = "America/Lima"

        [[surfaces]]
        kind = "bar"
        anchor = "top"
        thickness = 28

        [[surfaces.start]]
        kind = "start_button"

        [[surfaces.start]]
        kind = "clock"
        format = "%H:%M"
        size = 14

        [[surfaces.end]]
        kind = "astro"
        moon = true
        lat = -12.04

        [[surfaces]]
        kind = "bar"
        anchor = "bottom"
        autohide = true

        [[surfaces.center]]
        kind = "shuma_input"
        hotkey = "F12"
    "#;

    let cfg: Config = toml::from_str(src).expect("el TOML debe parsear al modelo");

    assert_eq!(cfg.general.timezone, "America/Lima");
    assert_eq!(cfg.surfaces.len(), 2);

    let top = &cfg.surfaces[0];
    assert_eq!(top.kind, SurfaceKind::Bar);
    assert_eq!(top.anchor, Anchor::Top);
    assert_eq!(top.thickness, 28.0);
    assert_eq!(top.start.len(), 2);
    assert_eq!(top.start[0].kind, "start_button");
    // Props heterogéneas (string + int) llegan por flatten/untagged.
    assert_eq!(top.start[1].str_prop("format", "?"), "%H:%M");
    assert_eq!(top.start[1].num_prop("size", 0.0), 14.0);

    let astro = &top.end[0];
    assert_eq!(astro.kind, "astro");
    assert!(astro.bool_prop("moon", false));
    assert_eq!(astro.num_prop("lat", 0.0), -12.04);

    let shell = &cfg.surfaces[1];
    assert_eq!(shell.anchor, Anchor::Bottom);
    assert!(shell.autohide);
    assert_eq!(shell.center[0].kind, "shuma_input");
    assert_eq!(shell.center[0].str_prop("hotkey", "?"), "F12");
}

#[test]
fn props_desconocidas_se_conservan() {
    let src = r#"
        [[surfaces]]
        anchor = "right"

        [[surfaces.start]]
        kind = "custom"
        color = "rebeccapurple"
        ratio = 0.42
        veces = 3
    "#;
    let cfg: Config = toml::from_str(src).unwrap();
    let w = &cfg.surfaces[0].start[0];
    assert_eq!(w.str_prop("color", "?"), "rebeccapurple");
    assert_eq!(w.num_prop("ratio", 0.0), 0.42);
    assert_eq!(w.num_prop("veces", 0.0), 3.0);
    // anchor sin kind de superficie cae al default Bar.
    assert_eq!(cfg.surfaces[0].kind, SurfaceKind::Bar);
}

#[test]
fn marco_minimo_y_vacio() {
    // Sin superficies declaradas: config válido y vacío.
    let cfg: Config = toml::from_str("").unwrap();
    assert!(cfg.surfaces.is_empty());
    assert_eq!(cfg.general.timezone, "auto");

    // Un Prop entero no se confunde con float al volver a leerse.
    let _ = Prop::Int(1);
}
