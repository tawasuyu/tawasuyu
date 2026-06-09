# cards

**One way to read every kind of Card.**

*Leé esto en español: [LEEME.md](LEEME.md).*

Across tawasuyu, several domains describe "things that run or group
things" as JSON documents called **Cards**: runtime entities, semantic
groupings, UI modules. Each format was born in its own domain with its own
schema. This crate is the unifying arm: it reads any of them and projects
them onto **one canonical `Card` structure** that UI, storage, DHT and wire
code can consume without caring where the document came from.

## How it works

`load_card(path)` inspects the JSON's *shape* — no flags, no extension
magic — and dispatches to the right reader:

- an **Ente** has `payload` + `supervision` → read via `card-core`
  (`shared/card`), the runtime-entity schema;
- a **Monad** has `members` + `cardinality` → read via `chasqui-card`,
  the semantic-grouping schema;
- a **UiModule** has `entities` + `views` + `menu` → read via
  `nahual-meta-schema`, the UI-module schema.

Each format keeps living in its origin crate with its own schema; the
readers only deserialize and wrap. The canonical `Card` carries the shared
projection: an opaque string `id`, `schema_version`, a derived `label`,
optional `lineage`, and an `extensions` map for forward compatibility.

`load_cards_from_dir(dir)` walks the immediate subdirectories of a root,
loading the conventional card file of each (`card.ncl` preferred over
`card.json`; subdirs without one are skipped silently, real errors are
loud and stop the walk).

## Nickel templates (V2)

Besides plain JSON, Cards can be written in
[Nickel](https://nickel-lang.org/) (`card.ncl`): templates with defaults
merged via Nickel's native `import`, evaluated at load time. V1 (JSON) and
V2 (Nickel) coexist.

## Try it

```bash
cargo test -p cards                      # full suite
cargo test -p cards -- --test-threads=1  # template tests mutate env → serial
```

The integration tests (`tests/integration.rs`) double as usage examples:
shape detection, dispatch, JSON→Card round-trips, directory walking,
reader restriction.

## Status

Working: JSON readers for the three formats, shape-based dispatch, Nickel
templates, directory loading. The set of readers grows as new Card formats
appear in the suite.
