# cosmos-web

> Bindings WASM de [cosmos](../README.md) para navegador.

Expone un subset funcional del engine ([`cosmos-sky`](../cosmos-sky/README.md), [`cosmos-rise-set`](../cosmos-rise-set/README.md), [`cosmos-render`](../cosmos-render/README.md)) como módulo WASM consumible desde JS. Render a `<canvas>` con WebGL2; usa los mismos cálculos que el binario nativo. Sin descargar DE files completos — usa una versión reducida prefirmada.

## API

```js
import init, { sky_now, position } from 'cosmos_web';
await init();
const sky = sky_now(lat, lon);
```

## Deps

- [`cosmos-sky`](../cosmos-sky/README.md), [`cosmos-render`](../cosmos-render/README.md)
- `wasm-bindgen`, `web-sys`
