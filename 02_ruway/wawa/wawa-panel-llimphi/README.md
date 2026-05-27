# wawa-panel-llimphi

> Llimphi control panel of [wawa](../README.md).

View of Wawa app state: each app is a card with stats (uptime, mem, RPC calls). Buttons for deploy/restart/logs. Talks to `wawa-kernel` over virtio-console.

## Usage

```sh
cargo run --release -p wawa-panel-llimphi
```

## Deps

- [`llimphi-ui`](../../llimphi/), `chasqui-nous-real` (through virtio)
