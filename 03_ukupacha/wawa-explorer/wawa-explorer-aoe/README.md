# wawa-explorer-aoe

> Cliente Akasha (raw sockets) para [wawa-explorer](../README.md).

Conecta a una instancia Wawa corriendo via el protocolo Akasha (raw socket ethernet, no TCP). Recibe snapshots del DAG en vivo sin montar nada.

## Compatibilidad

- **Linux**: requiere `CAP_NET_RAW` o `setcap cap_net_raw=p`.
- **macOS / Windows**: no soportado (raw sockets bloqueados).

## Deps

- [`wawa-explorer-core`](../wawa-explorer-core/README.md)
- `socket2`, `nix`
