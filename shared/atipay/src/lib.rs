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
    /// Contextos de usuario (`pacha`): cambiar de modo de uso, listar, cerrar.
    Pacha,
}

impl Superficie {
    /// El prefijo de `id` que reclama esta superficie (`"mirada"`, `"sandokan"`…).
    pub fn prefijo(self) -> &'static str {
        match self {
            Superficie::Mirada => "mirada",
            Superficie::Sandokan => "sandokan",
            Superficie::Shuma => "shuma",
            Superficie::Sistema => "sistema",
            Superficie::Pacha => "pacha",
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

/// Una cosa que el sistema *puede hacer*. La unidad del catálogo. Lleva todo lo
/// necesario para armar el comando que la materializa: el `programa` (CLI) y los
/// `args_base` fijos, sobre los que se apilan los valores de `params`. Así una
/// superficie heterogénea (p.ej. `Sistema`, donde cada acción usa su propio CLI:
/// `systemctl`, `nmcli`, `loginctl`…) modela cada capacidad sin un único programa.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capacidad {
    /// Identificador estable y único, prefijado por superficie: `"mirada.workspace"`,
    /// `"sandokan.run"`. Es lo que la IA devuelve en una [`Invocacion`].
    pub id: String,
    pub superficie: Superficie,
    /// El programa CLI que la ejecuta (`"mirada-ctl"`, `"systemctl"`…).
    pub programa: String,
    /// Args fijos antes de los parámetros del usuario (`["radio","wifi","on"]`).
    pub args_base: Vec<String>,
    /// Frase corta de qué hace, en lenguaje natural (la lee el modelo).
    pub resumen: String,
    pub params: Vec<Param>,
    pub peligro: Peligro,
}

impl Capacidad {
    /// Capacidad cuyo verbo CLI coincide con el sufijo del id (el caso común de
    /// `mirada-ctl <verbo>` / `sandokan-cli <verbo>`).
    pub fn cli(superficie: Superficie, sufijo: &str, programa: &str, resumen: &str, peligro: Peligro, params: Vec<Param>) -> Self {
        Self::cli_args(superficie, sufijo, programa, &[sufijo], resumen, peligro, params)
    }

    /// Capacidad con args base explícitos (cuando el comando no es simplemente
    /// `programa <sufijo>`: `loginctl lock-session`, `nmcli radio wifi on`…).
    pub fn cli_args(superficie: Superficie, sufijo: &str, programa: &str, args_base: &[&str], resumen: &str, peligro: Peligro, params: Vec<Param>) -> Self {
        Self {
            id: format!("{}.{}", superficie.prefijo(), sufijo),
            superficie,
            programa: programa.into(),
            args_base: args_base.iter().map(|a| a.to_string()).collect(),
            resumen: resumen.into(),
            params,
            peligro,
        }
    }
}

/// Construye el [`Plan`] de una capacidad validando los argumentos de la
/// invocación contra los `params` (enteros, enums, requeridos). El comando es
/// `programa + args_base + valores de params` (en orden). No ejecuta nada.
fn resolver_plan(cap: &Capacidad, inv: &Invocacion) -> Result<Plan, AtipayError> {
    let mut args = cap.args_base.clone();
    for p in &cap.params {
        let valor = inv.arg(&p.nombre)?;
        match &p.tipo {
            TipoParam::Entero => {
                valor.parse::<i64>().map_err(|_| AtipayError::ArgInvalido {
                    id: cap.id.clone(),
                    arg: p.nombre.clone(),
                    motivo: format!("esperaba un entero, vino '{valor}'"),
                })?;
            }
            TipoParam::Enum(opciones) => {
                if !opciones.iter().any(|o| o == valor) {
                    return Err(AtipayError::ArgInvalido {
                        id: cap.id.clone(),
                        arg: p.nombre.clone(),
                        motivo: format!("'{valor}' no es una opción válida ({})", opciones.join("/")),
                    });
                }
            }
            _ => {}
        }
        args.push(valor.to_string());
    }
    Ok(Plan { id: cap.id.clone(), programa: cap.programa.clone(), args, peligro: cap.peligro })
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

impl Plan {
    /// Renderiza el plan como una **línea de shell** lista para ejecutar, con
    /// comillas simples alrededor de los args que las necesiten (espacios o
    /// metacaracteres). Es lo que `shuma` pone en el input para revisar y correr.
    pub fn linea_comando(&self) -> String {
        let mut out = self.programa.clone();
        for a in &self.args {
            out.push(' ');
            let necesita_comillas = a.is_empty() || a.chars().any(|c| c.is_whitespace() || "\"'$`\\*?~#&;|<>(){}[]!".contains(c));
            if necesita_comillas {
                // `'` no se puede escapar dentro de comillas simples: se cierra,
                // se mete un `'\''` y se reabre — el truco POSIX clásico.
                out.push('\'');
                out.push_str(&a.replace('\'', "'\\''"));
                out.push('\'');
            } else {
                out.push_str(a);
            }
        }
        out
    }
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

/// Lo que cada superficie de control implementa: qué sabe hacer. Es puramente
/// declarativo — la traducción de una invocación a un plan la hace el catálogo
/// con los datos de la propia [`Capacidad`] (programa + args_base + params), así
/// que una fuente no repite la lógica de armado.
pub trait FuenteCapacidades: Send + Sync {
    /// Qué superficie cubre esta fuente.
    fn superficie(&self) -> Superficie;
    /// El catálogo de capacidades de esta superficie.
    fn capacidades(&self) -> Vec<Capacidad>;
}

/// El catálogo agregado: junta varias fuentes y resuelve invocaciones. Es el
/// punto único que consulta la IA.
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
        #[cfg(feature = "sistema")]
        c.registrar(Box::new(crate::sistema::FuenteSistema));
        #[cfg(feature = "shuma")]
        c.registrar(Box::new(crate::shuma::FuenteShuma));
        #[cfg(feature = "pacha")]
        c.registrar(Box::new(crate::pacha::FuentePacha));
        c
    }

    /// Todas las capacidades de todas las fuentes, en una lista plana.
    pub fn capacidades(&self) -> Vec<Capacidad> {
        self.fuentes.iter().flat_map(|f| f.capacidades()).collect()
    }

    /// Resuelve una invocación a un plan: busca la capacidad por `id` y arma el
    /// comando validando los argumentos. Error legible si el `id` no existe.
    pub fn plan(&self, inv: &Invocacion) -> Result<Plan, AtipayError> {
        let caps = self.capacidades();
        let cap = caps
            .iter()
            .find(|c| c.id == inv.id)
            .ok_or_else(|| AtipayError::Desconocida(inv.id.clone()))?;
        resolver_plan(cap, inv)
    }

    /// Renderiza el catálogo como un **menú compacto** para incrustar en el
    /// prompt de un LLM: una línea por capacidad con el comando-plantilla
    /// (`programa verbo <params>`) y el resumen. Es lo que `shuma` le pasa al
    /// modelo en `:hacé` para que elija UNA y devuelva la línea de comando
    /// exacta — el camino sin tool-use, para backends que sólo chatean.
    pub fn prompt_menu(&self) -> String {
        let mut out = String::new();
        for c in self.capacidades() {
            let comando = if c.args_base.is_empty() {
                c.programa.clone()
            } else {
                format!("{} {}", c.programa, c.args_base.join(" "))
            };
            let params: String = c
                .params
                .iter()
                .map(|p| match &p.tipo {
                    TipoParam::Enum(ops) => format!(" <{}:{}>", p.nombre, ops.join("|")),
                    _ => format!(" <{}>", p.nombre),
                })
                .collect();
            out.push_str(&format!("{comando}{params}  — {}\n", c.resumen));
        }
        out
    }

    /// Menú por **id** para que el modelo elija una acción y devuelva su `id` +
    /// args en JSON (la vía de `:hacé`, que luego resuelve a un [`Plan`] con
    /// [`Catalogo::plan`]). Una línea por capacidad: `id — resumen (args: …)`.
    pub fn prompt_menu_ids(&self) -> String {
        let mut out = String::new();
        for c in self.capacidades() {
            let params: String = c
                .params
                .iter()
                .map(|p| match &p.tipo {
                    TipoParam::Enum(ops) => format!(" {}:{}", p.nombre, ops.join("|")),
                    _ => format!(" {}", p.nombre),
                })
                .collect();
            let ps = if params.is_empty() { String::new() } else { format!("  (args:{params})") };
            out.push_str(&format!("{}  — {}{}\n", c.id, c.resumen, ps));
        }
        out
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
#[cfg(feature = "sistema")]
pub mod sistema;
#[cfg(feature = "shuma")]
pub mod shuma;
#[cfg(feature = "pacha")]
pub mod pacha;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_resuelve_a_la_capacidad_y_su_programa() {
        let cat = Catalogo::estandar();
        let plan = cat.plan(&Invocacion::nueva("mirada.focus-next")).unwrap();
        assert_eq!(plan.programa, "mirada-ctl");
        let plan = cat.plan(&Invocacion::nueva("sandokan.list")).unwrap();
        assert_eq!(plan.programa, "sandokan-cli");
        // Superficie heterogénea: cada capacidad lleva su propio CLI.
        let plan = cat.plan(&Invocacion::nueva("sistema.apagar")).unwrap();
        assert_eq!(plan.programa, "systemctl");
        assert_eq!(plan.args, vec!["poweroff"]);
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

    #[test]
    fn linea_comando_entrecomilla_args_con_espacios() {
        let cat = Catalogo::estandar();
        let plan = cat.plan(&Invocacion::nueva("mirada.spawn").con("comando", "foot -e htop")).unwrap();
        assert_eq!(plan.linea_comando(), "mirada-ctl spawn 'foot -e htop'");
        // Sin metacaracteres → sin comillas.
        let plan = cat.plan(&Invocacion::nueva("mirada.workspace").con("n", "3")).unwrap();
        assert_eq!(plan.linea_comando(), "mirada-ctl workspace 3");
    }

    #[test]
    fn prompt_menu_trae_comandos_y_opciones_de_enum() {
        let menu = Catalogo::estandar().prompt_menu();
        // Comando concreto (programa + verbo).
        assert!(menu.contains("mirada-ctl workspace <n>"));
        assert!(menu.contains("sandokan-cli stop <id>"));
        // Las opciones de un enum se despliegan para guiar al modelo.
        assert!(menu.contains("master-stack|"));
    }
}
