//! `allichay` — el vocabulario declarativo de configuración.
//!
//! `allichay` (quechua: *arreglar*, *poner en orden*, *configurar*) es la capa
//! que vuelve la configuración de una app **datos** en vez de pantallas a mano.
//! Una app describe QUÉ es configurable como un [`Schema`]; un renderizador
//! único (`llimphi-module-allichay`) lo pinta con dientes y controles; y los
//! cambios vuelven a la app como un [`FieldPath`] + [`FieldValue`] que ella
//! aplica a su propio struct y persiste en su propio formato. allichay no sabe
//! quién la pinta ni dónde se guarda.
//!
//! La unidad de navegación es la [`Section`] (un *diente* del rail), que puede
//! anidar [`Section::subsections`]. Cada [`Field`] lleva su valor actual
//! ([`FieldValue`]) y la pista de cómo editarlo ([`Control`]).
//!
//! ```
//! use allichay::{Schema, Section, Field};
//! let schema = Schema::new().section(
//!     Section::new("apariencia", "Apariencia").icon("◐")
//!         .field(Field::toggle("oscuro", "Modo oscuro", true))
//!         .field(Field::slider("gap", "Margen", 8.0, 0.0, 32.0, 1.0)),
//! );
//! assert!(schema.find_field(&"apariencia.oscuro".into()).is_some());
//! ```
//!
//! Es `no_std + alloc`: el mismo vocabulario sirve al frontend Llimphi sobre
//! Linux y, a futuro, al kernel launcher de wawa (`x86_64-unknown-none`).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// =====================================================================
// Valor de un campo
// =====================================================================

/// El valor actual de un [`Field`], agnóstico del tipo Rust de origen. La app
/// traduce su struct a esto al construir el [`Schema`], y de esto a su struct al
/// recibir un cambio en [`Configurable::apply`].
///
/// v1 cubre los **escalares**: booleano, entero, flotante, texto, selección de
/// enum (el id elegido) y color RGBA. Las variantes de tabla y lista quedan para
/// v2 (keymaps, reglas, listas de superficies).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum FieldValue {
    /// Booleano — se edita con un [`Control::Toggle`].
    Bool(bool),
    /// Entero — típicamente con un [`Control::Slider`] (paso entero).
    Int(i64),
    /// Flotante — con un [`Control::Slider`].
    Float(f64),
    /// Texto libre — con un [`Control::TextInput`].
    Text(String),
    /// El id de la opción elegida de un [`Control::Dropdown`].
    Enum(String),
    /// Color RGBA — con un [`Control::ColorPicker`].
    Color([u8; 4]),
}

impl FieldValue {
    /// El nombre del tipo, para mensajes de error.
    pub fn type_name(&self) -> &'static str {
        match self {
            FieldValue::Bool(_) => "bool",
            FieldValue::Int(_) => "int",
            FieldValue::Float(_) => "float",
            FieldValue::Text(_) => "text",
            FieldValue::Enum(_) => "enum",
            FieldValue::Color(_) => "color",
        }
    }

    /// Lee el valor como booleano, si lo es.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            FieldValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Lee el valor como entero. Un [`FieldValue::Float`] se trunca; un
    /// [`FieldValue::Bool`] no se promueve (devuelve `None`).
    pub fn as_int(&self) -> Option<i64> {
        match self {
            FieldValue::Int(i) => Some(*i),
            FieldValue::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    /// Lee el valor como flotante. Un [`FieldValue::Int`] se promueve.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            FieldValue::Float(f) => Some(*f),
            FieldValue::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Lee el valor como `&str` (texto o id de enum).
    pub fn as_str(&self) -> Option<&str> {
        match self {
            FieldValue::Text(s) | FieldValue::Enum(s) => Some(s),
            _ => None,
        }
    }

    /// Lee el valor como color RGBA, si lo es.
    pub fn as_color(&self) -> Option<[u8; 4]> {
        match self {
            FieldValue::Color(c) => Some(*c),
            _ => None,
        }
    }
}

// =====================================================================
// Control: cómo se edita un campo
// =====================================================================

/// Una opción de un [`Control::Dropdown`]: un id estable (lo que viaja en
/// [`FieldValue::Enum`]) y su rótulo visible.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EnumOption {
    /// Id estable de la opción (no se traduce).
    pub id: String,
    /// Rótulo visible (puede traducirse al construir el schema).
    pub label: String,
}

impl EnumOption {
    /// Una opción con id y rótulo.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// La pista de UI de cómo se edita un [`Field`]. El renderizador elige el
/// widget concreto; la app sólo declara la intención.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Control {
    /// Interruptor on/off para un [`FieldValue::Bool`].
    Toggle,
    /// Deslizador acotado para [`FieldValue::Int`] / [`FieldValue::Float`].
    Slider {
        /// Mínimo permitido.
        min: f64,
        /// Máximo permitido.
        max: f64,
        /// Paso del deslizador (1.0 para enteros).
        step: f64,
    },
    /// Selección entre opciones fijas para un [`FieldValue::Enum`].
    Dropdown {
        /// Las opciones disponibles, en orden.
        options: Vec<EnumOption>,
    },
    /// Selector de color RGBA para un [`FieldValue::Color`].
    ColorPicker,
    /// Campo de texto libre para un [`FieldValue::Text`].
    TextInput,
}

// =====================================================================
// Field
// =====================================================================

/// Un campo configurable: su id (estable, para el [`FieldPath`]), su rótulo, una
/// ayuda opcional, el valor actual y el control para editarlo.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Field {
    /// Id estable dentro de su sección. Es el último segmento del [`FieldPath`].
    pub id: String,
    /// Rótulo visible.
    pub label: String,
    /// Ayuda corta (puede ir vacía).
    #[cfg_attr(feature = "serde", serde(default))]
    pub help: String,
    /// El valor actual.
    pub value: FieldValue,
    /// Cómo se edita.
    pub control: Control,
}

impl Field {
    /// Un campo crudo (raro de usar directo; preferí los constructores por tipo).
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        value: FieldValue,
        control: Control,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            help: String::new(),
            value,
            control,
        }
    }

    /// Añade una ayuda corta (encadenable).
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = help.into();
        self
    }

    /// Un interruptor booleano.
    pub fn toggle(id: impl Into<String>, label: impl Into<String>, value: bool) -> Self {
        Self::new(id, label, FieldValue::Bool(value), Control::Toggle)
    }

    /// Un deslizador flotante acotado.
    pub fn slider(
        id: impl Into<String>,
        label: impl Into<String>,
        value: f64,
        min: f64,
        max: f64,
        step: f64,
    ) -> Self {
        Self::new(
            id,
            label,
            FieldValue::Float(value),
            Control::Slider { min, max, step },
        )
    }

    /// Un deslizador entero acotado (paso 1).
    pub fn slider_int(
        id: impl Into<String>,
        label: impl Into<String>,
        value: i64,
        min: i64,
        max: i64,
    ) -> Self {
        Self::new(
            id,
            label,
            FieldValue::Int(value),
            Control::Slider {
                min: min as f64,
                max: max as f64,
                step: 1.0,
            },
        )
    }

    /// Una selección entre opciones fijas. `selected` es el id actual.
    pub fn dropdown(
        id: impl Into<String>,
        label: impl Into<String>,
        selected: impl Into<String>,
        options: Vec<EnumOption>,
    ) -> Self {
        Self::new(
            id,
            label,
            FieldValue::Enum(selected.into()),
            Control::Dropdown { options },
        )
    }

    /// Un campo de texto libre.
    pub fn text(id: impl Into<String>, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(id, label, FieldValue::Text(value.into()), Control::TextInput)
    }

    /// Un selector de color RGBA.
    pub fn color(id: impl Into<String>, label: impl Into<String>, value: [u8; 4]) -> Self {
        Self::new(id, label, FieldValue::Color(value), Control::ColorPicker)
    }
}

// =====================================================================
// Section
// =====================================================================

/// Una sección de configuración: un *diente* del rail. Agrupa campos y puede
/// anidar subsecciones (las "subsecciones" del panel).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Section {
    /// Id estable de la sección. Es un segmento del [`FieldPath`].
    pub id: String,
    /// Título visible.
    pub title: String,
    /// Identificador de icono del diente (`"◐"`, `"settings"`…). Conjunto
    /// abierto: el renderizador cae a un default si no lo conoce.
    #[cfg_attr(feature = "serde", serde(default))]
    pub icon: String,
    /// Ayuda/subtítulo de la sección (puede ir vacía).
    #[cfg_attr(feature = "serde", serde(default))]
    pub help: String,
    /// Los campos directos de la sección.
    #[cfg_attr(feature = "serde", serde(default))]
    pub fields: Vec<Field>,
    /// Subsecciones anidadas.
    #[cfg_attr(feature = "serde", serde(default))]
    pub subsections: Vec<Section>,
}

impl Section {
    /// Una sección con id y título, sin campos.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            icon: String::new(),
            help: String::new(),
            fields: Vec::new(),
            subsections: Vec::new(),
        }
    }

    /// Fija el icono del diente (encadenable).
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = icon.into();
        self
    }

    /// Fija la ayuda/subtítulo (encadenable).
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = help.into();
        self
    }

    /// Añade un campo (encadenable).
    pub fn field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }

    /// Añade una subsección (encadenable).
    pub fn subsection(mut self, section: Section) -> Self {
        self.subsections.push(section);
        self
    }

    /// Busca un campo por su ruta **relativa** a esta sección (el primer
    /// segmento ya debe corresponder a un campo o subsección de aquí).
    fn find_rel(&self, segments: &[String]) -> Option<&Field> {
        match segments {
            [] => None,
            [field_id] => self.fields.iter().find(|f| &f.id == field_id),
            [sub_id, rest @ ..] => self
                .subsections
                .iter()
                .find(|s| &s.id == sub_id)
                .and_then(|s| s.find_rel(rest)),
        }
    }
}

// =====================================================================
// Schema
// =====================================================================

/// El esquema completo de configuración de una app: una lista de secciones.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Schema {
    /// Las secciones top-level (los dientes del rail).
    pub sections: Vec<Section>,
}

impl Schema {
    /// Un esquema vacío.
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    /// Añade una sección top-level (encadenable).
    pub fn section(mut self, section: Section) -> Self {
        self.sections.push(section);
        self
    }

    /// Busca un campo por su [`FieldPath`] absoluto (sección[.subsección…].campo).
    pub fn find_field(&self, path: &FieldPath) -> Option<&Field> {
        match path.segments() {
            [] => None,
            [section_id, rest @ ..] => self
                .sections
                .iter()
                .find(|s| &s.id == section_id)
                .and_then(|s| s.find_rel(rest)),
        }
    }

    /// `true` si la ruta apunta a un campo existente.
    pub fn contains(&self, path: &FieldPath) -> bool {
        self.find_field(path).is_some()
    }
}

// =====================================================================
// FieldPath
// =====================================================================

/// La ruta a un campo dentro de un [`Schema`]: los ids de sección, subsecciones
/// y el campo, separados por `.`. P. ej. `"apariencia.bordes.color_foco"`.
///
/// Es un newtype sobre `Vec<String>` para llevar helpers; se construye fácil
/// desde un `&str` con `.into()`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FieldPath(pub Vec<String>);

impl FieldPath {
    /// Una ruta vacía.
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Los segmentos de la ruta.
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    /// Añade un segmento (encadenable).
    pub fn push(mut self, seg: impl Into<String>) -> Self {
        self.0.push(seg.into());
        self
    }

    /// El último segmento (el id del campo), si la ruta no está vacía.
    pub fn leaf(&self) -> Option<&str> {
        self.0.last().map(String::as_str)
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            f.write_str(seg)?;
        }
        Ok(())
    }
}

impl From<&str> for FieldPath {
    fn from(s: &str) -> Self {
        FieldPath(s.split('.').map(ToString::to_string).collect())
    }
}

impl From<String> for FieldPath {
    fn from(s: String) -> Self {
        FieldPath::from(s.as_str())
    }
}

// =====================================================================
// Configurable
// =====================================================================

/// Lo que implementa la config de una app para volverse editable: describe su
/// [`Schema`] y aplica un cambio puntual. El renderizador llama `schema()` para
/// pintar y, ante cada edición, devuelve el `(FieldPath, FieldValue)` que la app
/// pasa a `apply()` antes de persistir con su propio `save()`.
pub trait Configurable {
    /// Describe la configuración actual como un esquema editable.
    fn schema(&self) -> Schema;

    /// Aplica un cambio puntual. La implementación valida la ruta y el tipo, y
    /// puede acotar el valor a su rango válido en vez de rechazarlo.
    fn apply(&mut self, path: &FieldPath, value: FieldValue) -> Result<(), AllichayError>;
}

/// Error al aplicar un cambio de configuración.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllichayError {
    /// La ruta no corresponde a ningún campo conocido.
    UnknownPath(String),
    /// El tipo del valor no calza con el campo destino.
    TypeMismatch {
        /// El tipo que el campo esperaba.
        expected: &'static str,
        /// El tipo que llegó.
        got: &'static str,
    },
}

impl fmt::Display for AllichayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AllichayError::UnknownPath(p) => write!(f, "ruta de config desconocida: {p}"),
            AllichayError::TypeMismatch { expected, got } => {
                write!(f, "tipo de valor incorrecto: se esperaba {expected}, llegó {got}")
            }
        }
    }
}

// No implementamos `core::error::Error` (estabilizado recién en 1.81, y el
// workspace pide rust-version 1.80): los consumidores `std` mapean el `Display`
// a su propio error si lo necesitan.

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn schema_demo() -> Schema {
        Schema::new()
            .section(
                Section::new("apariencia", "Apariencia")
                    .icon("◐")
                    .field(Field::toggle("oscuro", "Modo oscuro", true))
                    .field(Field::slider("gap", "Margen", 8.0, 0.0, 32.0, 1.0))
                    .subsection(
                        Section::new("bordes", "Bordes")
                            .field(Field::color("foco", "Color de foco", [92, 143, 235, 255])),
                    ),
            )
            .section(
                Section::new("idioma", "Idioma").icon("✦").field(Field::dropdown(
                    "locale",
                    "Idioma",
                    "es-PE",
                    vec![
                        EnumOption::new("es-PE", "Español"),
                        EnumOption::new("en-US", "English"),
                    ],
                )),
            )
    }

    #[test]
    fn find_field_top_level() {
        let s = schema_demo();
        let f = s.find_field(&"apariencia.gap".into()).unwrap();
        assert_eq!(f.id, "gap");
        assert_eq!(f.value, FieldValue::Float(8.0));
    }

    #[test]
    fn find_field_anidado() {
        let s = schema_demo();
        let f = s.find_field(&"apariencia.bordes.foco".into()).unwrap();
        assert_eq!(f.value, FieldValue::Color([92, 143, 235, 255]));
    }

    #[test]
    fn ruta_desconocida_no_encuentra() {
        let s = schema_demo();
        assert!(!s.contains(&"apariencia.no_existe".into()));
        assert!(!s.contains(&"otra.cosa".into()));
        assert!(s.find_field(&FieldPath::empty()).is_none());
    }

    #[test]
    fn fieldpath_round_trip_string() {
        let p: FieldPath = "a.b.c".into();
        assert_eq!(p.segments().len(), 3);
        assert_eq!(p.leaf(), Some("c"));
        assert_eq!(p.to_string(), "a.b.c");
    }

    #[test]
    fn fieldvalue_promociones() {
        assert_eq!(FieldValue::Int(3).as_float(), Some(3.0));
        assert_eq!(FieldValue::Float(3.9).as_int(), Some(3));
        assert_eq!(FieldValue::Bool(true).as_int(), None);
        assert_eq!(FieldValue::Enum("x".into()).as_str(), Some("x"));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn schema_serde_round_trip() {
        // El espejo es por JSON sólo en el test (allichay no fija formato).
        let s = schema_demo();
        let json = serde_json::to_string(&s).unwrap();
        let back: Schema = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
