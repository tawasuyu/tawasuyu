# cosmos-canvas-llimphi

> Backend Llimphi (vello) para [cosmos](../README.md).

Convierte los `Vec<Shape>` de [`cosmos-render`](../cosmos-render/README.md) en operaciones `vello::Scene` adentro de un `View::paint_with(...)` Llimphi. Pan + zoom + rotación. Tracking del cursor sobre el cielo → tooltip con info del objeto bajo el puntero.

## Deps

- [`cosmos-render`](../cosmos-render/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/) (vello)
