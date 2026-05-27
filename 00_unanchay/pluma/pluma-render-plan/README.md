# pluma-render-plan

> Document render plan for [pluma](../README.md).

Converts a `Documento` to a flat `OperRender` list (atom blocks: text, table, code, embed). The visual editor consumes the plan to paint; the server consumes the plan to emit HTML/PDF without touching the model.

## API

```rust
use pluma_render_plan::plan;

let plan = plan(&doc);
for op in &plan.ops {
    /* draw / serialize */
}
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-cuerpo`](../pluma-cuerpo/README.md)
- `serde`, `uuid`
