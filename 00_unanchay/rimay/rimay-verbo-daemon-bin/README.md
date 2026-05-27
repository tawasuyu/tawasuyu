# rimay-verbo-daemon-bin

> Daemon binary of [rimay](../README.md).

Minimal wrapper that parses CLI/env, builds `Config` and starts [`rimay-verbo-daemon`](../rimay-verbo-daemon/README.md). Lets you pick backend at runtime via `RIMAY_BACKEND={mock,fastembed}`.

## Usage

```sh
RIMAY_BACKEND=fastembed cargo run --release -p rimay-verbo-daemon-bin
RIMAY_BACKEND=mock cargo run --release -p rimay-verbo-daemon-bin
```

## Deps

- [`rimay-verbo-daemon`](../rimay-verbo-daemon/README.md)
- [`rimay-verbo-fastembed`](../rimay-verbo-fastembed/README.md) (feature `fastembed`)
- [`rimay-verbo-mock`](../rimay-verbo-mock/README.md) (always, as fallback)
