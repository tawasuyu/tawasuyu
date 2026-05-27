# puriy

> `puriy` (Quechua: *to travel, to walk*). Sovereign web browser.

DOM/CSS/JS engine based on **Servo** (Rust-native) + render delegated to **Llimphi**. Result: a browser identical on Linux/Wayland and on Wawa bare-metal, with no Chromium or WebKit, no FFI to corporate C++. Architecture in [SDD.md](SDD.md).

## Install

```sh
# auto-detects target (Wayland/X11 → Llimphi, else headless)
cargo run --release -p puriy-app -- https://example.com
cargo run --release -p puriy-app -- https://example.com --target headless
```

## Compatibility

- **Linux/Wayland** — Llimphi on `mirada` (the monorepo's compositor).
- **Wawa bare-metal** — Llimphi on framebuffer; no host-OS deps.
- Renders HTML5 + CSS3 subset (compound selectors + combinators + attributes + structural pseudo-classes + nth-child + not; `width`/`max-width`, `text-align`, `line-height`, `border`/`border-radius`, `box-shadow`, `text-decoration`, `list-style-type`); `:hover` with limited scope; no JS yet.

## Crates

| Crate | Role |
|---|---|
| [`puriy-core`](puriy-core/README.md) | Shared public types. |
| [`puriy-engine`](puriy-engine/README.md) | Fetch + HTML/CSS parse + StyleEngine + box tree. |
| [`puriy-llimphi`](puriy-llimphi/README.md) | Chrome (URL bar, tabs, scroll, links) + BoxTree → View. |
| [`puriy-app`](puriy-app/README.md) | Binary; auto-detects Llimphi vs headless. |

## Considerations

- **No JS yet.** Static or gracefully-degrading pages work; modern webapps don't.
- **Cache TTL** honors `Cache-Control: max-age=N`; persistent entries in `$XDG_CACHE_HOME/puriy/`.
- **One tab = one history.** Per-tab back/forward, Ctrl+T/W/Tab for multi-tab.
- Servo is kept current; Stylo (Firefox's CSS engine via Servo) is Option B for a later phase.
