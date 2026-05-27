# pluma-notebook-graph-llimphi

> Graph view of the [pluma](../README.md) notebook: cells as nodes, Bezier wires.

Visual alternative to the linear list of [`pluma-notebook-llimphi`](../pluma-notebook-llimphi/README.md): each cell is a node on a free canvas, connected by edges that reflect output→input flow. Useful for notebooks with many parallel cells or when logical order isn't linear. Uses the [`nodegraph`](../../../02_ruway/llimphi/widgets/nodegraph/README.md) widget.

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md), [`pluma-notebook-exec`](../pluma-notebook-exec/README.md)
- [`llimphi-widget-nodegraph`](../../../02_ruway/llimphi/widgets/nodegraph/README.md)
