# arje-wasm

> Init WASM runtime for [arje](../../README.md).

`wasmtime` when JIT is available (post-boot), minimal interpreter (`wasmi`) during early boot. Allows writing `arje-brain` rules in WASM for operator extensibility.

## Deps

- `wasmtime`, `wasmi`
