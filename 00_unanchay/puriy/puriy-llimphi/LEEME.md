# puriy-llimphi

> Chrome + render Llimphi de [puriy](../README.md).

Lado gráfico: URL bar, tabs, scroll, links clickables. Convierte el `BoxTree` que produce [`puriy-engine`](../puriy-engine/README.md) en `View<Msg>` de Llimphi recursivamente. El engine corre en un worker thread; el `BoxTree` cruza al UI thread por `Handle::dispatch` (el `DomTree` con `Rc<Node>` queda en el worker — es `!Send`).

## Features

- Scroll vertical (wheel, PageUp/Dn, ArrowUp/Dn, Home/End).
- Links clickables (`<a href>` → `Msg::Navigate`).
- Address bar editable.
- Pestañas múltiples (`Ctrl+T/W/Tab`).
- Historial por pestaña (`Alt+←/→`, botones ◀ ▶ ⟳).
- Images PNG/JPEG.

## Deps

- [`puriy-engine`](../puriy-engine/README.md), [`puriy-core`](../puriy-core/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/), widgets `text-input`
