# rimay-verbo-daemon-bin

> Binario del daemon de [rimay](../README.md).

Wrapper mínimo que parsea CLI/env, construye la `Config` y arranca [`rimay-verbo-daemon`](../rimay-verbo-daemon/README.md). Permite escoger backend en runtime via `RIMAY_BACKEND={mock,fastembed}`.

## Uso

```sh
RIMAY_BACKEND=fastembed cargo run --release -p rimay-verbo-daemon-bin
RIMAY_BACKEND=mock cargo run --release -p rimay-verbo-daemon-bin
```

## Deps

- [`rimay-verbo-daemon`](../rimay-verbo-daemon/README.md)
- [`rimay-verbo-fastembed`](../rimay-verbo-fastembed/README.md) (feature `fastembed`)
- [`rimay-verbo-mock`](../rimay-verbo-mock/README.md) (siempre, para fallback)
