# pluma-render-plan

> Plan de render del documento de [pluma](../README.md).

Convierte un `Documento` en una lista plana de `OperRender` (bloques por átomo: texto, tabla, código, embed). El editor visual consume el plan para pintar; el server consume el plan para emitir HTML/PDF sin tocar el modelo.

## API

```rust
use pluma_render_plan::plan;

let plan = plan(&doc);
for op in &plan.ops {
    /* dibujar / serializar */
}
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-cuerpo`](../pluma-cuerpo/README.md)
- `serde`, `uuid`
