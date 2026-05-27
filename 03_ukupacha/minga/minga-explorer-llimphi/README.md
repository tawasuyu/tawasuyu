# minga-explorer-llimphi

> UI: peers, content, traffic of [minga](../README.md).

Three tabs: peers (per-connection state), content (what you have locally), traffic (bandwidth/req-rate per peer). Useful for diagnosing the network.

## Usage

```sh
cargo run --release -p minga-explorer-llimphi
```

## Deps

- All `minga-*`
- [`llimphi-ui`](../../../02_ruway/llimphi/)
