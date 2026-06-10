# forth-emisor — compilador Forth→WASM para wawa

Toma un dialecto **Forth** y emite un módulo **WASM** válido. Núcleo `no_std`
reusable: lo consume el pipeline de apps de wawa (`build-pluma.sh`, etc.) para
escribir lógica de bajo nivel en Forth y correrla en el cage WASM del kernel.

## Capacidades

- Definición de palabras y macros.
- ABI `(i32) -> i32`.
- Cascade injection a través de macros importadas (Fase 40).

## Estado (2026-05-31)

### Hecho
- Emisión de módulos WASM válidos desde el dialecto Forth.
- Macros + ABI `(i32) -> i32` + cascade injection across imported macros (Fase 40).
- Núcleo `no_std` (cruza al pipeline de wawa); ≈9 tests.

### Pendiente
- ABI más rica (múltiples args/retornos, tipos no-i32).
- Optimización del WASM emitido (hoy se apoya en `wasm-opt` aguas abajo).
- Diagnósticos/errores de compilación más finos.

## Lugar en el repo

`shared/forth-emisor` — extraído del kernel de wawa (Fase 30). Consumido por el
pipeline de apps WASM de `03_ukupacha/wawa`.
