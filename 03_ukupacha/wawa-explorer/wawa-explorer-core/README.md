# wawa-explorer-core

> Lectura del `.img` + decode del DAG para [wawa-explorer](../README.md).

Parsea el formato `.img` de [`wawa-fs`](../../wawa/wawa-fs/README.md): manifest tree + chunks indexados. Verifica BLAKE3 al leer. No muta nunca.

## Deps

- [`wawa-fs`](../../wawa/wawa-fs/README.md) (sólo lectura), `blake3`, `serde`
