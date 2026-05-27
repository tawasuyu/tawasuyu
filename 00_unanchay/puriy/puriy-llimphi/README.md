# puriy-llimphi

> Chrome + Llimphi render of [puriy](../README.md).

Graphical side: URL bar, tabs, scroll, clickable links. Recursively converts the `BoxTree` from [`puriy-engine`](../puriy-engine/README.md) into Llimphi `View<Msg>`. The engine runs in a worker thread; the `BoxTree` crosses to the UI thread via `Handle::dispatch` (the `DomTree` with `Rc<Node>` stays in the worker — it's `!Send`).

## Features

- Vertical scroll (wheel, PageUp/Dn, ArrowUp/Dn, Home/End).
- Clickable links (`<a href>` → `Msg::Navigate`).
- Editable address bar.
- Multiple tabs (`Ctrl+T/W/Tab`).
- Per-tab history (`Alt+←/→`, ◀ ▶ ⟳ buttons).
- PNG/JPEG images.

## Deps

- [`puriy-engine`](../puriy-engine/README.md), [`puriy-core`](../puriy-core/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/), widgets `text-input`
