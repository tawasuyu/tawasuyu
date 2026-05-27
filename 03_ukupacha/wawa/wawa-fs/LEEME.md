# wawa-fs

> Filesystem (DAG BLAKE3) de [wawa](../README.md).

Chunks content-addressed por BLAKE3. Manifest tree representa la estructura POSIX-like. **Inmutable**: una edición = nuevos chunks + nuevo manifest. La ingesta (POSIX → BLAKE3) la hace el destilador en el host; el kernel sólo lee.

## Uso

```sh
cargo run --release -p wawa-fs -- --image /path/to/wawa.img
```

## Deps

- `blake3`, `serde`, `postcard`
