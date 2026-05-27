# arje-zero

> Zero point: the first thing that runs inside [arje](../../README.md).

PID 0 / minimal init. Mounts `/proc`, `/sys`, `/dev`. Brings up [`arje-soma`](../arje-soma/README.md). Doesn't launch a shell by default — waits for orders from soma.

## Deps

- `nix` (mounts, syscalls)
