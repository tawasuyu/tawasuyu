# arje-wasm

> Runtime WASM de init para [arje](../../README.md).

`wasmtime` cuando hay JIT (post-boot), interpreter mínimo (`wasmi`) durante boot temprano. Permite escribir las reglas de `arje-brain` en WASM por si el operador quiere extender.

## Deps

- `wasmtime`, `wasmi`
