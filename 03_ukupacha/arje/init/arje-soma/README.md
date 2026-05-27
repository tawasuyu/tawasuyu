# arje-soma

> System runtime "body" of [arje](../../README.md).

Main daemon post-boot: reads the system manifest, brings up the listed services, watches them, restarts if they fall. Conceptual equivalent of `systemd-init`, much smaller.

## Deps

- [`arje-bus`](../../runtime/arje-bus/README.md), [`arje-incarnate`](../arje-incarnate/README.md)
