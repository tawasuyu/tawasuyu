# pluma (wawa app)

> Visor de markdown adentro del kernel de [wawa](../../README.md).

Versión WASM de [`pluma`](../../../../00_unanchay/pluma/README.md) compilada al kernel. Lee atomos del DAG, los muestra; el render usa `pluma-md` + Llimphi-HAL del kernel.

## Build

```sh
./scripts/build-pluma.sh
```

## Deps

- [`pluma-core`](../../../../00_unanchay/pluma/pluma-core/README.md), [`pluma-md`](../../../../00_unanchay/pluma/pluma-md/README.md)
