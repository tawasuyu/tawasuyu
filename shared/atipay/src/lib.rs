//! `atipay` — el catálogo de capacidades invocables de la suite.
//!
//! El problema que resuelve: una IA (la de `shuma`) que quiere «manejar muchas
//! cosas» no puede hardcodear cada superficie de control. Cada una expone lo
//! suyo por su lado — `mirada-ctl actions`, `sandokan-cli list`, los builtins
//! de shuma, los nombres D-Bus. `atipay` las **agrega en un solo catálogo
//! determinista**: una lista plana de [`Capacidad`]es que la IA consulta de un
//! saque (proyectada a definiciones tool-use con [`Catalogo::as_tools`]) y, dada
//! una [`Invocacion`] elegida por el modelo, traduce a un [`Plan`] ejecutable
//! ([`Catalogo::plan`]).
//!
//! **Doctrina (de `shuma/INTELIGENCIA.md`): determinista primero, LLM opcional
//! después.** `atipay` es 100% datos: enumera capacidades y arma planes, pero
//! **no ejecuta nada** (no spawnea procesos, no abre sockets). El LLM sólo
//! *elige* sobre el catálogo; quien corre el plan es el llamador (shuma ya
//! ejecuta comandos). Así el catálogo es puro, testeable y `no`-sorpresas.
//!
//! Arquitectura: cada superficie implementa [`FuenteCapacidades`] (qué sabe
//! hacer + cómo traducir una invocación a un plan). El [`Catalogo`] registra
//! las fuentes y rutea por el prefijo del `id` (`mirada.*`, `sandokan.*`…).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Dónde se ejecuta una capacidad. Determina a qué fuente rutea el catálogo y
/// le da a la IA el contexto de qué subsistema está tocando.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Superficie {
    /// El compositor / escritorio (`mirada-ctl`): ventanas, escritorios, layout.
    Mirada,
    /// El plano de control de procesos/servicios (`sandokan`): run/stop/observar.
    Sandokan,
    /// El propio shell (`shuma`): builtins, macros, workspaces.
    Shuma,
    /// El sistema operativo por interfaces estándar (D-Bus: energía, red, hora…).
    Sistema,
}

impl Superficie {
    /// El prefijo de `id` que reclama esta superficie (`"mirada"`, `"sandokan"`…).
    pub fn prefijo(self) -> &'static str {
        match self {
            Superficie::Mirada => "mirada",
            Superficie::Sandokan => "sandokan",
            Superficie::Shuma => "shuma",
            Superficie::Sistema => "sistema",
        }
    }
}

/// Cuánto cuesta equivocarse con una capacidad. La IA usa esto para decidir si
/// pedir confirmación al usuario antes de ejecutar (no es permiso: es prudencia).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Peligro {
    /// Sólo lee o cambia algo trivial y reversible al instante (enfocar, listar).
    Seguro,
    /// Cambia estado pero se deshace fácil (mover ventana, cambiar layout).
    Reversible,
    /// Acción que cuesta o no se deshace sin más (cerrar, apagar, parar servicio).
    Disruptivo,
}

/// El tipo de un parámetro de una capacidad. Guía el `input_schema` tool-use y
/// la validación del plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoParam {
    Texto,
    Entero,
    /// Id de una ventana (lo entrega `mirada-ctl windows`).
    IdVentana,
    /// Id (ULID) de una Card en ejecución (lo entrega `sandokan list`).
    IdCard,
    /// Una de un conjunto cerrado de opciones.
    Enum(Vec<String>),
}

/// Un parámetro de una capacidad.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    pub nombre: String,
    pub tipo: TipoParam,
    pub requerido: bool,
    pub descripcion: String,
}

impl Param {
    /// Parámetro requerido de texto libre.
    pub fn texto(nombre: &str, descripcion: &str) -> Self {
        Self { nombre: nombre.into(), tipo: TipoParam::Texto, requerido: true, descripcion: descripcion.into() }
    }
    /// Parámetro requerido entero.
    pub fn entero(nombre: &str, descripcion: &str) -> Self {
        Self { nombre: nombre.into(), tipo: TipoParam::Entero, requerido: true, descripcion: descripcion.into() }
    }
    /// Marca el parámetro como opcional.
    pub fn opcional(mut self) -> Self {
        self.requerido = false;
        self
    }
}

/// Una cosa que el sistema *puede hacer*. La unidad del catálogo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capacidad {
    /// Identificador estable y único, prefijado por superficie: `"mirada.workspace"`,
    /// `"sandokan.run"`. Es lo que la IA devuelve en una [`Invocacion`].
    pub id: String,
    pub superficie: Superficie,
    /// Frase corta de qué hace, en lenguaje natural (la lee el modelo).
    pub resumen: String,
    pub params: Vec<Param>,
    pub peligro: Peligro,
}

impl Capacidad {
    /// Constructor breve para fuentes que autorían capacidades como datos.
    pub fn nueva(superficie: Superficie, sufijo: &str, resumen: &str, peligro: Peligro, params: Vec<Param>) -> Self {
        Self { id: format!("{}.{}", superficie.prefijo(), sufijo), superficie, resumen: resumen.into(), params, peligro }
    }
}

/// La elección de la IA: qué capacidad invocar y con qué argumentos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocacion {
    pub id: String,
    #[serde(default)]
    pub args: BTreeMap<String, String>,
}

impl Invocacion {
    pub fn nueva(id: &str) -> Self {
        Self { id: id.into(), args: BTreeMap::new() }
    }
    pub fn con(mut self, clave: &str, valor: &str) -> Self {
        self.args.insert(clave.into(), valor.into());
        self
    }
    /// Lee un argumento requerido, o un error legible si falta.
    pub fn arg(&self, clave: &str) -> Result<&str, AtipayError> {
        self.args.get(clave).map(String::as_str).ok_or_else(|| AtipayError::FaltaArg {
            id: self.id.clone(),
            arg: clave.to_string(),
        })
    }
}

/// El resultado determinista de resolver una invocación: el comando concreto a
/// ejecutar (programa + args), sin ejecutarlo. El llamador (shuma) lo corre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// La capacidad que se resolvió (para auditar / confirmar).
    pub id: String,
    /// Programa a invocar (`"mirada-ctl"`, `"sandokan-cli"`…).
    pub programa: String,
    pub args: Vec<String>,
    /// Heredado de la capacidad: la IA/UI decide si confirmar antes de correr.
    pub peligro: Peligro,
}

/// Errores de resolución del catálogo. Mensajes ya legibles para el usuario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AtipayError {
    /// No hay ninguna capacidad con ese `id` en el catálogo.
    Desconocida(String),
    /// Falta un argumento requerido.
    FaltaArg { id: String, arg: String },
    /// Un argumento no es válido para su tipo (p.ej. esperaba entero).
    ArgInvalido { id: String, arg: String, motivo: String },
}

impl std::fmt::Display for AtipayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AtipayError::Desconocida(id) => write!(f, "capacidad desconocida: '{id}'"),
            AtipayError::FaltaArg { id, arg } => write!(f, "falta el argumento '{arg}' para '{id}'"),
            AtipayError::ArgInvalido { id, arg, motivo } => {
                write!(f, "argumento '{arg}' inválido para '{id}': {motivo}")
            }
        }
    }
}

impl std::error::Error for AtipayError {}

/// Lo que cada superficie de control implementa: qué sabe hacer y cómo traducir
/// una invocación a un plan ejecutable.
pub trait FuenteCapacidades: Send + Sync {
    /// Qué superficie cubre esta fuente (para el ruteo por prefijo).
    fn superficie(&self) -> Superficie;
    /// El catálogo de capacidades de esta superficie.
    fn capacidades(&self) -> Vec<Capacidad>;
    /// Traduce una invocación (ya sabida que es de esta superficie) a un plan.
    fn plan(&self, inv: &Invocacion) -> Result<Plan, AtipayError>;
}

/// El catálogo agregado: junta varias fuentes y rutea invocaciones por el
/// prefijo del `id`. Es el punto único que consulta la IA.
#[derive(Default)]
pub struct Catalogo {
    fuentes: Vec<Box<dyn FuenteCapacidades>>,
}

impl Catalogo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra una fuente. El orden de registro es el orden del listado.
    pub fn registrar(&mut self, fuente: Box<dyn FuenteCapacidades>) -> &mut Self {
        self.fuentes.push(fuente);
        self
    }

    /// El catálogo con las fuentes compiladas por defecto (las de los features
    /// activos). Es el arranque normal: `Catalogo::estandar()`.
    pub fn estandar() -> Self {
        let mut c = Self::new();
        #[cfg(feature = "mirada")]
        c.registrar(Box::new(crate::mirada::FuenteMirada));
        #[cfg(feature = "sandokan")]
        c.registrar(Box::new(crate::sandokan::FuenteSandokan));
        c
    }

    /// Todas las capacidades de todas las fuentes, en una lista plana.
    pub fn capacidades(&self) -> Vec<Capacidad> {
        self.fuentes.iter().flat_map(|f| f.capacidades()).collect()
    }

    /// Resuelve una invocación a un plan, ruteando por el prefijo del `id` a la
    /// fuente cuya superficie lo reclama.
    pub fn plan(&self, inv: &Invocacion) -> Result<Plan, AtipayError> {
        let prefijo = inv.id.split('.').next().unwrap_or("");
        let fuente = self
            .fuentes
            .iter()
            .find(|f| f.superficie().prefijo() == prefijo)
            .ok_or_else(|| AtipayError::Desconocida(inv.id.clone()))?;
        fuente.plan(inv)
    }

    /// Proyecta el catálogo a un array de definiciones **tool-use** (estilo
    /// Anthropic: `{name, description, input_schema}`), listo para pasarle al
    /// LLM vía `pluma-llm`. El modelo elige una tool y sus argumentos; eso se
    /// deserializa en una [`Invocacion`] y se resuelve con [`Catalogo::plan`].
    pub fn as_tools(&self) -> serde_json::Value {
        let tools: Vec<serde_json::Value> = self
            .capacidades()
            .into_iter()
            .map(|cap| {
                let mut props = serde_json::Map::new();
                let mut requeridos = Vec::new();
                for p in &cap.params {
                    props.insert(p.nombre.clone(), esquema_param(p));
                    if p.requerido {
                        requeridos.push(serde_json::Value::String(p.nombre.clone()));
                    }
                }
                serde_json::json!({
                    // `.` no es válido en nombres de tool de algunos backends; se
                    // mapea a `__` y se revierte al deserializar la invocación.
                    "name": cap.id.replace('.', "__"),
                    "description": cap.resumen,
                    "input_schema": {
                        "type": "object",
                        "properties": serde_json::Value::Object(props),
                        "required": serde_json::Value::Array(requeridos),
                    },
                })
            })
            .collect();
        serde_json::Value::Array(tools)
    }
}

/// El `input_schema` JSON de un parámetro según su tipo.
fn esquema_param(p: &Param) -> serde_json::Value {
    let mut s = serde_json::Map::new();
    match &p.tipo {
        TipoParam::Entero => {
            s.insert("type".into(), "integer".into());
        }
        TipoParam::Enum(opciones) => {
            s.insert("type".into(), "string".into());
            s.insert("enum".into(), serde_json::Value::Array(opciones.iter().map(|o| serde_json::Value::String(o.clone())).collect()));
        }
        // Texto / IdVentana / IdCard viajan como string (los ids son opacos).
        _ => {
            s.insert("type".into(), "string".into());
        }
    }
    s.insert("description".into(), serde_json::Value::String(p.descripcion.clone()));
    serde_json::Value::Object(s)
}

/// Revierte el nombre de tool (`mirada__workspace`) al `id` del catálogo
/// (`mirada.workspace`). Lo usa el llamador al recibir el tool-call del LLM.
pub fn id_desde_tool(nombre: &str) -> String {
    nombre.replacen("__", ".", 1)
}

#[cfg(feature = "mirada")]
pub mod mirada;
#[cfg(feature = "sandokan")]
pub mod sandokan;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruteo_por_prefijo_a_la_fuente_correcta() {
        let cat = Catalogo::estandar();
        // mirada
        let plan = cat.plan(&Invocacion::nueva("mirada.focus-next")).unwrap();
        assert_eq!(plan.programa, "mirada-ctl");
        // sandokan
        let plan = cat.plan(&Invocacion::nueva("sandokan.list")).unwrap();
        assert_eq!(plan.programa, "sandokan-cli");
    }

    #[test]
    fn invocacion_desconocida_es_error_legible() {
        let cat = Catalogo::estandar();
        let err = cat.plan(&Invocacion::nueva("inexistente.cosa")).unwrap_err();
        assert!(err.to_string().contains("desconocida"));
    }

    #[test]
    fn as_tools_mapea_punto_a_doble_guion_y_marca_requeridos() {
        let cat = Catalogo::estandar();
        let tools = cat.as_tools();
        let arr = tools.as_array().unwrap();
        // Hay al menos una tool de mirada con el `.` mapeado.
        assert!(arr.iter().any(|t| t["name"].as_str().unwrap().starts_with("mirada__")));
        assert_eq!(id_desde_tool("mirada__workspace"), "mirada.workspace");
    }

    #[test]
    fn catalogo_no_esta_vacio() {
        assert!(!Catalogo::estandar().capacidades().is_empty());
    }
}
