# arje-zero

> Punto cero: lo primero que corre dentro de [arje](../../README.md).

PID 0 / init mínimo. Monta `/proc`, `/sys`, `/dev`. Levanta [`arje-soma`](../arje-soma/README.md). No ejecuta shell por default — espera órdenes de soma.

## Deps

- `nix` (mounts, syscalls)
