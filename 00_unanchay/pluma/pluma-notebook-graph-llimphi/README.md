# pluma-notebook-graph-llimphi

> Vista grafo del notebook de [pluma](../README.md): celdas como nodos, cables Bezier.

Alternativa visual a la lista lineal de [`pluma-notebook-llimphi`](../pluma-notebook-llimphi/README.md): cada celda es un nodo en un lienzo libre, conectado por aristas que reflejan flujo de outputs → inputs. Útil para notebooks con muchas celdas paralelas o cuando el orden lógico no es lineal. Usa el widget [`nodegraph`](../../../02_ruway/llimphi/widgets/nodegraph/README.md).

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md), [`pluma-notebook-exec`](../pluma-notebook-exec/README.md)
- [`llimphi-widget-nodegraph`](../../../02_ruway/llimphi/widgets/nodegraph/README.md)
