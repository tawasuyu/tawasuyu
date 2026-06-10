# nakui

> Excel-style reactive engine, on solid principles.

`nakui` is a spreadsheet with a head: exact `Decimal` (not `f64`), topological-order cascade, WAL before applying, atomic invariants, time-travel via immutable history. Three views over the same token graph — **matrix** (classic Excel), **graph** (dependency DAG), **form** (single record) — guided by the schema's `view_hint`.

Long-term vision: nakui as a **DAG token engine**, foundation for verticals (Fintech, IAM, Logistics, MedTech).

## Install

```sh
# Llimphi UI
cargo run --release -p nakui-ui-llimphi
cargo run --release -p nakui-sheet-llimphi
cargo run --release -p nakui-explorer-llimphi
```

## Compatibility

- **Linux / macOS / Windows** — Llimphi UI.
- **Wawa** — `nakui-core` and `nakui-sheet-nakuicore` compile to WASM.
- Local persistence with WAL in `$XDG_DATA_HOME/nakui/`.

## Crates

| Crate | Role |
|---|---|
| [`nakui-core`](nakui-core/README.md) | Engine: tokens, schema, DAG, cascade, WAL. |
| [`nakui-backend`](nakui-backend/) | GUI-agnostic backend: MemoryStore + EventLog + executors, WAL/snapshot persistence with recovery. |
| [`nakui-sheet`](nakui-sheet/README.md) | Matrix view (ranges, cells, formulas, pivot engine). |
| [`nakui-sheet-nakuicore`](nakui-sheet-nakuicore/README.md) | nakui-sheet ↔ nakui-core bridge. |
| [`nakui-sheet-llimphi`](nakui-sheet-llimphi/README.md) | Matrix UI Llimphi. |
| [`nakui-ui-llimphi`](nakui-ui-llimphi/README.md) | UI shell (view selector, panel). |
| [`nakui-explorer-llimphi`](nakui-explorer-llimphi/README.md) | Token-graph explorer. |

Production modules live in [`modules/`](modules/) — `crm`, `inventory`, `sales`, `treasury`: Nickel schema (`schema.ncl`) + Rhai morphisms, executed by the `nakui-core` executor kernel (133/133 tests green).

## Considerations

- **No `f64`.** Exact `Decimal` throughout the engine; numeric format lives in the view, not the data.
- **WAL before mutate.** Every operation goes through the log; in-memory data only changes when the WAL synced.
- **Not Excel.** No formula compatibility with XLSX; forms and the graph are first-class, not addons.
