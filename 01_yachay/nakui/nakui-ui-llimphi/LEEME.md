# nakui-ui-llimphi

> Shell de la metainterfaz de [nakui](../README.md): la app del ERP, manejada enteramente por manifiestos `module.json`.

Carga cards UiModule desde un directorio y monta un shell Llimphi (sidebar de módulos + menú + área principal) sobre un `NakuiBackend` (event log + replay + snapshot + auto-compact + executors Rhai). Todo el ciclo CRUD corre contra el event log — no hace falta CLI/tests para mutar.

Cinco vistas meta-driven (las cuatro de paridad con el widget GPUI borrado `nahual-widget-meta-form` + `Report`):

- **List** — filas reales del store, búsqueda por `search_in`, orden clickeando el header de columna (asc→desc→sin), paginación, editar/borrar por fila, `👁` a la ficha, `+ Nuevo` y export CSV de las filas filtradas/ordenadas.
- **Form** — un input por `FieldKind` (text/multiline/number/date/boolean/select/entity_ref/auto_id) con foco de teclado; el submit dispara `SeedEntity`, una edición (`update` con delta) o un `Morphism`. Los `EntityRef` se validan antes de escribir.
- **Detail** — la ficha de un record (← Volver / ✎ Editar), sus campos con refs resueltas a un label legible, y listas de records relacionados (back-references vía `via_field`).
- **Dashboard** — una grilla de tarjetas de KPI (`compute_metric`) con `ValueFormat` y filtros. Escalares `Count`/`Sum`/`Avg`/`Min`/`Max` y desgloses por dimensión: `GroupBy` (conteo), `SumBy`/`AvgBy` (suma/promedio de un campo agrupado por otro — *facturación por cliente*, *ticket promedio por plan*). Con `group_ref`, las claves UUID del desglose se resuelven al nombre legible del record referido. Cada desglose tiene un botón `⤓ CSV`. Los filtros (`CardFilter`) aceptan operadores `eq`/`ne`/`gt`/`gte`/`lt`/`lte`/`between`/`non_empty` (comparación numérica o, si no parsea, lexicográfica — sirve para rangos de fecha ISO).
- **Report** — los mismos agregados que un tablero, dispuestos como documento de una columna (título + subtítulo) con un botón **Exportar (.md)** que vuelca el reporte completo a Markdown (escalares en negrita + tablas de desglose).

## Uso

```sh
# default: ./nakui-modules en el cwd
cargo run --release -p nakui-ui-llimphi

# apuntando al demo incluido (clientes + órdenes)
NAKUI_MODULES_DIR=01_yachay/nakui/nakui-ui-llimphi/examples/nakui-modules \
  cargo run --release -p nakui-ui-llimphi
```

Env: `NAKUI_MODULES_DIR` (dir de módulos), `NAKUI_EVENT_LOG` (ruta del WAL), `NAKUI_SNAPSHOT_THRESHOLD` (auto-compact).

## Deps

- [`nakui-core`](../nakui-core/README.md) (`NakuiBackend`), [`nahual-meta-schema`](../../../02_ruway/nahual/libs/meta-schema/), [`nahual-meta-runtime`](../../../02_ruway/nahual/libs/meta-runtime/), [`cards`](../../../02_ruway/cards/)
- [`llimphi-ui`](../../../02_ruway/llimphi/) + widgets `field` / `button` / `text-input` / `banner` / `list` / `app-header`
