# wawa-explorer-core

> `.img` reader + DAG decode for [wawa-explorer](../README.md).

Parses the `.img` format of [`wawa-fs`](../../wawa/wawa-fs/README.md): manifest tree + indexed chunks. Verifies BLAKE3 on read. Never mutates.

## Deps

- [`wawa-fs`](../../wawa/wawa-fs/README.md) (read-only), `blake3`, `serde`
