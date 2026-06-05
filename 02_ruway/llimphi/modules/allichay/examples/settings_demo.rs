//! Demo del renderizador `allichay`: una config de juguete editable.
//!
//! Ejercita el módulo sin levantar ninguna app de dominio: un `DemoConfig`
//! implementa [`allichay::Configurable`], el renderizador lo pinta con dientes
//! y controles, y cada cambio se aplica + se loguea por consola.
//!
//! ```bash
//! cargo run -p llimphi-module-allichay --example settings_demo --release
//! ```

use allichay::{Column, Configurable, EnumOption, Field, FieldPath, FieldValue, Schema, Section};
use llimphi_module_allichay::{allichay_view, AllichayMsg, AllichayState};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::{App, Handle, KeyEvent, View};

/// Config de juguete con un campo de cada tipo.
#[derive(Clone)]
struct DemoConfig {
    oscuro: bool,
    gap: f64,
    columnas: i64,
    idioma: String,
    acento: [u8; 4],
    nombre: String,
    auto: bool,
    /// Lista de textos (ejercita [`Control::List`]).
    rutas: Vec<String>,
    /// Tabla (etiqueta, comando) — ejercita [`Control::Table`].
    accesos: Vec<(String, String)>,
}

impl Default for DemoConfig {
    fn default() -> Self {
        Self {
            oscuro: true,
            gap: 8.0,
            columnas: 2,
            idioma: "es-PE".into(),
            acento: [92, 143, 235, 255],
            nombre: "mi escritorio".into(),
            auto: false,
            rutas: vec!["~/proyectos".into(), "/usr/share".into()],
            accesos: vec![
                ("Editor".into(), "nada".into()),
                ("Terminal".into(), "alacritty".into()),
            ],
        }
    }
}

impl Configurable for DemoConfig {
    fn schema(&self) -> Schema {
        Schema::new()
            .section(
                Section::new("apariencia", "Apariencia")
                    .icon("◐")
                    .help("Cómo se ve el escritorio")
                    .field(Field::toggle("oscuro", "Modo oscuro", self.oscuro))
                    .field(
                        Field::slider("gap", "Margen entre ventanas", self.gap, 0.0, 32.0, 1.0)
                            .help("En píxeles"),
                    )
                    .field(Field::color("acento", "Color de acento", self.acento))
                    .subsection(
                        Section::new("teselado", "Teselado").field(Field::slider_int(
                            "columnas",
                            "Columnas",
                            self.columnas,
                            1,
                            6,
                        )),
                    ),
            )
            .section(
                Section::new("general", "General")
                    .icon("≡")
                    .field(Field::dropdown(
                        "idioma",
                        "Idioma",
                        self.idioma.clone(),
                        vec![
                            EnumOption::new("es-PE", "Español"),
                            EnumOption::new("en-US", "English"),
                            EnumOption::new("qu-PE", "Runasimi"),
                        ],
                    ))
                    .field(Field::text("nombre", "Nombre del equipo", self.nombre.clone()))
                    .field(Field::toggle("auto", "Arrancar al inicio", self.auto)),
            )
            .section(
                Section::new("agregados", "Listas y tablas")
                    .icon("≣")
                    .help("Los controles v2")
                    .field(Field::list(
                        "rutas",
                        "Rutas de búsqueda",
                        self.rutas.clone(),
                        "ruta",
                    ))
                    .field(Field::table(
                        "accesos",
                        "Accesos directos",
                        vec![Column::new("label", "Nombre"), Column::new("cmd", "Comando")],
                        self.accesos
                            .iter()
                            .map(|(l, c)| vec![l.clone(), c.clone()])
                            .collect(),
                    )),
            )
    }

    fn apply(
        &mut self,
        path: &FieldPath,
        value: FieldValue,
    ) -> Result<(), allichay::AllichayError> {
        match path.leaf() {
            Some("oscuro") => self.oscuro = value.as_bool().unwrap_or(self.oscuro),
            Some("gap") => self.gap = value.as_float().unwrap_or(self.gap),
            Some("columnas") => self.columnas = value.as_int().unwrap_or(self.columnas),
            Some("idioma") => {
                if let Some(s) = value.as_str() {
                    self.idioma = s.to_string();
                }
            }
            Some("acento") => self.acento = value.as_color().unwrap_or(self.acento),
            Some("nombre") => {
                if let Some(s) = value.as_str() {
                    self.nombre = s.to_string();
                }
            }
            Some("auto") => self.auto = value.as_bool().unwrap_or(self.auto),
            Some("rutas") => {
                if let Some(items) = value.as_list() {
                    self.rutas = items.to_vec();
                }
            }
            Some("accesos") => {
                if let Some(rows) = value.as_table() {
                    self.accesos = rows
                        .iter()
                        .map(|r| {
                            (
                                r.first().cloned().unwrap_or_default(),
                                r.get(1).cloned().unwrap_or_default(),
                            )
                        })
                        .collect();
                }
            }
            other => {
                return Err(allichay::AllichayError::UnknownPath(
                    other.unwrap_or("").into(),
                ))
            }
        }
        Ok(())
    }
}

struct Model {
    cfg: DemoConfig,
    state: AllichayState,
}

#[derive(Clone)]
enum Msg {
    Allichay(AllichayMsg),
    Key(KeyEvent),
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "allichay · demo"
    }

    fn initial_size() -> (u32, u32) {
        (760, 560)
    }

    fn init(_handle: &Handle<Msg>) -> Model {
        Model {
            cfg: DemoConfig::default(),
            state: AllichayState::new(),
        }
    }

    fn update(model: Model, msg: Msg, _handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Allichay(AllichayMsg::SelectSection(i)) => m.state.select(i),
            Msg::Allichay(AllichayMsg::Focus(path)) => {
                // Sembrar el buffer con el valor actual del campo.
                let seed = m
                    .cfg
                    .schema()
                    .find_field(&path)
                    .and_then(|f| f.value.as_str().map(str::to_string))
                    .unwrap_or_default();
                m.state.focus(&path, &seed);
            }
            Msg::Allichay(AllichayMsg::FocusCell(path, row, col)) => {
                // El estado siembra el buffer leyendo la celda del valor actual.
                if let Some(f) = m.cfg.schema().find_field(&path) {
                    m.state.focus_cell(&path, f.value.clone(), row, col);
                }
            }
            Msg::Allichay(AllichayMsg::FocusHex(path)) => {
                let seed = m
                    .cfg
                    .schema()
                    .find_field(&path)
                    .and_then(|f| f.value.as_color())
                    .map(llimphi_module_allichay::color_hex)
                    .unwrap_or_default();
                m.state.focus_hex(&path, &seed);
            }
            Msg::Allichay(AllichayMsg::Change(path, value)) => {
                println!("cambio: {path} = {value:?}");
                if let Err(e) = m.cfg.apply(&path, value) {
                    eprintln!("  error: {e}");
                }
            }
            Msg::Allichay(AllichayMsg::ScrollTo(offset)) => m.state.set_scroll(offset),
            Msg::Key(event) => {
                if let Some((path, value)) = m.state.apply_key(&event) {
                    println!("texto: {path} = {value:?}");
                    let _ = m.cfg.apply(&path, value);
                }
            }
        }
        m
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        // Sólo capturamos teclas cuando hay un campo de texto en edición.
        if model.state.is_editing() {
            Some(Msg::Key(event.clone()))
        } else {
            None
        }
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let schema = model.cfg.schema();
        let body = allichay_view(&schema, &model.state, &theme, Msg::Allichay);
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![body])
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
