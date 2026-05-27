# wawa-fs

> Filesystem (BLAKE3 DAG) of [wawa](../README.md).

Chunks content-addressed by BLAKE3. Manifest tree represents the POSIX-like structure. **Immutable**: an edit = new chunks + new manifest. Ingestion (POSIX → BLAKE3) happens in the host distiller; the kernel only reads.

## Usage

```sh
cargo run --release -p wawa-fs -- --image /path/to/wawa.img
```

## Deps

- `blake3`, `serde`, `postcard`
