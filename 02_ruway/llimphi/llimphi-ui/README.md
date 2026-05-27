# llimphi-ui

> Retained-mode `View<Msg>` + Elm-arch of [llimphi](../README.md).

Public API of the framework: `App { Model, Msg, init, update, view }`. Reactive: `update` mutates `Model`, `view(&Model)` produces the tree; the runtime diffs against the previous tree and applies the minimum. Hover/focus/click translate to typed `Msg`s.

## Deps

- [`llimphi-hal`](../llimphi-hal/README.md), [`llimphi-raster`](../llimphi-raster/README.md), [`llimphi-layout`](../llimphi-layout/README.md), [`llimphi-text`](../llimphi-text/README.md), [`llimphi-theme`](../llimphi-theme/README.md)
