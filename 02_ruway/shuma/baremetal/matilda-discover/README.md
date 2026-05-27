# matilda-discover

> Current-state discovery of [matilda](../../README.md).

Reads the system (installed packages, files in `/etc`, systemd services) and produces the "actual" `HostConfig`. Comparable with the desired state to compute the diff.

## Deps

- [`matilda-core`](../matilda-core/README.md)
- `dbus` (systemd), `walkdir`
