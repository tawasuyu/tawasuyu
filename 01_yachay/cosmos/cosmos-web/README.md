# cosmos-web

> WASM bindings of [cosmos](../README.md) for browser.

Exposes a functional subset of the engine ([`cosmos-sky`](../cosmos-sky/README.md), [`cosmos-rise-set`](../cosmos-rise-set/README.md), [`cosmos-render`](../cosmos-render/README.md)) as a WASM module consumable from JS. Renders to `<canvas>` with WebGL2; uses the same calculations as the native binary. No full DE-file download — uses a pre-signed reduced version.

## API

```js
import init, { sky_now, position } from 'cosmos_web';
await init();
const sky = sky_now(lat, lon);
```

## Deps

- [`cosmos-sky`](../cosmos-sky/README.md), [`cosmos-render`](../cosmos-render/README.md)
- `wasm-bindgen`, `web-sys`
