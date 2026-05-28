# agora-graph

> Grafo de confianza de [agora](../LEEME.md): atestaciones verificadas + corroboración + política negociada.

Un `TrustGraph` almacena identidades conocidas y atestaciones **verificadas**: `add_attestation` ejecuta `Attestation::verify` antes de aceptar, así que una firma rota no entra. Los duplicados se descartan en silencio — la convergencia es idempotente.

El grafo deliberadamente **no** emite veredicto. `corroboration(sujeto, predicado, valor)` devuelve evidencia cruda: atestadores distintos y si el sujeto se auto-atestó. `TrustPolicy { min_third_party, accept_self }` es el umbral *negociado* que cada lector adopta. Dos lectores con políticas distintas mirando el mismo grafo pueden discrepar legítimamente.

## Deps

- [`agora-core`](../agora-core/LEEME.md), `serde`
