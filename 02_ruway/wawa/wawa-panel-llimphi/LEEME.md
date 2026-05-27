# wawa-panel-llimphi

> Panel de control Llimphi de [wawa](../README.md).

Vista de estado de apps Wawa: cada app es una card con stats (uptime, mem, llamadas RPC). Botones de deploy/restart/logs. Habla con `wawa-kernel` por virtio-console.

## Uso

```sh
cargo run --release -p wawa-panel-llimphi
```

## Deps

- [`llimphi-ui`](../../llimphi/), `chasqui-nous-real` (a través de virtio)
