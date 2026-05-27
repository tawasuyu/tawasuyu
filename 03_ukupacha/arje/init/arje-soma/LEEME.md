# arje-soma

> "Cuerpo" del sistema en runtime para [arje](../../README.md).

Daemon principal post-boot: lee el manifest del sistema, levanta los servicios listados, los observa, los reinicia si caen. Equivalente conceptual de `systemd-init`, mucho más chico.

## Deps

- [`arje-bus`](../../runtime/arje-bus/README.md), [`arje-incarnate`](../arje-incarnate/README.md)
