# mirada-ctl

> Control CLI of [mirada](../README.md).

Commands: `mirada-ctl workspace next`, `mirada-ctl window close`, `mirada-ctl layout tiled`, etc. Talks to the compositor via [`mirada-link`](../mirada-link/README.md).

## Usage

```sh
cargo run --release -p mirada-ctl -- workspace next
```

## Deps

- [`mirada-link`](../mirada-link/README.md)
- `clap`
