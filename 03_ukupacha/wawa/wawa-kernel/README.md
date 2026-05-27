# wawa-kernel

> Kernel de [wawa](../README.md).

Scheduler cooperativo (no preemptivo entre apps WASM), syscall table mínima, capability-based security (cada app declara qué puede tocar; sin el cap, syscall falla), IPC via channels tipados.

## Build

```sh
cargo build --release -p wawa-kernel
```

## Deps

- `wasmtime` (con `cranelift`), `serde`, `blake3`
