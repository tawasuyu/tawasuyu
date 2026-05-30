//! Schema declarativo de la metainterfaz (nahual meta-schema).
//!
//! Cada **módulo** declara aquí qué menús, vistas, listas y
//! formularios expone, sin escribir código GPUI ni Rust. Cualquier
//! runtime de UI dirigida por datos (Nakui hoy, otros mañana) lo
//! carga y monta la UI correspondiente.
//!
//! ## Filosofía
//!
//! - **UI como datos**: agregar un módulo = escribir un JSON o un
//!   `.ncl`. Ningún recompile, ningún acoplamiento con el binario
//!   del runtime.
//! - **Backend-agnostic**: este crate sólo describe la *forma* de
//!   la UI. La conexión a un store/log/executor concretos vive en
//!   el runtime que lo consume (ej: el meta-runtime de Nakui que
//!   wirea esto a `nakui_core` + KCL post-checks).
//! - **Schema primero, semántica después**: validación semántica
//!   (referencias rotas a entities, campos faltantes, etc.) vive
//!   en el runtime que lo carga, no acá.
//!
//! ## Anatomía de un módulo
//!
//! ```json
//! {
//!   "id": "customers",
//!   "label": "Clientes",
//!   "entities": [
//!     { "name": "customer", "fields": [ ... ] }
//!   ],
//!   "menu": [
//!     { "label": "Listar", "view": "list" },
//!     { "label": "Nuevo", "view": "form" }
//!   ],
//!   "views": {
//!     "list": { "kind": "list", "entity": "customer", "columns": [...] },
//!     "form": { "kind": "form", "entity": "customer", "fields": [...] }
//!   }
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Manifiesto de un módulo declarativo de UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Module {
    /// Identificador estable. Único dentro del directorio cargado.
    pub id: String,

    /// Nombre legible para mostrar en el sidebar.
    pub label: String,

    /// Descripción corta opcional (tooltip / subtítulo).
    #[serde(default)]
    pub description: Option<String>,

    /// Entities que el módulo introduce o consume. El runtime las
    /// usa para validar columns/fields y para inicializar el store
    /// cuando son nuevas.
    #[serde(default)]
    pub entities: Vec<EntitySpec>,

    /// Path opaco al backend que va a manejar este módulo. Lo
    /// interpreta el runtime concreto (no este schema).
    ///
    /// Convención actual de Nakui: directorio con `nsmc.json` +
    /// schemas KCL + scripts Rhai. Cuando está set, el runtime
    /// Nakui carga un `Executor` para ese path y permite que las
    /// acciones `Morphism { name }` despachen al pipeline real
    /// (compute → log → apply). Otro backend puede ignorar este
    /// campo o darle un significado distinto.
    ///
    /// Path resuelto relativo al directorio del `module.json`
    /// o absoluto.
    ///
    /// Si es `None`, los backends que requieren manifest deberían
    /// degradar (toast informativo, deshabilitar morphisms, etc.);
    /// los `SeedEntity` siguen funcionando — son altas
    /// administrativas que no necesitan validación de manifest.
    ///
    /// Nombre conservado por compat con módulos ya escritos.
    /// Renombrar a `backend_module_dir` o similar si emerge un
    /// segundo backend que también lo use.
    #[serde(default, alias = "backend_module_dir")]
    pub nakui_module_dir: Option<String>,

    /// Items del menú. Cada uno apunta a una key de `views`. Orden
    /// importa (es el orden en que se presentan en el sidebar).
    pub menu: Vec<MenuItem>,

    /// Vistas indexadas por key. Las keys son referenciadas por
    /// `MenuItem.view` y por `Action::OpenView.view`.
    pub views: BTreeMap<String, View>,
}

/// Item del menú lateral.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuItem {
    pub label: String,
    /// Key de la vista a abrir. Debe existir en `Module.views`.
    pub view: String,
    /// Icono opcional (texto unicode o emoji; el runtime decide
    /// renderización).
    #[serde(default)]
    pub icon: Option<String>,
}

/// Una vista renderizada en el área principal cuando su menú es seleccionado.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum View {
    /// Tabla de instancias de una entity, columnas + acciones por fila.
    List(ListView),
    /// Formulario de creación / edición.
    Form(FormView),
    /// Ficha de un record: sus campos + listas de records relacionados.
    Detail(DetailView),
    /// Tablero de KPIs: una grilla de tarjetas de agregados.
    Dashboard(DashboardView),
    /// Reporte imprimible: los mismos agregados que un tablero, pero
    /// dispuestos como documento de una columna (título + fecha de
    /// generación) y exportable a Markdown.
    Report(ReportView),
    /// Grafo de dependencias: el DAG de morfismos del módulo nakui. Cada
    /// morfismo es un nodo; los tokens que lee/escribe son sus pins; las
    /// aristas de flujo de datos (escritura→lectura del mismo token) son
    /// los cables. Visualiza la cascada reactiva que conecta el dato.
    Graph(GraphView),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListView {
    pub title: String,
    /// Entity (del nakui store) cuyas instancias se listan.
    pub entity: String,
    /// Columnas a mostrar. Orden importa.
    pub columns: Vec<Column>,
    /// Acciones disponibles a nivel de la lista (ej. "Nuevo" → form).
    /// Renderizadas como botones en el header.
    #[serde(default)]
    pub actions: Vec<Action>,
    /// Cuando está set, se muestra una caja de búsqueda que filtra
    /// las filas por substring contra los valores de estas columnas.
    #[serde(default)]
    pub search_in: Vec<String>,
    /// Si está set, cada fila gana un botón 👁 que abre esta vista
    /// (debe ser una `View::Detail`) para el record de la fila.
    #[serde(default)]
    pub row_detail: Option<String>,
}

/// Ficha de un record: sus campos + listas de records relacionados.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailView {
    pub title: String,
    /// Entity del record que se muestra.
    pub entity: String,
    /// Campos a mostrar, en orden. Reusa [`Column`] (label + field +
    /// `ref_entity` + `format`; el `weight` se ignora en la ficha).
    #[serde(default)]
    pub fields: Vec<Column>,
    /// Listas de records relacionados (back-references).
    #[serde(default)]
    pub related: Vec<RelatedList>,
    /// KPIs scopeados al record (el "360" de la ficha): agregados sobre
    /// los records relacionados (`entity` cuyo `via_field` apunta al
    /// record actual), p.ej. en la ficha de un cliente "Total facturado"
    /// / "Órdenes" / "Ticket promedio". Renderizados como stat cards
    /// arriba de las listas relacionadas.
    #[serde(default)]
    pub metrics: Vec<DetailMetric>,
}

/// Un KPI scopeado a un record dentro de una [`DetailView`]: computa
/// `metric` sobre los records de `entity` cuyo `via_field` referencia
/// al record que se está viendo (mismo criterio de scope que una
/// [`RelatedList`]), con un `filter` opcional adicional.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailMetric {
    pub label: String,
    /// Entity sobre cuyos records relacionados se computa el agregado.
    pub entity: String,
    /// Campo de esa entity que referencia (UUID) al record actual.
    pub via_field: String,
    pub metric: Metric,
    /// Filtro adicional opcional (AND con el scope), p.ej. `pagado=true`
    /// para un KPI "cobrado".
    #[serde(default)]
    pub filter: Option<CardFilter>,
    /// Formato del número resultante (`Currency` para sumas de dinero).
    #[serde(default)]
    pub format: ValueFormat,
}

/// Una lista de records relacionados dentro de una [`DetailView`]: los
/// records de otra entity cuyo campo `via_field` apunta al record que
/// se está viendo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedList {
    pub title: String,
    /// Entity de los records relacionados.
    pub entity: String,
    /// Campo de esa entity cuyo valor (UUID) referencia al record
    /// actual. El runtime filtra `record[via_field] == id_actual`.
    pub via_field: String,
    pub columns: Vec<Column>,
}

/// Tablero de KPIs: una grilla de tarjetas de agregados.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardView {
    pub title: String,
    pub cards: Vec<DashboardCard>,
}

/// Reporte imprimible: mismos agregados que un tablero, presentados
/// como documento exportable. Opcionalmente un subtítulo/encabezado.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportView {
    pub title: String,
    /// Línea de subtítulo opcional bajo el título (período, área…).
    #[serde(default)]
    pub subtitle: Option<String>,
    pub cards: Vec<DashboardCard>,
    /// Controles interactivos: cada uno es un filtro etiquetado que el
    /// usuario prende/apaga desde la UI. Cuando está activo, su filtro
    /// se aplica (AND) sobre los records de cada card antes de agregar
    /// —recortando el reporte sin tocar el `module.json`—.
    #[serde(default)]
    pub toggles: Vec<ReportToggle>,
}

/// Un control de filtro interactivo de un [`ReportView`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportToggle {
    /// Texto del botón (p.ej. "Solo pagadas", "Q1 2026").
    pub label: String,
    /// Si está set, el toggle sólo afecta a las cards de esta entity
    /// (evita vaciar cards de otra entity que no tiene el campo). Si es
    /// `None`, se aplica a todas las cards.
    #[serde(default)]
    pub entity: Option<String>,
    /// Filtro que se aplica cuando el control está activo.
    pub filter: CardFilter,
}

/// Vista grafo: el DAG de morfismos del módulo nakui. No tiene
/// parámetros más allá del título y un subtítulo opcional — el grafo se
/// deriva en runtime del manifest del `Executor` del módulo (los
/// morfismos y los tokens que lee/escribe cada uno), no se declara acá.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphView {
    pub title: String,
    /// Línea de contexto opcional bajo el título.
    #[serde(default)]
    pub subtitle: Option<String>,
}

/// Una tarjeta de KPI del tablero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardCard {
    /// Etiqueta de la tarjeta.
    pub label: String,
    /// Entity sobre cuyos records se computa el agregado.
    pub entity: String,
    /// Qué se computa.
    pub metric: Metric,
    /// Filtro opcional: sólo entran los records que lo cumplen.
    #[serde(default)]
    pub filter: Option<CardFilter>,
    /// Formato del número resultante (`Currency` para sumas de dinero).
    /// Ignorado por `GroupBy`.
    #[serde(default)]
    pub format: ValueFormat,
    /// Cuando el agregado produce un desglose por grupo (`GroupBy` /
    /// `SumBy` / `AvgBy`) y la clave de grupo es un UUID que referencia
    /// a otra entity, esta entity se usa para resolver cada clave a su
    /// label legible (p.ej. "facturación por cliente" muestra nombres
    /// en vez de UUIDs). El runtime de presentación hace la resolución;
    /// el motor de métricas permanece agnóstico.
    #[serde(default)]
    pub group_ref: Option<String>,
    /// Cómo se dibuja un desglose (`GroupBy` / `SumBy` / `AvgBy`):
    /// barras ASCII (default), torta o dona. Ignorado por métricas
    /// escalares (`Count` / `Sum` / `Avg` / `Min` / `Max`), que siempre
    /// se muestran como número grande.
    #[serde(default)]
    pub chart: ChartKind,
    /// Tope de filas de un desglose: se conservan las `limit` de mayor
    /// valor (el motor ya las ordena de mayor a menor) y el resto se
    /// colapsa en una fila "Otros" (suma para conteos/`SumBy`, promedio
    /// de los grupos restantes para `AvgBy`). Mantiene legibles los
    /// gráficos sobre dimensiones de muchos grupos. `None` = sin tope.
    /// Ignorado cuando hay `bucket` (las series temporales no se recortan).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Cuando el campo de grupo de un desglose es una fecha ISO-8601,
    /// trunca la clave a año/mes/día antes de agregar — convierte el
    /// desglose en una **serie temporal** (p.ej. "facturación por mes").
    /// El resultado se ordena cronológicamente (no por valor) y no se
    /// recorta. `None` = agrupar por el valor crudo del campo.
    #[serde(default)]
    pub bucket: Option<DateBucket>,
}

/// Granularidad de truncado de una fecha ISO-8601 para series temporales.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateBucket {
    /// Año: `2026-01-15` → `2026`.
    Year,
    /// Año-mes: `2026-01-15` → `2026-01`.
    Month,
    /// Día (fecha completa): `2026-01-15` → `2026-01-15`.
    Day,
}

/// Forma visual de un desglose de tablero/reporte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    /// Barras horizontales en caracteres de bloque. Default.
    #[default]
    Bars,
    /// Gráfico de torta: cada grupo es un sector proporcional a su valor.
    Pie,
    /// Como `Pie` pero con el centro hueco (anillo).
    Donut,
    /// Columnas verticales, una por grupo (en el orden del desglose).
    /// Apto para series ordenadas (p.ej. ingresos por mes).
    Columns,
    /// Línea que une los valores de cada grupo, con un punto por grupo.
    /// Pensado para tendencias sobre un eje ordenado.
    Line,
}

/// El agregado que computa una [`DashboardCard`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Metric {
    /// Cantidad de records.
    Count,
    /// Suma de un campo numérico.
    Sum { field: String },
    /// Promedio de un campo numérico (ignora records sin el campo).
    Avg { field: String },
    /// Mínimo de un campo numérico.
    Min { field: String },
    /// Máximo de un campo numérico.
    Max { field: String },
    /// Conteo de records por cada valor distinto de un campo.
    GroupBy { field: String },
    /// Suma de `value` por cada valor distinto de `group` — el reporte
    /// ERP clásico ("facturación por cliente", "ingresos por mes").
    SumBy { group: String, value: String },
    /// Promedio de `value` por cada valor distinto de `group`
    /// ("ticket promedio por plan").
    AvgBy { group: String, value: String },
    /// Suma de `value` por cada combinación de `group` × `series` — un
    /// desglose de **dos dimensiones** ("facturación por mes, por
    /// plan"). El eje principal es `group` (x); cada valor distinto de
    /// `series` es una serie sobre ese eje. Se renderiza como
    /// multi-línea o columnas agrupadas.
    SumBySeries {
        group: String,
        series: String,
        value: String,
    },
}

/// Operador de comparación de un [`CardFilter`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    /// Igualdad textual contra `value`. Default (compat con `equals`).
    #[default]
    Eq,
    /// Desigualdad textual.
    Ne,
    /// Mayor que `value` (numérico, o fecha ISO comparada como texto).
    Gt,
    /// Mayor o igual.
    Gte,
    /// Menor que.
    Lt,
    /// Menor o igual.
    Lte,
    /// Dentro de `[min, max]` inclusivo (cualquiera de las dos cotas
    /// puede omitirse para un rango abierto).
    Between,
    /// El campo existe y no está vacío.
    NonEmpty,
}

/// Filtro de una [`DashboardCard`]: decide qué records entran al
/// agregado. El default (`op: eq` + `value`) replica el viejo
/// `{ field, equals }`, que sigue parseando por el alias `equals`.
/// Para `gt`/`lt`/`between`, las comparaciones son numéricas si ambos
/// lados parsean como número, y lexicográficas en caso contrario —
/// suficiente para rangos de fecha en ISO-8601 (`YYYY-MM-DD`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardFilter {
    pub field: String,
    /// Operador. Default `eq`.
    #[serde(default)]
    pub op: FilterOp,
    /// Comparando para `eq`/`ne`/`gt`/`gte`/`lt`/`lte`. Acepta el alias
    /// `equals` por compatibilidad con tableros ya escritos.
    #[serde(default, alias = "equals")]
    pub value: Option<String>,
    /// Cota inferior para `between` (inclusiva). `None` = sin piso.
    #[serde(default)]
    pub min: Option<String>,
    /// Cota superior para `between` (inclusiva). `None` = sin techo.
    #[serde(default)]
    pub max: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    /// Path del campo dentro del record. Para tipos planos: nombre.
    /// Para nested: puntos (`address.city`). El runtime navega.
    pub field: String,
    /// Texto del header.
    pub label: String,
    /// Ancho relativo (peso flex). Default 1.
    #[serde(default = "default_weight")]
    pub weight: f32,
    /// Si está set, la celda resuelve su valor (un UUID) al label
    /// legible del record de esta entity, en vez de mostrar el UUID
    /// crudo. Para columnas que son referencias a otra entity.
    #[serde(default)]
    pub ref_entity: Option<String>,
    /// Formato de presentación del valor de la celda.
    #[serde(default)]
    pub format: ValueFormat,
}

fn default_weight() -> f32 {
    1.0
}

/// Formato de presentación de un valor en una celda de lista.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValueFormat {
    /// Sin format — el valor se muestra crudo. Default.
    #[default]
    Plain,
    /// Entero/decimal con separador de miles (`12000` → `12,000`).
    Number,
    /// Moneda: separador de miles + símbolo prefijo (`12000` → `$12,000`).
    Currency { symbol: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormView {
    pub title: String,
    /// Entity destino del seed/morphism al submit.
    pub entity: String,
    pub fields: Vec<FieldSpec>,
    /// Acción al submit. Típicamente `Action::SeedEntity` para alta
    /// directa o `Action::Morphism` cuando hay validación/cálculo.
    pub on_submit: Action,
}

/// Especificación de un campo de formulario o columna implícita.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
    /// Nombre del campo en el record (clave del JSON).
    pub name: String,
    /// Etiqueta legible.
    pub label: String,
    /// Tipo del valor — define el widget de input + parseo.
    pub kind: FieldKind,
    /// Valor por defecto al abrir el form (string raw; el parseo
    /// según `kind` lo hace el runtime).
    #[serde(default)]
    pub default: Option<String>,
    /// Si `true`, el form rechaza submit con campo vacío.
    #[serde(default)]
    pub required: bool,
    /// Texto de ayuda mostrado bajo el input.
    #[serde(default)]
    pub help: Option<String>,
    /// Si `kind == EntityRef`, indica qué entity referencia. Sin
    /// esto, el runtime no sabe qué records ofrecer en el selector
    /// y la validación `Module::validate` rechaza el manifest.
    /// Para los demás kinds, este campo se ignora.
    #[serde(default)]
    pub ref_entity: Option<String>,
    /// Opciones de un campo `kind == Select`. Ignorado para los demás
    /// kinds. `Module::validate` exige que un Select las tenga.
    #[serde(default)]
    pub options: Vec<SelectOption>,
    /// Sección del formulario a la que pertenece el campo. Campos
    /// consecutivos con la misma sección se agrupan bajo un encabezado.
    #[serde(default)]
    pub section: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// Texto libre.
    Text,
    /// Texto multilínea.
    Multiline,
    /// Número (i64 o f64; runtime intenta parsear como i64 primero).
    Number,
    /// Booleano (renderizado como checkbox).
    Boolean,
    /// Fecha (format ISO YYYY-MM-DD; almacenada como string).
    Date,
    /// Referencia a otro record. El runtime renderiza un selector
    /// clickable de records existentes de la entity declarada en
    /// `FieldSpec.ref_entity`; el value almacenado es el UUID del
    /// seleccionado, parseable como cualquier text/UUID al submit.
    EntityRef,
    /// Valor elegido de un conjunto cerrado declarado en
    /// `FieldSpec.options`. El runtime lo renderiza como selección
    /// (no texto libre). `Module::validate` exige `options` no vacío.
    Select,
    /// Identificador autogenerado (UUID v4). El runtime lo rellena al
    /// abrir el formulario; el usuario no lo teclea ni lo edita. Para
    /// los ids de idempotencia que piden los morfismos.
    AutoId,
}

/// Una opción de un campo [`FieldKind::Select`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectOption {
    /// Valor que se guarda (lo que recibe el backend o el morfismo).
    pub value: String,
    /// Etiqueta legible. Si se omite, se muestra el `value` crudo.
    #[serde(default)]
    pub label: Option<String>,
}

impl SelectOption {
    /// Texto a mostrar: `label` si está, sino el `value`.
    pub fn display(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.value)
    }
}

/// Acciones disparables por menús, botones o submit de formularios.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Cambia la vista activa a otra del mismo módulo.
    OpenView {
        view: String,
        /// Etiqueta del botón / item; default = nombre humano de la vista.
        #[serde(default)]
        label: Option<String>,
    },
    /// Crea un seed directo en la entity con los valores del form.
    /// Equivalente a `nakui_core::event_log::seed_and_log`.
    /// Sin pipeline de morphism — alta administrativa.
    SeedEntity {
        entity: String,
        /// Tras el submit exitoso, opcionalmente abrir esta vista
        /// (por convención: `"list"` para volver al listado).
        #[serde(default)]
        next_view: Option<String>,
    },
    /// Ejecuta un morphism declarado en el manifest del módulo
    /// nakui-core (cuyo path vive en `Module.nakui_module_dir`).
    /// Inputs (records existentes) y params (valores escalares) se
    /// mapean desde los campos del form.
    Morphism {
        /// Nombre del morphism declarado en `nsmc.json` del manifest
        /// nakui apuntado por el módulo.
        name: String,
        /// Mapeo `role → field_name`: por cada input declarado en
        /// el `MorphismSpec.inputs`, indica qué field del form
        /// contiene el UUID del record. El runtime parsea el value
        /// como `Uuid` y lo pasa como input al `execute_and_log`.
        ///
        /// Ej: `{ "stock": "stock_id", "caja": "caja_id" }` para un
        /// morphism `vender` que toma roles `stock` y `caja`.
        #[serde(default)]
        inputs: BTreeMap<String, String>,
        /// Lista de fields del form cuyos values van al `params`
        /// JSON object pasado al morphism. Si está vacío, todos los
        /// fields que no estén en `inputs` van a params.
        #[serde(default)]
        params: Vec<String>,
        #[serde(default)]
        next_view: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySpec {
    /// Nombre de la entity (clave en el store). Único dentro del módulo.
    pub name: String,
    /// Label legible (singular).
    pub label: String,
    /// Campos esperados en records de esta entity. Usados por el
    /// runtime para inferir columnas / validar formularios.
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
}

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("io leyendo {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parseo de {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("módulo {id}: la vista '{view}' referenciada por menu_item '{item}' no existe")]
    DanglingMenuView {
        id: String,
        item: String,
        view: String,
    },
    #[error("módulos con id duplicado: '{id}' aparece en {first} y {second}")]
    DuplicateModuleId {
        id: String,
        first: PathBuf,
        second: PathBuf,
    },
    #[error(
        "módulo {id} vista '{view}': field '{field}' tiene kind=entity_ref \
         pero no declaró ref_entity"
    )]
    EntityRefMissingTarget {
        id: String,
        view: String,
        field: String,
    },
    #[error(
        "módulo {id} vista '{view}': field '{field}' tiene kind=select \
         pero no declaró options"
    )]
    SelectMissingOptions {
        id: String,
        view: String,
        field: String,
    },
    #[error(
        "módulo {id} vista '{view}': row_detail='{target}' no apunta a \
         una vista kind=detail"
    )]
    RowDetailInvalid {
        id: String,
        view: String,
        target: String,
    },
}

impl Module {
    /// Carga un module.json desde disco. Validación estructural
    /// posterior (vistas referenciadas existen, etc.) la ejecuta el
    /// runtime — acá sólo parseamos.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, SchemaError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| SchemaError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        serde_json::from_slice(&bytes).map_err(|source| SchemaError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Validación post-parse:
    /// - Cada `MenuItem.view` debe existir en `views`.
    /// - Cada `FieldSpec` con `kind=EntityRef` debe declarar
    ///   `ref_entity`.
    pub fn validate(&self) -> Result<(), SchemaError> {
        for item in &self.menu {
            if !self.views.contains_key(&item.view) {
                return Err(SchemaError::DanglingMenuView {
                    id: self.id.clone(),
                    item: item.label.clone(),
                    view: item.view.clone(),
                });
            }
        }
        for (view_key, view) in &self.views {
            match view {
                View::Form(form) => {
                    for f in &form.fields {
                        if f.kind == FieldKind::EntityRef && f.ref_entity.is_none() {
                            return Err(SchemaError::EntityRefMissingTarget {
                                id: self.id.clone(),
                                view: view_key.clone(),
                                field: f.name.clone(),
                            });
                        }
                        if f.kind == FieldKind::Select && f.options.is_empty() {
                            return Err(SchemaError::SelectMissingOptions {
                                id: self.id.clone(),
                                view: view_key.clone(),
                                field: f.name.clone(),
                            });
                        }
                    }
                }
                View::List(list) => {
                    if let Some(target) = &list.row_detail {
                        if !matches!(self.views.get(target), Some(View::Detail(_))) {
                            return Err(SchemaError::RowDetailInvalid {
                                id: self.id.clone(),
                                view: view_key.clone(),
                                target: target.clone(),
                            });
                        }
                    }
                }
                View::Detail(_) | View::Dashboard(_) | View::Report(_) | View::Graph(_) => {}
            }
        }
        Ok(())
    }
}

/// Carga todos los `module.json` encontrados bajo `dir` (recursivo
/// 1 nivel — espera `dir/<modulo>/module.json`). Devuelve la lista
/// ordenada por id, validada.
///
/// Falla si: I/O, parseo, módulo inválido, o ids duplicados.
pub fn load_modules_from_dir(dir: impl AsRef<Path>) -> Result<Vec<Module>, SchemaError> {
    let dir = dir.as_ref();
    let mut modules: Vec<(PathBuf, Module)> = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|source| SchemaError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let manifest = p.join("module.json");
            if manifest.exists() {
                let m = Module::from_path(&manifest)?;
                m.validate()?;
                modules.push((manifest, m));
            }
        }
    }
    // Detectar duplicados de id.
    modules.sort_by(|a, b| a.1.id.cmp(&b.1.id));
    let mut prev: Option<&(PathBuf, Module)> = None;
    for cur in &modules {
        if let Some(p) = prev {
            if p.1.id == cur.1.id {
                return Err(SchemaError::DuplicateModuleId {
                    id: cur.1.id.clone(),
                    first: p.0.clone(),
                    second: cur.0.clone(),
                });
            }
        }
        prev = Some(cur);
    }
    Ok(modules.into_iter().map(|(_, m)| m).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_module() -> Module {
        Module {
            id: "customers".into(),
            label: "Clientes".into(),
            description: Some("Gestión de clientes".into()),
            nakui_module_dir: None,
            entities: vec![EntitySpec {
                name: "customer".into(),
                label: "Cliente".into(),
                fields: vec![
                    FieldSpec {
                        name: "name".into(),
                        label: "Nombre".into(),
                        kind: FieldKind::Text,
                        default: None,
                        required: true,
                        help: None,
                        ref_entity: None,
                        options: Vec::new(),
                        section: None,
                    },
                    FieldSpec {
                        name: "email".into(),
                        label: "Email".into(),
                        kind: FieldKind::Text,
                        default: None,
                        required: false,
                        help: Some("Opcional".into()),
                        ref_entity: None,
                        options: Vec::new(),
                        section: None,
                    },
                ],
            }],
            menu: vec![
                MenuItem {
                    label: "Listar".into(),
                    view: "list".into(),
                    icon: None,
                },
                MenuItem {
                    label: "Nuevo".into(),
                    view: "form".into(),
                    icon: None,
                },
            ],
            views: BTreeMap::from([
                (
                    "list".into(),
                    View::List(ListView {
                        title: "Clientes".into(),
                        entity: "customer".into(),
                        columns: vec![
                            Column {
                                field: "name".into(),
                                label: "Nombre".into(),
                                weight: 2.0,
                                ref_entity: None,
                                format: ValueFormat::Plain,
                            },
                            Column {
                                field: "email".into(),
                                label: "Email".into(),
                                weight: 3.0,
                                ref_entity: None,
                                format: ValueFormat::Plain,
                            },
                        ],
                        actions: vec![Action::OpenView {
                            view: "form".into(),
                            label: Some("Nuevo".into()),
                        }],
                        search_in: vec!["name".into(), "email".into()],
                        row_detail: None,
                    }),
                ),
                (
                    "form".into(),
                    View::Form(FormView {
                        title: "Nuevo cliente".into(),
                        entity: "customer".into(),
                        fields: vec![FieldSpec {
                            name: "name".into(),
                            label: "Nombre".into(),
                            kind: FieldKind::Text,
                            default: None,
                            required: true,
                            help: None,
                            ref_entity: None,
                            options: Vec::new(),
                            section: None,
                        }],
                        on_submit: Action::SeedEntity {
                            entity: "customer".into(),
                            next_view: Some("list".into()),
                        },
                    }),
                ),
            ]),
        }
    }

    #[test]
    fn module_serialize_deserialize_roundtrip() {
        let m = sample_module();
        let s = serde_json::to_string_pretty(&m).unwrap();
        let m2: Module = serde_json::from_str(&s).unwrap();
        assert_eq!(m.id, m2.id);
        assert_eq!(m.menu.len(), m2.menu.len());
        assert_eq!(m.views.len(), m2.views.len());
    }

    #[test]
    fn validate_passes_for_well_formed_module() {
        sample_module().validate().unwrap();
    }

    #[test]
    fn validate_catches_dangling_menu_view() {
        let mut m = sample_module();
        m.menu.push(MenuItem {
            label: "Roto".into(),
            view: "no_existe".into(),
            icon: None,
        });
        let err = m.validate().unwrap_err();
        assert!(matches!(err, SchemaError::DanglingMenuView { .. }));
    }

    #[test]
    fn from_path_loads_real_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("module.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&sample_module()).unwrap()).unwrap();
        let m = Module::from_path(&path).unwrap();
        assert_eq!(m.id, "customers");
    }

    #[test]
    fn load_modules_from_dir_finds_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let cust_dir = tmp.path().join("customers");
        std::fs::create_dir(&cust_dir).unwrap();
        let mut m1 = sample_module();
        m1.id = "customers".into();
        std::fs::write(
            cust_dir.join("module.json"),
            serde_json::to_vec_pretty(&m1).unwrap(),
        )
        .unwrap();
        let prod_dir = tmp.path().join("products");
        std::fs::create_dir(&prod_dir).unwrap();
        let mut m2 = sample_module();
        m2.id = "products".into();
        std::fs::write(
            prod_dir.join("module.json"),
            serde_json::to_vec_pretty(&m2).unwrap(),
        )
        .unwrap();

        let mods = load_modules_from_dir(tmp.path()).unwrap();
        assert_eq!(mods.len(), 2);
        assert_eq!(mods[0].id, "customers");
        assert_eq!(mods[1].id, "products");
    }

    #[test]
    fn validate_catches_entity_ref_without_target() {
        let mut m = sample_module();
        // Inyectamos un form con un campo EntityRef sin ref_entity.
        m.views.insert(
            "broken_form".into(),
            View::Form(FormView {
                title: "Roto".into(),
                entity: "customer".into(),
                fields: vec![FieldSpec {
                    name: "ref_to_nowhere".into(),
                    label: "Referencia".into(),
                    kind: FieldKind::EntityRef,
                    default: None,
                    required: true,
                    help: None,
                    ref_entity: None,
                    options: Vec::new(),
                    section: None,
                }],
                on_submit: Action::SeedEntity {
                    entity: "customer".into(),
                    next_view: None,
                },
            }),
        );
        let err = m.validate().unwrap_err();
        assert!(
            matches!(err, SchemaError::EntityRefMissingTarget { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn entity_ref_with_target_validates_clean() {
        let mut m = sample_module();
        m.views.insert(
            "ok_form".into(),
            View::Form(FormView {
                title: "OK".into(),
                entity: "customer".into(),
                fields: vec![FieldSpec {
                    name: "supplier".into(),
                    label: "Proveedor".into(),
                    kind: FieldKind::EntityRef,
                    default: None,
                    required: true,
                    help: None,
                    ref_entity: Some("supplier".into()),
                    options: Vec::new(),
                    section: None,
                }],
                on_submit: Action::SeedEntity {
                    entity: "customer".into(),
                    next_view: None,
                },
            }),
        );
        m.menu.push(MenuItem {
            label: "OK".into(),
            view: "ok_form".into(),
            icon: None,
        });
        m.validate().unwrap();
    }

    #[test]
    fn select_without_options_is_rejected() {
        let mut m = sample_module();
        m.views.insert(
            "sel_form".into(),
            View::Form(FormView {
                title: "Select roto".into(),
                entity: "customer".into(),
                fields: vec![FieldSpec {
                    name: "estado".into(),
                    label: "Estado".into(),
                    kind: FieldKind::Select,
                    default: None,
                    required: true,
                    help: None,
                    ref_entity: None,
                    options: Vec::new(),
                    section: None,
                }],
                on_submit: Action::SeedEntity {
                    entity: "customer".into(),
                    next_view: None,
                },
            }),
        );
        let err = m.validate().unwrap_err();
        assert!(
            matches!(err, SchemaError::SelectMissingOptions { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn select_with_options_validates_clean() {
        let mut m = sample_module();
        m.views.insert(
            "sel_form".into(),
            View::Form(FormView {
                title: "Select OK".into(),
                entity: "customer".into(),
                fields: vec![FieldSpec {
                    name: "estado".into(),
                    label: "Estado".into(),
                    kind: FieldKind::Select,
                    default: None,
                    required: true,
                    help: None,
                    ref_entity: None,
                    options: vec![
                        SelectOption {
                            value: "activo".into(),
                            label: Some("Activo".into()),
                        },
                        SelectOption {
                            value: "baja".into(),
                            label: None,
                        },
                    ],
                    section: None,
                }],
                on_submit: Action::SeedEntity {
                    entity: "customer".into(),
                    next_view: None,
                },
            }),
        );
        m.menu.push(MenuItem {
            label: "Sel".into(),
            view: "sel_form".into(),
            icon: None,
        });
        m.validate().unwrap();
    }

    #[test]
    fn select_option_display_falls_back_to_value() {
        let with_label = SelectOption {
            value: "x".into(),
            label: Some("Equis".into()),
        };
        let bare = SelectOption {
            value: "y".into(),
            label: None,
        };
        assert_eq!(with_label.display(), "Equis");
        assert_eq!(bare.display(), "y");
    }

    #[test]
    fn chart_kind_defaults_to_bars_and_parses() {
        // Card sin `chart` → default Bars (back-compat con tableros viejos).
        let card: DashboardCard = serde_json::from_value(serde_json::json!({
            "label": "Clientes por plan",
            "entity": "customers",
            "metric": { "kind": "group_by", "field": "tier" }
        }))
        .unwrap();
        assert_eq!(card.chart, ChartKind::Bars);
        assert_eq!(card.limit, None);

        // `limit` parsea como entero opcional.
        let capped: DashboardCard = serde_json::from_value(serde_json::json!({
            "label": "Top clientes",
            "entity": "orders",
            "metric": { "kind": "sum_by", "group": "customer", "value": "monto" },
            "limit": 8
        }))
        .unwrap();
        assert_eq!(capped.limit, Some(8));
        assert_eq!(card.bucket, None);

        // `bucket` parsea en snake_case a DateBucket.
        let serie: DashboardCard = serde_json::from_value(serde_json::json!({
            "label": "Por mes",
            "entity": "orders",
            "metric": { "kind": "sum_by", "group": "fecha", "value": "monto" },
            "chart": "line",
            "bucket": "month"
        }))
        .unwrap();
        assert_eq!(serie.bucket, Some(DateBucket::Month));

        // Con `chart` explícito en snake_case.
        let pie: DashboardCard = serde_json::from_value(serde_json::json!({
            "label": "Facturación",
            "entity": "orders",
            "metric": { "kind": "sum_by", "group": "customer", "value": "monto" },
            "chart": "pie"
        }))
        .unwrap();
        assert_eq!(pie.chart, ChartKind::Pie);
        assert_eq!(ChartKind::default(), ChartKind::Bars);

        // Columns y Line también parsean en snake_case.
        for (raw, want) in [("columns", ChartKind::Columns), ("line", ChartKind::Line)] {
            let c: DashboardCard = serde_json::from_value(serde_json::json!({
                "label": "x",
                "entity": "orders",
                "metric": { "kind": "sum_by", "group": "mes", "value": "monto" },
                "chart": raw
            }))
            .unwrap();
            assert_eq!(c.chart, want);
        }
    }

    #[test]
    fn load_modules_detects_duplicate_id() {
        let tmp = tempfile::tempdir().unwrap();
        let a_dir = tmp.path().join("a");
        let b_dir = tmp.path().join("b");
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        let m = sample_module(); // id = "customers"
        std::fs::write(a_dir.join("module.json"), serde_json::to_vec(&m).unwrap()).unwrap();
        std::fs::write(b_dir.join("module.json"), serde_json::to_vec(&m).unwrap()).unwrap();
        let err = load_modules_from_dir(tmp.path()).unwrap_err();
        assert!(matches!(err, SchemaError::DuplicateModuleId { .. }));
    }
}
