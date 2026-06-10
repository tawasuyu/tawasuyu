# puriy

> `puriy` (quechua: *viajar, caminar*). Navegador web soberano.

![el navegador puriy renderizando una página de tawasuyu con su propio motor: tira de pestañas y barra de URL, banner hero, seis tarjetas de cuadrantes en grilla y un pie — la barra de estado dice "OK · 76 boxes", todas parseadas, estiladas y layouteadas por puriy-engine, sin Chromium en ninguna parte](https://tawasuyu.net/00_unanchay/puriy/pantallazo.png)

Motor DOM/CSS **propio en Rust** (de upstream sólo los parsers: `html5ever`, `markup5ever_rcdom`, `cssparser` como anchor) + JS vía **QuickJS-NG compilado a WASM** (sandbox `wasmi`) + render delegado a **Llimphi**. Servo quedó como inspiración inicial, no como dependencia arquitectónica. Resultado: un navegador sin Chromium ni WebKit, sin FFI a C++ corporativo, pensado para correr idéntico en Linux/Wayland y en Wawa bare-metal. Detalle de arquitectura en [SDD.md](SDD.md).

## Instalación

```sh
# detecta target (Wayland/X11 → Llimphi, sino headless)
cargo run --release -p puriy-app -- https://example.com
cargo run --release -p puriy-app -- https://example.com --target headless
```

## Compatibilidad

- **Linux/Wayland** — Llimphi sobre `mirada` (compositor del monorepo).
- **Wawa bare-metal** — Llimphi sobre framebuffer, sin dependencias del OS host (target de diseño del SDD; el motor JS ya usa el mismo molde `.wasm` + `wasmi` que las apps de wawa).
- Renderiza HTML5 + CSS3 amplio: cascada con especificidad real, herencia, `!important`, `var()`/`calc()`, pseudo-elementos `::before`/`::after`, selectores compound + combinadores + atributos + pseudoclases estructurales + nth-child + not + `:hover`/`:focus`, y cientos de propiedades cableadas al engine — incluyendo alias de vendor `-webkit-`/`-moz-`/`-ms-`/`-epub-` → estándar.
- **JS real**: QuickJS-NG con stdlib **ES2024 completa nativa** (verificado por tests de conformance) + Web APIs por bootstrap modular (~140 módulos: DOM + eventos tipados, fetch/XHR, WebSocket, timers/microtasks, canvas 2D con `getImageData`/`putImageData`, Web Crypto mínimo, storage, y más). Los gaps están en el **wiring nativo** (DOM bindings completos, red, render), no en el lenguaje.

## Crates

| Crate | Rol |
|---|---|
| [`puriy-core`](puriy-core/README.md) | Tipos públicos compartidos. |
| [`puriy-engine`](puriy-engine/README.md) | Fetch + parse HTML/CSS + StyleEngine + box tree. |
| `puriy-js` | Runtime JS: QuickJS-NG (reactor WASI sobre `wasmi`), stdlib ES2024 nativa + ~140 módulos bootstrap de Web APIs. |
| [`puriy-llimphi`](puriy-llimphi/README.md) | Chrome (URL bar, tabs, scroll, links, find, zoom, paneles) + BoxTree → View. |
| [`puriy-app`](puriy-app/README.md) | Binario; autodetecta target Llimphi vs headless. |

## Consideraciones

- **JS completo, wiring parcial.** El lenguaje no es el límite (QuickJS-NG, ES2024 nativo); lo que separa a puriy de las webapps modernas es el wiring nativo restante (DOM bindings completos, red, render) y conformance (web-platform-tests).
- **Cache TTL** respeta `Cache-Control: max-age=N`; entries persistentes en `$XDG_CACHE_HOME/puriy/`.
- **Una pestaña = una historia.** Back/Forward por tab, Ctrl+T/W/Tab para múltiples.
- El motor es propio: de Servo sólo quedan los parsers upstream (`html5ever`/`markup5ever_rcdom`/`cssparser`). Adoptar Stylo entero quedó descartado — la pila propia ya está más alineada con Llimphi y con wawa.
