# dominium-canvas-llimphi

> Backend Llimphi (vello) para [dominium](../README.md).

Convierte el `Vec<Quad>` que produce [`dominium-render-plan`](../dominium-render-plan/README.md) en operaciones `vello::Scene` adentro de un `View::paint_with(...)` de Llimphi. Single-pass; cero allocs por frame (re-usa el buffer de quads).

## Deps

- [`dominium-render-plan`](../dominium-render-plan/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/) (vello)
