# llimphi-ui

> `View<Msg>` retained-mode + Elm-arch de [llimphi](../README.md).

API pública del framework: `App { Model, Msg, init, update, view }`. Reactivo: `update` muta el `Model`, `view(&Model)` produce el árbol; el runtime difea contra el árbol anterior y aplica el mínimo. Hover/focus/click se traducen a `Msg`s tipados.

## Deps

- [`llimphi-hal`](../llimphi-hal/README.md), [`llimphi-raster`](../llimphi-raster/README.md), [`llimphi-layout`](../llimphi-layout/README.md), [`llimphi-text`](../llimphi-text/README.md), [`llimphi-theme`](../llimphi-theme/README.md)
