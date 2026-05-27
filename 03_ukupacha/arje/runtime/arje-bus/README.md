# arje-bus

> Internal bus (arje IPC) for [arje](../../README.md).

Communication between `arje-zero`, `arje-soma`, daemons. Typed topics like [`chasqui`](../../../../02_ruway/chasqui/README.md) but **inside kernel space** and without network — uses Unix sockets / shared memory.

## Deps

- `serde`, `nix`
