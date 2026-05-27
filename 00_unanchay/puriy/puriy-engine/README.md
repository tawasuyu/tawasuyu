# puriy-engine

> Engine of [puriy](../README.md): fetch + HTML/CSS parse + StyleEngine + box tree.

Non-graphical browser core. Pipeline: `fetch → parse_html → parse_styles → build_box_tree → BoxTree`. Uses `html5ever` (DOM tree), `markup5ever_rcdom` (Rc-DOM), own CSS parser (subset documented in the [SDD](../SDD.md)), `ureq` for sync HTTP (no tokio). LRU 64 MB cache with `Cache-Control: max-age` TTL.

## CSS support

Selectors: tag, `.class`, `#id`, attributes (`[attr]`, `[attr=v]`, `[attr^=v]`, `[attr$=v]`, `[attr*=v]`), structural pseudo-classes (`:first-child`, `:nth-child`, `:not`), combinators (` `, `>`, `+`, `~`).

Properties: `color`, `background`, `font-{size,weight}`, `text-align`, `text-decoration`, `line-height`, `width`, `max-width`, `border` (shorthand + atomic), `border-radius`, `box-shadow`, `list-style-type`, `:hover` with limited scope.

## Deps

- `html5ever`, `markup5ever_rcdom`
- `ureq`, `url`
- [`puriy-core`](../puriy-core/README.md)
