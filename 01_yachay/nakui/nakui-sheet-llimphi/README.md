# nakui-sheet-llimphi

> Llimphi UI of the [nakui](../README.md) matrix view.

Virtualized grid (millions of cells without allocating anything off-screen), inline editing, formulas with autocompletion, range selection (Shift+click), arrow-key navigation, copy/paste with real clipboard, **freeze panes** (Ctrl+Shift+F anchors the rows above / columns left of the active cell; toggles off; also in the context menu), **pivot tables** (Ctrl+Shift+P over a selection: group rows by one column and aggregate another with SUM/COUNT/AVG/MIN/MAX; A/G/V/H cycle function/group/value/header, Esc closes). Reuses [`text-input`](../../../02_ruway/llimphi/widgets/text-input/README.md) for editing and [`text-editor`](../../../02_ruway/llimphi/widgets/text-editor/README.md) for long formulas.

## Deps

- [`nakui-sheet`](../nakui-sheet/README.md), [`nakui-sheet-nakuicore`](../nakui-sheet-nakuicore/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/) + widgets `text-input`, `text-editor`
