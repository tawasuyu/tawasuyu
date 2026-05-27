# hello_wasm (wawa app)

> Hello-world WASM para [wawa](../../README.md).

App mínima de prueba: imprime "hola" y devuelve un exit code. Sirve para verificar la cadena `cargo build → WASM → kernel load → run`. Punto de partida para nuevas apps.

## Build

```sh
cargo build --release -p hello_wasm --target wasm32-wasip1
```
