# wawa-explorer

> Host-side viewer of Wawa's DAG.

Runs on a Linux host and reads Wawa's filesystem **without mounting anything**: opens the `.img`, walks the content-addressed DAG, shows the tree with detail in Llimphi. Akasha client (raw sockets) to inspect a running Wawa. Useful for debugging, forensics, and education.

## Install

```sh
cargo run --release -p wawa-explorer-llimphi -- /path/to/wawa.img
cargo run --release -p wawa-explorer-llimphi -- akasha://<host>:<port>
```

## Compatibility

- **Linux** — raw sockets require `CAP_NET_RAW` or `setcap`.
- **macOS** — `.img` mode only.
- **Windows** — `.img` mode only.

## Crates

| Crate | Role |
|---|---|
| [`wawa-explorer-core`](wawa-explorer-core/README.md) | `.img` reader, DAG decode. |
| [`wawa-explorer-aoe`](wawa-explorer-aoe/README.md) | Akasha client (raw sockets). |
| [`wawa-explorer-llimphi`](wawa-explorer-llimphi/README.md) | UI: tree + detail panel. |

## Considerations

- **Read-only.** Doesn't mutate the DAG or the live system.
- Akasha is a custom protocol; raw sockets require elevated permissions or `cap_net_raw=p`.
- Useful for validating what `wawa-fs` materializes when something doesn't add up.
