# nakui-explorer-llimphi

> Explorer del grafo de tokens de [nakui](../README.md).

Vista "DAG" — cada celda/token es un nodo; las dependencias de fórmula son las aristas. Layout fuerza-dirigida con `petgraph` + Fruchterman-Reingold; zoom y pan; click en un nodo abre detalle (fórmula, valor, historial vía time-travel). Útil para auditoría — entender por qué A1 depende de Z99.

## Deps

- [`nakui-core`](../nakui-core/README.md)
- [`llimphi-widget-nodegraph`](../../../02_ruway/llimphi/widgets/nodegraph/README.md)
- `petgraph`
