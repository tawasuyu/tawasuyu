# pluma (wawa app)

> Markdown reader inside the [wawa](../../README.md) kernel.

WASM version of [`pluma`](../../../../00_unanchay/pluma/README.md) compiled to the kernel. Reads atoms from the DAG, shows them; render uses `pluma-md` + kernel's Llimphi-HAL.

## Build

```sh
./scripts/build-pluma.sh
```

## Deps

- [`pluma-core`](../../../../00_unanchay/pluma/pluma-core/README.md), [`pluma-md`](../../../../00_unanchay/pluma/pluma-md/README.md)
