//! El marco como configuración editable.
//!
//! `pata-core` ya es declarativo (un [`Config`] es datos puros), así que su
//! esquema [`allichay`] es casi un reflejo del modelo: una sección por
//! superficie + los settings generales. El renderizador lo pinta con dientes y
//! controles; los cambios vuelven por [`Configurable::apply`], que muta el
//! modelo. La persistencia (escribir el TOML) la hace `pata-config::save` en el
//! lado `std` — acá sólo vive la lógica `no_std`.
//!
//! v1 cubre los **escalares** de cada superficie (tipo, borde, grosor, padding,
//! gap, autohide, ancho de panel) y la zona horaria. Editar la **lista** de
//! superficies y de widgets (agregar/quitar/reordenar) queda para v2.

use alloc::format;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use allichay::{AllichayError, Configurable, EnumOption, Field, FieldPath, FieldValue, Schema, Section};

use crate::config::{Anchor, Config, Surface, SurfaceKind};

/// Prefijo de los ids de sección de cada superficie: `surface0`, `surface1`, …
const SURFACE_PREFIX: &str = "surface";

impl SurfaceKind {
    /// Id estable (coincide con el `serde(rename_all = "lowercase")`).
    fn as_id(&self) -> &'static str {
        match self {
            SurfaceKind::Bar => "bar",
            SurfaceKind::Panel => "panel",
            SurfaceKind::Dock => "dock",
            SurfaceKind::Sidebar => "sidebar",
        }
    }
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "bar" => Some(SurfaceKind::Bar),
            "panel" => Some(SurfaceKind::Panel),
            "dock" => Some(SurfaceKind::Dock),
            "sidebar" => Some(SurfaceKind::Sidebar),
            _ => None,
        }
    }
    fn options() -> Vec<EnumOption> {
        vec![
            EnumOption::new("bar", "Barra"),
            EnumOption::new("panel", "Panel"),
            EnumOption::new("dock", "Dock"),
            EnumOption::new("sidebar", "Sidebar"),
        ]
    }
    /// Glifo del diente de la superficie según su tipo.
    fn glyph(&self) -> &'static str {
        match self {
            SurfaceKind::Bar => "▭",
            SurfaceKind::Panel => "▦",
            SurfaceKind::Dock => "▣",
            SurfaceKind::Sidebar => "❘",
        }
    }
}

impl Anchor {
    fn as_id(&self) -> &'static str {
        match self {
            Anchor::Top => "top",
            Anchor::Bottom => "bottom",
            Anchor::Left => "left",
            Anchor::Right => "right",
        }
    }
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "top" => Some(Anchor::Top),
            "bottom" => Some(Anchor::Bottom),
            "left" => Some(Anchor::Left),
            "right" => Some(Anchor::Right),
            _ => None,
        }
    }
    fn options() -> Vec<EnumOption> {
        vec![
            EnumOption::new("top", "Arriba"),
            EnumOption::new("bottom", "Abajo"),
            EnumOption::new("left", "Izquierda"),
            EnumOption::new("right", "Derecha"),
        ]
    }
}

/// El esquema de una superficie: sus campos escalares.
fn surface_section(index: usize, s: &Surface) -> Section {
    let id = format!("{SURFACE_PREFIX}{index}");
    let title = format!("Superficie {}", index + 1);
    Section::new(id, title)
        .icon(s.kind.glyph())
        .help("Una barra, panel, dock o sidebar del marco")
        .field(Field::dropdown(
            "kind",
            "Tipo",
            s.kind.as_id(),
            SurfaceKind::options(),
        ))
        .field(Field::dropdown(
            "anchor",
            "Borde",
            s.anchor.as_id(),
            Anchor::options(),
        ))
        .field(Field::slider(
            "thickness",
            "Grosor",
            s.thickness as f64,
            16.0,
            200.0,
            1.0,
        ))
        .field(Field::slider("padding", "Padding", s.padding as f64, 0.0, 48.0, 1.0))
        .field(Field::slider("gap", "Separación", s.gap as f64, 0.0, 48.0, 1.0))
        .field(Field::toggle("autohide", "Autoesconder", s.autohide))
        .field(Field::slider(
            "panel_width",
            "Ancho de panel (sidebar)",
            s.panel_width as f64,
            120.0,
            600.0,
            10.0,
        ))
}

impl Configurable for Config {
    fn schema(&self) -> Schema {
        let mut schema = Schema::new().section(
            Section::new("general", "General")
                .icon("≡")
                .field(Field::text("timezone", "Zona horaria", self.general.timezone.clone())
                    .help("\"auto\" detecta del sistema; o un nombre IANA (America/Lima)")),
        );
        for (i, s) in self.surfaces.iter().enumerate() {
            schema = schema.section(surface_section(i, s));
        }
        schema
    }

    fn apply(&mut self, path: &FieldPath, value: FieldValue) -> Result<(), AllichayError> {
        let segs = path.segments();
        let unknown = || AllichayError::UnknownPath(path.to_string());
        match segs {
            [section, field] if section == "general" => match field.as_str() {
                "timezone" => {
                    if let Some(s) = value.as_str() {
                        self.general.timezone = s.to_string();
                    }
                    Ok(())
                }
                _ => Err(unknown()),
            },
            [section, field] if section.starts_with(SURFACE_PREFIX) => {
                let idx: usize = section[SURFACE_PREFIX.len()..]
                    .parse()
                    .map_err(|_| unknown())?;
                let surf = self.surfaces.get_mut(idx).ok_or_else(unknown)?;
                apply_surface_field(surf, field, value)
            }
            _ => Err(unknown()),
        }
    }
}

fn apply_surface_field(
    surf: &mut Surface,
    field: &str,
    value: FieldValue,
) -> Result<(), AllichayError> {
    match field {
        "kind" => {
            if let Some(k) = value.as_str().and_then(SurfaceKind::from_id) {
                surf.kind = k;
            }
        }
        "anchor" => {
            if let Some(a) = value.as_str().and_then(Anchor::from_id) {
                surf.anchor = a;
            }
        }
        "thickness" => {
            if let Some(v) = value.as_float() {
                surf.thickness = v as f32;
            }
        }
        "padding" => {
            if let Some(v) = value.as_float() {
                surf.padding = v as f32;
            }
        }
        "gap" => {
            if let Some(v) = value.as_float() {
                surf.gap = v as f32;
            }
        }
        "autohide" => {
            if let Some(b) = value.as_bool() {
                surf.autohide = b;
            }
        }
        "panel_width" => {
            if let Some(v) = value.as_float() {
                surf.panel_width = v as f32;
            }
        }
        _ => return Err(AllichayError::UnknownPath(field.to_string())),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_refleja_superficies() {
        let cfg = Config::preset();
        let schema = cfg.schema();
        // general + una sección por superficie.
        assert_eq!(schema.sections.len(), 1 + cfg.surfaces.len());
        assert_eq!(schema.sections[0].id, "general");
        assert_eq!(schema.sections[1].id, "surface0");
    }

    #[test]
    fn apply_edita_grosor_de_una_barra() {
        let mut cfg = Config::preset();
        let path = FieldPath::from("surface0.thickness");
        cfg.apply(&path, FieldValue::Float(48.0)).unwrap();
        assert_eq!(cfg.surfaces[0].thickness, 48.0);
    }

    #[test]
    fn apply_cambia_tipo_y_borde() {
        let mut cfg = Config::preset();
        cfg.apply(&"surface0.kind".into(), FieldValue::Enum("dock".into()))
            .unwrap();
        cfg.apply(&"surface0.anchor".into(), FieldValue::Enum("left".into()))
            .unwrap();
        assert_eq!(cfg.surfaces[0].kind, SurfaceKind::Dock);
        assert_eq!(cfg.surfaces[0].anchor, Anchor::Left);
    }

    #[test]
    fn apply_timezone() {
        let mut cfg = Config::preset();
        cfg.apply(&"general.timezone".into(), FieldValue::Text("America/Lima".into()))
            .unwrap();
        assert_eq!(cfg.general.timezone, "America/Lima");
    }

    #[test]
    fn apply_ruta_desconocida_es_error() {
        let mut cfg = Config::preset();
        assert!(cfg.apply(&"surface9.thickness".into(), FieldValue::Float(1.0)).is_err());
        assert!(cfg.apply(&"general.nope".into(), FieldValue::Bool(true)).is_err());
    }
}
