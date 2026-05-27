# nakui-explorer-llimphi

> Token-graph explorer for [nakui](../README.md).

"DAG" view — each cell/token is a node; formula dependencies are the edges. Force-directed layout with `petgraph` + Fruchterman-Reingold; zoom and pan; click on a node opens detail (formula, value, history via time-travel). Useful for audit — understand why A1 depends on Z99.

## Deps

- [`nakui-core`](../nakui-core/README.md)
- [`llimphi-widget-nodegraph`](../../../02_ruway/llimphi/widgets/nodegraph/README.md)
- `petgraph`
