//! La config del compositor como configuración editable.
//!
//! Refleja [`Config`] en un [`allichay::Schema`] para que el panel de control la
//! pinte con dientes y controles, y la haga editable sin tocar `config.ron` a
//! mano. Los cambios vuelven por [`Configurable::apply`], que muta el modelo; el
//! panel persiste con [`Config::save`] y el `FileWatch` del compositor recarga
//! en caliente.
//!
//! v1 cubre los **escalares**: teselado, foco, decoración, fondo, terminal
//! dropdown y disposición de monitores. Los editores de **tabla** (keymap,
//! reglas) y de **listas** (menú raíz, zonas, overrides por salida) quedan para
//! v2 — siguen editándose por RON mientras tanto.

use allichay::{
    AllichayError, Configurable, EnumOption, Field, FieldPath, FieldValue, Schema, Section,
};

use mirada_layout::WallpaperFit;

use crate::action::{layout_from_slug, layout_slug};
use crate::config::Config;

/// Las opciones de modo de teselado (slug + rótulo).
fn layout_options() -> Vec<EnumOption> {
    vec![
        EnumOption::new("master-stack", "Maestra + pila"),
        EnumOption::new("centered-master", "Maestra centrada"),
        EnumOption::new("spiral", "Espiral"),
        EnumOption::new("grid", "Grilla"),
        EnumOption::new("columns", "Columnas"),
        EnumOption::new("rows", "Filas"),
        EnumOption::new("monocle", "Monóculo"),
    ]
}

/// Las opciones de ajuste del wallpaper (slug + rótulo).
fn wallpaper_fit_options() -> Vec<EnumOption> {
    vec![
        EnumOption::new("stretch", "Estirar"),
        EnumOption::new("fit", "Encajar"),
        EnumOption::new("fill", "Cubrir"),
        EnumOption::new("center", "Centrar"),
        EnumOption::new("tile", "Mosaico"),
    ]
}

impl Configurable for Config {
    fn schema(&self) -> Schema {
        Schema::new()
            .section(
                Section::new("teselado", "Teselado")
                    .icon("🎛")
                    .help("Cómo se acomodan las ventanas")
                    .field(Field::dropdown(
                        "layout",
                        "Modo",
                        layout_slug(self.layout),
                        layout_options(),
                    ))
                    .field(Field::slider_int("gap", "Margen", self.gap as i64, 0, 48))
                    .field(Field::slider(
                        "master_ratio",
                        "Ancho de la maestra",
                        self.master_ratio as f64,
                        0.05,
                        0.95,
                        0.05,
                    ))
                    .field(Field::slider_int(
                        "master_count",
                        "Ventanas maestras",
                        self.master_count as i64,
                        1,
                        6,
                    ))
                    .field(Field::slider(
                        "master_step",
                        "Paso de la maestra",
                        self.master_step as f64,
                        0.01,
                        0.5,
                        0.01,
                    ))
                    .field(Field::slider_int(
                        "float_step",
                        "Paso de flotantes (px)",
                        self.float_step as i64,
                        1,
                        200,
                    ))
                    .field(Field::toggle(
                        "focus_follows_mouse",
                        "El foco sigue al puntero",
                        self.focus_follows_mouse,
                    )),
            )
            .section(
                Section::new("decoracion", "Decoración")
                    .icon("🎨")
                    .help("Marco y barra de título de las ventanas")
                    .field(Field::slider_int(
                        "border_width",
                        "Grosor del marco",
                        self.border_width as i64,
                        0,
                        12,
                    ))
                    .field(Field::slider_int(
                        "titlebar_height",
                        "Alto de la barra de título",
                        self.titlebar_height as i64,
                        0,
                        48,
                    ))
                    .field(Field::color("border_focus", "Color con foco", self.border_focus))
                    .field(Field::color(
                        "border_normal",
                        "Color sin foco",
                        self.border_normal,
                    )),
            )
            .section(
                Section::new("fondo", "Fondo")
                    .icon("")
                    .help("Wallpaper y fuente del escritorio")
                    .field(Field::text(
                        "wallpaper_path",
                        "Imagen de fondo",
                        self.wallpaper_path.clone(),
                    ))
                    .field(Field::dropdown(
                        "wallpaper_fit",
                        "Ajuste",
                        self.wallpaper_fit.slug(),
                        wallpaper_fit_options(),
                    ))
                    .field(Field::text("font_path", "Fuente", self.font_path.clone())),
            )
            .section(
                Section::new("terminal", "Terminal")
                    .icon("⌨")
                    .help("La terminal desplegable (quake)")
                    .field(Field::text(
                        "dropterm_cmd",
                        "Comando",
                        self.dropterm_cmd.clone(),
                    ))
                    .field(Field::slider_int(
                        "dropterm_height_pct",
                        "Alto (% de pantalla)",
                        self.dropterm_height_pct as i64,
                        1,
                        100,
                    )),
            )
            .section(
                Section::new("monitores", "Monitores")
                    .icon("🖥")
                    .help("Disposición de varias salidas")
                    .field(Field::dropdown(
                        "output_direction",
                        "Disposición",
                        self.output_direction.clone(),
                        vec![
                            EnumOption::new("horizontal", "Horizontal"),
                            EnumOption::new("vertical", "Vertical"),
                        ],
                    )),
            )
    }

    fn apply(&mut self, path: &FieldPath, value: FieldValue) -> Result<(), AllichayError> {
        let unknown = || AllichayError::UnknownPath(path.to_string());
        match path.leaf().ok_or_else(unknown)? {
            "layout" => {
                if let Some(m) = value.as_str().and_then(layout_from_slug) {
                    self.layout = m;
                }
            }
            "gap" => {
                if let Some(v) = value.as_int() {
                    self.gap = v as i32;
                }
            }
            "master_ratio" => {
                if let Some(v) = value.as_float() {
                    self.master_ratio = v as f32;
                }
            }
            "master_count" => {
                if let Some(v) = value.as_int() {
                    self.master_count = v.max(1) as usize;
                }
            }
            "master_step" => {
                if let Some(v) = value.as_float() {
                    self.master_step = v as f32;
                }
            }
            "float_step" => {
                if let Some(v) = value.as_int() {
                    self.float_step = v as i32;
                }
            }
            "focus_follows_mouse" => {
                if let Some(b) = value.as_bool() {
                    self.focus_follows_mouse = b;
                }
            }
            "border_width" => {
                if let Some(v) = value.as_int() {
                    self.border_width = v as i32;
                }
            }
            "titlebar_height" => {
                if let Some(v) = value.as_int() {
                    self.titlebar_height = v as i32;
                }
            }
            "border_focus" => {
                if let Some(c) = value.as_color() {
                    self.border_focus = c;
                }
            }
            "border_normal" => {
                if let Some(c) = value.as_color() {
                    self.border_normal = c;
                }
            }
            "wallpaper_path" => {
                if let Some(s) = value.as_str() {
                    self.wallpaper_path = s.to_string();
                }
            }
            "wallpaper_fit" => {
                if let Some(f) = value.as_str().and_then(WallpaperFit::from_slug) {
                    self.wallpaper_fit = f;
                }
            }
            "font_path" => {
                if let Some(s) = value.as_str() {
                    self.font_path = s.to_string();
                }
            }
            "dropterm_cmd" => {
                if let Some(s) = value.as_str() {
                    self.dropterm_cmd = s.to_string();
                }
            }
            "dropterm_height_pct" => {
                if let Some(v) = value.as_int() {
                    self.dropterm_height_pct = v.clamp(1, 100) as u32;
                }
            }
            "output_direction" => {
                if let Some(s) = value.as_str() {
                    // Sólo aceptamos los slugs válidos; otro valor se ignora.
                    if s == "horizontal" || s == "vertical" {
                        self.output_direction = s.to_string();
                    }
                }
            }
            _ => return Err(unknown()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirada_layout::LayoutMode;

    #[test]
    fn schema_tiene_las_secciones_escalares() {
        let schema = Config::default().schema();
        let ids: Vec<&str> = schema.sections.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["teselado", "decoracion", "fondo", "terminal", "monitores"]
        );
    }

    #[test]
    fn apply_layout_por_slug() {
        let mut c = Config::default();
        c.apply(&"teselado.layout".into(), FieldValue::Enum("grid".into()))
            .unwrap();
        assert_eq!(c.layout, LayoutMode::Grid);
    }

    #[test]
    fn apply_escalares_y_color() {
        let mut c = Config::default();
        c.apply(&"teselado.gap".into(), FieldValue::Int(16)).unwrap();
        c.apply(&"teselado.master_ratio".into(), FieldValue::Float(0.7))
            .unwrap();
        c.apply(&"decoracion.border_focus".into(), FieldValue::Color([1, 2, 3, 255]))
            .unwrap();
        assert_eq!(c.gap, 16);
        assert!((c.master_ratio - 0.7).abs() < 1e-6);
        assert_eq!(c.border_focus, [1, 2, 3, 255]);
    }

    #[test]
    fn apply_wallpaper_fit_y_texto() {
        let mut c = Config::default();
        c.apply(&"fondo.wallpaper_fit".into(), FieldValue::Enum("fill".into()))
            .unwrap();
        c.apply(&"fondo.wallpaper_path".into(), FieldValue::Text("/w.png".into()))
            .unwrap();
        assert_eq!(c.wallpaper_fit, WallpaperFit::Fill);
        assert_eq!(c.wallpaper_path, "/w.png");
    }

    #[test]
    fn apply_dropterm_height_se_acota() {
        let mut c = Config::default();
        c.apply(&"terminal.dropterm_height_pct".into(), FieldValue::Int(250))
            .unwrap();
        assert_eq!(c.dropterm_height_pct, 100);
    }

    #[test]
    fn apply_ruta_desconocida_es_error() {
        let mut c = Config::default();
        assert!(c.apply(&"teselado.nope".into(), FieldValue::Int(1)).is_err());
    }
}
