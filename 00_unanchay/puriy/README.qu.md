<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# puriy

> `puriy` (runa-simi: *viajar, puriy, ñan*). Sapan kaq web qhawakuq.

**Servo**-pi (Rust ch'uya) DOM/CSS/JS motor + **Llimphi**-man render-haywariy. Tukukuy: huk navegador Linux/Wayland-pipas Wawa-bare-metal-pipas kikin, mana Chromium, mana WebKit, mana C++-corporativo FFI. SDD ukhupi [SDD.md](SDD.md).

## Churay

```sh
# target awto-rikuq (Wayland/X11 → Llimphi, mana → headless)
cargo run --release -p puriy-app -- https://example.com
cargo run --release -p puriy-app -- https://example.com --target headless
```

## Tinkuy

- **Linux/Wayland** — Llimphi `mirada`-pa patanpi (monorepo compositorpaq).
- **Wawa bare-metal** — Llimphi framebuffer patanpi; OS host deps illaq.
- HTML5 + CSS3 subset rikuchiy (compound selectors + combinators + attributos + estructural pseudo-classes + nth-child + not; `width`/`max-width`, `text-align`, `line-height`, `border`/`border-radius`, `box-shadow`, `text-decoration`, `list-style-type`); `:hover` pisi scopepi; JS mananraq.

## Crateskuna

| Crate | Ima ruwan |
|---|---|
| [`puriy-core`](puriy-core/README.md) | Hawa-tipo huñunakuna. |
| [`puriy-engine`](puriy-engine/README.md) | Fetch + HTML/CSS parse + StyleEngine + box tree. |
| [`puriy-llimphi`](puriy-llimphi/README.md) | Chrome (URL bar, tabs, scroll, links) + BoxTree → View. |
| [`puriy-app`](puriy-app/README.md) | Binario; awto-rikuq Llimphi/headless. |

## Yuyaykunaq

- **JS mananraq.** Ch'uya páginakuna utaq sumaq tikraqkuna llank'ankuqa; mosoq web aplikacionkuna mana.
- **Cache TTL** `Cache-Control: max-age=N` yupaychan; mantenidos entries `$XDG_CACHE_HOME/puriy/`-pi.
- **Sapanka tab = sapanka historial.** Tab-paq back/forward, Ctrl+T/W/Tab tukuy-tabs-paq.
- Servo musuq qhipapi; Stylo (Firefox CSS motor Servo-rayku) Opción B qhipakaqpaq pacha.
