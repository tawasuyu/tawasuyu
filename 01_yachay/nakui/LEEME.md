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
| [`nakui-backend`](nakui-backend/) | Backend agnóstico de GUI: MemoryStore + EventLog + executors, persistencia WAL/snapshot con recovery. |
| [`nakui-sheet`](nakui-sheet/README.md) | Vista matriz (rangos, celdas, fórmulas, motor de pivot). |
| [`nakui-sheet-nakuicore`](nakui-sheet-nakuicore/README.md) | Bridge nakui-sheet ↔ nakui-core. |
| [`nakui-sheet-llimphi`](nakui-sheet-llimphi/README.md) | UI matriz Llimphi. |
| [`nakui-ui-llimphi`](nakui-ui-llimphi/README.md) | Shell de UI (selector de vista, panel). |
| [`nakui-explorer-llimphi`](nakui-explorer-llimphi/README.md) | Explorer del grafo de tokens. |

Los módulos de producción viven en [`modules/`](modules/) — `crm`, `inventory`, `sales`, `treasury`: schema Nickel (`schema.ncl`) + morfismos Rhai, ejecutados por el kernel de executors de `nakui-core` (133/133 tests verdes).

## Consideraciones

- **Sin `f64`**. `Decimal` exacto en todo el motor; los formatos numéricos viven en la vista, no en el dato.
- **WAL antes que mutar**: cada operación pasa por log; el dato en memoria sólo cambia cuando el WAL se sincronizó.
- **No es Excel.** No buscamos compatibilidad de fórmulas con XLSX; los formularios y el grafo son first-class, no addons.

## Estado (2026-06-09)

### Hecho

- Motor `nakui-sheet`: modelo de celdas/valores (`cell.rs`, `value.rs`),
  grafo de dependencias (`graph.rs`), cascada reactiva y workbook
  (`sheet.rs`, `workbook.rs`), I/O CSV (`csv_io.rs`) y sinks (`sink.rs`).
- Parser de fórmulas + catálogo de funciones por categoría
  (`formula/`, splitteado desde el megafile `funcs.rs` de ~1.9k LOC).
- `nakui-core`: motor de tokens/schema/DAG con cascada y WAL (base de las
  tres vistas) — 133/133 tests verdes, incluyendo los suites de integración
  de los módulos `crm`/`inventory`/`sales`/`treasury` (`modules/`, schema
  Nickel + morfismos Rhai como `register_cash_move` y
  `transfer_between_cajas`).
- `nakui-backend`: backend agnóstico de GUI extraído de la UI (regla #2) —
  MemoryStore + EventLog + executors por módulo, persistencia WAL/snapshot
  con recovery y auto-compaction.
- Bridge `nakui-sheet-nakuicore` entre la vista matriz y el motor de tokens.
- UI Llimphi viva: `nakui-sheet-llimphi` (matriz), `nakui-ui-llimphi`
  (shell + selector de vista), `nakui-explorer-llimphi` (grafo de morfismos
  con zoom + fit-to-view). Menús principal y contextual de edición añadidos.
- Métricas/desgloses: SumBySeries multi-serie, columnas apiladas, running
  total, tesorería (saldo acumulado), count_distinct, labels legibles; el
  motor de tabla dinámica vive en `nakui_sheet::pivot`.
- Edición in-situ en la ficha de detalle (click en un campo abre el editor
  en el lugar, sin form aparte) + stat cards con la firma de panel del kit
  (`panel_signature_painter`, coherencia visual Nivel 4).
- Megafiles de los binarios Llimphi (~2k LOC) splitteados en módulos.

### Pendiente

- Consolidar la vista **formulario** (record único guiado por `view_hint`)
  con paridad funcional frente a matriz y grafo.
- Editor de fórmulas en la matriz Llimphi con autocompletado y errores inline.
- Cerrar persistencia WAL end-to-end desde la UI (`$XDG_DATA_HOME/nakui/`).
- Time-travel navegable desde la interfaz (historial inmutable expuesto).
- Avanzar hacia el meta-modelo de verticales (Fintech, IAM, Logística,
  MedTech) sobre el motor de tokens.
