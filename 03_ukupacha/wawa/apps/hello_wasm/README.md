# hello_wasm (wawa app)

> Hello-world WASM for [wawa](../../README.md).

Minimal test app: prints "hello" and returns an exit code. Verifies the `cargo build → WASM → kernel load → run` chain. Starting point for new apps.

## Build

```sh
cargo build --release -p hello_wasm --target wasm32-wasip1
```
