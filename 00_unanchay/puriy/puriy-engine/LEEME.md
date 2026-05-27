# puriy-engine

> Engine de [puriy](../README.md): fetch + HTML/CSS parse + StyleEngine + box tree.

Núcleo no-gráfico del navegador. Pipeline: `fetch → parse_html → parse_styles → build_box_tree → BoxTree`. Usa `html5ever` (DOM tree), `markup5ever_rcdom` (Rc-DOM), parser CSS propio (subset documentado en el [SDD](../SDD.md)), `ureq` para HTTP síncrono (sin tokio). Cache LRU 64 MB con TTL por `Cache-Control: max-age`.

## Soporte CSS

Selectores: tag, `.class`, `#id`, atributos (`[attr]`, `[attr=v]`, `[attr^=v]`, `[attr$=v]`, `[attr*=v]`), pseudo-clases estructurales (`:first-child`, `:nth-child`, `:not`), combinadores (` `, `>`, `+`, `~`).

Propiedades: `color`, `background`, `font-{size,weight}`, `text-align`, `text-decoration`, `line-height`, `width`, `max-width`, `border` (shorthand + atómicos), `border-radius`, `box-shadow`, `list-style-type`, `:hover` con scope limitado.

## Deps

- `html5ever`, `markup5ever_rcdom`
- `ureq`, `url`
- [`puriy-core`](../puriy-core/README.md)
