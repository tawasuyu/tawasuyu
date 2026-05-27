# chasqui-broker-explorer-llimphi

> Llimphi UI: topics + active subscribers of [chasqui](../README.md)'s broker.

Lists current topics, how many subscribers each has, throughput per topic, persistence active or not. Useful for diagnosing the system.

## Usage

```sh
cargo run --release -p chasqui-broker-explorer-llimphi
```

## Deps

- [`chasqui-core`](../chasqui-core/README.md), [`chasqui-nous-real`](../chasqui-nous-real/README.md)
- [`llimphi-ui`](../../llimphi/) + widgets `list`, `tabs`
