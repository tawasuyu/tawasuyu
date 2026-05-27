# puriy

> `puriy` (quechua: *viajar, caminar*). Navegador web soberano.

Engine de DOM/CSS/JS basado en **Servo** (Rust nativo) + render delegado a **Llimphi**. Resultado: un navegador idéntico en Linux/Wayland y en Wawa bare-metal, sin Chromium ni WebKit, sin FFI a C++ corporativo. Detalle de arquitectura en [SDD.md](SDD.md).

## Instalación

```sh
# detecta target (Wayland/X11 → Llimphi, sino headless)
cargo run --release -p puriy-app -- https://example.com
cargo run --release -p puriy-app -- https://example.com --target headless
```

## Compatibilidad

- **Linux/Wayland** — Llimphi sobre `mirada` (compositor del monorepo).
- **Wawa bare-metal** — Llimphi sobre framebuffer; sin dependencias del OS host.
- Renderiza HTML5 + subset de CSS3 (selectores compound + combinadores + atributos + pseudoclases estructurales + nth-child + not; `width`/`max-width`, `text-align`, `line-height`, `border`/`border-radius`, `box-shadow`, `text-decoration`, `list-style-type`); `:hover` con scope limitado; JS aún no.

## Crates

| Crate | Rol |
|---|---|
| [`puriy-core`](puriy-core/README.md) | Tipos públicos compartidos. |
| [`puriy-engine`](puriy-engine/README.md) | Fetch + parse HTML/CSS + StyleEngine + box tree. |
| [`puriy-llimphi`](puriy-llimphi/README.md) | Chrome (URL bar, tabs, scroll, links) + BoxTree → View. |
| [`puriy-app`](puriy-app/README.md) | Binario; autodetecta target Llimphi vs headless. |

## Consideraciones

- **Sin JS aún.** Páginas estáticas o que degradan limpio funcionan; webapps modernas no.
- **Cache TTL** respeta `Cache-Control: max-age=N`; entries persistentes en `$XDG_CACHE_HOME/puriy/`.
- **Una pestaña = una historia.** Back/Forward por tab, Ctrl+T/W/Tab para múltiples.
- Servo se mantiene actualizado; Stylo (motor CSS de Firefox via Servo) está como Opción B para una fase posterior.
