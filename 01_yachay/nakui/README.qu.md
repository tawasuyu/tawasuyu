<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# nakui

> Excel-laya motor reactivo, sumaq cimientupatapi.

`nakui` huk hoja de cálculo umayuq: cheqaq `Decimal` (mana `f64`), topológika kuti-purichiq, WAL ñawpaqman, atómikos invariantes, ñawpaq pacha-puriy mana tikraq historialwan. Kimsa qhawanakuna kikin token-grafu patapi — **matriz** (Excel hina), **grafu** (DAG dependencia), **formulario** (huk record) — schema-pa `view_hint`-ninwan kamachisqa.

Hatun pacha qhawana: nakui huk **DAG token motor** hina, vertikalkunapaq kawsay (Fintech, IAM, Logística, MedTech).

## Churay

```sh
cargo run --release -p nakui-ui-llimphi
cargo run --release -p nakui-sheet-llimphi
cargo run --release -p nakui-explorer-llimphi
```

## Tinkuy

- **Linux / macOS / Windows** — Llimphi UI.
- **Wawa** — `nakui-core` + `nakui-sheet-nakuicore` WASM-man wiñakun.
- Lokal waqaychay WALwan `$XDG_DATA_HOME/nakui/`-pi.

## Crateskuna

[`nakui-core`](nakui-core/README.md), [`nakui-sheet`](nakui-sheet/README.md), [`nakui-sheet-nakuicore`](nakui-sheet-nakuicore/README.md), [`nakui-sheet-llimphi`](nakui-sheet-llimphi/README.md), [`nakui-ui-llimphi`](nakui-ui-llimphi/README.md), [`nakui-explorer-llimphi`](nakui-explorer-llimphi/README.md).

## Yuyaykunaq

- **Mana `f64`.** Cheqaq `Decimal` motor pacha; numérik formato qhawanapi, mana datospi.
- **WAL ñawpaqman tikrana.** Sapanka ruway log-rayku puriq; ukhupi datu cuando WAL chayaqtinraq tikran.
- **Mana Excel.** Mana XLSX fórmula tinkuy; formularios + grafu first-class.
