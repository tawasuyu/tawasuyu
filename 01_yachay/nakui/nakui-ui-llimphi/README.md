# nakui-ui-llimphi

> Meta-interface shell of [nakui](../README.md): the ERP app, driven entirely by `module.json` manifests.

Loads UiModule cards from a directory and mounts a Llimphi shell (module sidebar + menu + main area) over a `NakuiBackend` (event log + replay + snapshot + auto-compact + Rhai executors). The whole CRUD loop runs against the event log — no CLI/tests needed to mutate.

Four meta-driven views, parity with the deleted `nahual-widget-meta-form` GPUI widget:

- **List** — real rows from the store, `search_in` search, click-to-sort columns (asc→desc→off), pagination, per-row edit/delete, `👁` to the detail card, `+ Nuevo`, and CSV export of the filtered/sorted rows.
- **Form** — one input per `FieldKind` (text/multiline/number/date/boolean/select/entity_ref/auto_id) with keyboard focus; submit fires `SeedEntity`, an edit (`update` with delta) or a `Morphism`. `EntityRef`s are validated before writing.
- **Detail** — a record's card (← Volver / ✎ Editar), its fields with refs resolved to a readable label, and related-record lists (back-references via `via_field`).
- **Dashboard** — a grid of KPI cards computing `Count`/`Sum`/`GroupBy` (`compute_metric`) with `ValueFormat` and filters.

## Usage

```sh
# default: ./nakui-modules in the cwd
cargo run --release -p nakui-ui-llimphi

# point at the bundled demo (clientes + órdenes)
NAKUI_MODULES_DIR=01_yachay/nakui/nakui-ui-llimphi/examples/nakui-modules \
  cargo run --release -p nakui-ui-llimphi
```

Env: `NAKUI_MODULES_DIR` (modules dir), `NAKUI_EVENT_LOG` (WAL path), `NAKUI_SNAPSHOT_THRESHOLD` (auto-compact).

## Deps

- [`nakui-core`](../nakui-core/README.md) (`NakuiBackend`), [`nahual-meta-schema`](../../../02_ruway/nahual/libs/meta-schema/), [`nahual-meta-runtime`](../../../02_ruway/nahual/libs/meta-runtime/), [`cards`](../../../02_ruway/cards/)
- [`llimphi-ui`](../../../02_ruway/llimphi/) + widgets `field` / `button` / `text-input` / `banner` / `list` / `app-header`
