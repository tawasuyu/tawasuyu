# arje-bus

> Bus interno (IPC arje) para [arje](../../README.md).

Comunicación entre `arje-zero`, `arje-soma`, daemons. Topics tipados como [`chasqui`](../../../../02_ruway/chasqui/README.md) pero **dentro del kernel space** y sin red — usa Unix sockets / shared memory.

## Deps

- `serde`, `nix`
