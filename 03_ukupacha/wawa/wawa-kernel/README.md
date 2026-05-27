# wawa-kernel

> Kernel of [wawa](../README.md).

Cooperative scheduler (non-preemptive between WASM apps), minimal syscall table, capability-based security (each app declares what it can touch; without the cap, syscall fails), IPC via typed channels.

## Build

```sh
cargo build --release -p wawa-kernel
```

## Deps

- `wasmtime` (with `cranelift`), `serde`, `blake3`
