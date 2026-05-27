# cosmos-canvas-llimphi

> Llimphi (vello) backend for [cosmos](../README.md).

Converts `Vec<Shape>` from [`cosmos-render`](../cosmos-render/README.md) into `vello::Scene` operations inside a Llimphi `View::paint_with(...)`. Pan + zoom + rotation. Cursor tracking over the sky → tooltip with info on the object under the pointer.

## Deps

- [`cosmos-render`](../cosmos-render/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/) (vello)
