# nakui

> Motor reactivo tipo Excel, sobre principios sólidos.

`nakui` es una hoja de cálculo con cabeza: `Decimal` exacto (no `f64`), cascada por orden topológico, WAL antes de aplicar, invariantes atómicos, time-travel por historial inmutable. Tres vistas sobre el mismo grafo de tokens — **matriz** (Excel clásico), **grafo** (DAG de dependencias), **formulario** (record único) — guiadas por `view_hint` del schema.

Visión a futuro: nakui como **motor de tokens en DAG**, base para verticales (Fintech, IAM, Logística, MedTech).

## Instalación

```sh
# UI Llimphi
cargo run --release -p nakui-ui-llimphi
cargo run --release -p nakui-sheet-llimphi
cargo run --release -p nakui-explorer-llimphi
```

## Compatibilidad

- **Linux / macOS / Windows** — UI Llimphi.
- **Wawa** — `nakui-core` y `nakui-sheet-nakuicore` compilan a WASM.
- Persistencia local con WAL en `$XDG_DATA_HOME/nakui/`.

## Crates

| Crate | Rol |
|---|---|
| [`nakui-core`](nakui-core/README.md) | Motor: tokens, schema, DAG, cascada, WAL. |
| [`nakui-sheet`](nakui-sheet/README.md) | Vista matriz (rangos, celdas, fórmulas). |
| [`nakui-sheet-nakuicore`](nakui-sheet-nakuicore/README.md) | Bridge nakui-sheet ↔ nakui-core. |
| [`nakui-sheet-llimphi`](nakui-sheet-llimphi/README.md) | UI matriz Llimphi. |
| [`nakui-ui-llimphi`](nakui-ui-llimphi/README.md) | Shell de UI (selector de vista, panel). |
| [`nakui-explorer-llimphi`](nakui-explorer-llimphi/README.md) | Explorer del grafo de tokens. |

## Consideraciones

- **Sin `f64`**. `Decimal` exacto en todo el motor; los formatos numéricos viven en la vista, no en el dato.
- **WAL antes que mutar**: cada operación pasa por log; el dato en memoria sólo cambia cuando el WAL se sincronizó.
- **No es Excel.** No buscamos compatibilidad de fórmulas con XLSX; los formularios y el grafo son first-class, no addons.
