# wawa-explorer-aoe

> Akasha client (raw sockets) for [wawa-explorer](../README.md).

Connects to a running Wawa instance via the Akasha protocol (raw ethernet socket, not TCP). Receives live DAG snapshots without mounting anything.

## Compatibility

- **Linux**: requires `CAP_NET_RAW` or `setcap cap_net_raw=p`.
- **macOS / Windows**: not supported (raw sockets blocked).

## Deps

- [`wawa-explorer-core`](../wawa-explorer-core/README.md)
- `socket2`, `nix`
