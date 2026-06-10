# puriy

> `puriy` (Quechua: *to travel, to walk*). Sovereign web browser.

![the puriy browser rendering a tawasuyu page with its own engine: tab strip and URL bar, hero banner, six quadrant cards in a grid and a footer — the status bar reads "OK · 76 boxes", every one parsed, styled and laid out by puriy-engine, no Chromium anywhere](https://tawasuyu.net/00_unanchay/puriy/pantallazo.png)

**Own DOM/CSS engine in Rust** (from upstream only the parsers: `html5ever`, `markup5ever_rcdom`, `cssparser` as anchor) + JS via **QuickJS-NG compiled to WASM** (`wasmi` sandbox) + render delegated to **Llimphi**. Servo remained the initial inspiration, not an architectural dependency. Result: a browser with no Chromium or WebKit, no FFI to corporate C++, designed to run identically on Linux/Wayland and on Wawa bare-metal. Architecture in [SDD.md](SDD.md).

## Install

```sh
# auto-detects target (Wayland/X11 → Llimphi, else headless)
cargo run --release -p puriy-app -- https://example.com
cargo run --release -p puriy-app -- https://example.com --target headless
```

## Compatibility

- **Linux/Wayland** — Llimphi on `mirada` (the monorepo's compositor).
- **Wawa bare-metal** — Llimphi on framebuffer, no host-OS deps (the SDD's design target; the JS engine already uses the same `.wasm` + `wasmi` mold as wawa's apps).
- Renders HTML5 + broad CSS3: cascade with real specificity, inheritance, `!important`, `var()`/`calc()`, `::before`/`::after` pseudo-elements, compound selectors + combinators + attributes + structural pseudo-classes + nth-child + not + `:hover`/`:focus`, and hundreds of properties plumbed into the engine — including vendor-prefix aliases `-webkit-`/`-moz-`/`-ms-`/`-epub-` → standard.
- **Real JS**: QuickJS-NG with a **complete native ES2024** stdlib (verified by conformance tests) + Web APIs via modular bootstrap (~140 modules: DOM + typed events, fetch/XHR, WebSocket, timers/microtasks, canvas 2D with `getImageData`/`putImageData`, minimal Web Crypto, storage, and more). The gaps live in the **native wiring** (full DOM bindings, networking, render), not in the language.

## Crates

| Crate | Role |
|---|---|
| [`puriy-core`](puriy-core/README.md) | Shared public types. |
| [`puriy-engine`](puriy-engine/README.md) | Fetch + HTML/CSS parse + StyleEngine + box tree. |
| `puriy-js` | JS runtime: QuickJS-NG (WASI reactor over `wasmi`), native ES2024 stdlib + ~140 bootstrap modules of Web APIs. |
| [`puriy-llimphi`](puriy-llimphi/README.md) | Chrome (URL bar, tabs, scroll, links, find, zoom, panels) + BoxTree → View. |
| [`puriy-app`](puriy-app/README.md) | Binary; auto-detects Llimphi vs headless. |

## Considerations

- **Complete JS, partial wiring.** The language is not the limit (QuickJS-NG, native ES2024); what separates puriy from modern webapps is the remaining native wiring (full DOM bindings, networking, render) and conformance (web-platform-tests).
- **Cache TTL** honors `Cache-Control: max-age=N`; persistent entries in `$XDG_CACHE_HOME/puriy/`.
- **One tab = one history.** Per-tab back/forward, Ctrl+T/W/Tab for multi-tab.
- The engine is our own: of Servo only the upstream parsers remain (`html5ever`/`markup5ever_rcdom`/`cssparser`). Adopting Stylo wholesale was discarded — the in-house stack is already better aligned with Llimphi and wawa.
