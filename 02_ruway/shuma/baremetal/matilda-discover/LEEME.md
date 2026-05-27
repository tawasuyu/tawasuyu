# matilda-discover

> Descubrimiento de estado actual de [matilda](../../README.md).

Lee el sistema (paquetes instalados, archivos en `/etc`, servicios systemd) y produce un `HostConfig` "actual". Comparable con el deseado para calcular el diff.

## Deps

- [`matilda-core`](../matilda-core/README.md)
- `dbus` (systemd), `walkdir`
