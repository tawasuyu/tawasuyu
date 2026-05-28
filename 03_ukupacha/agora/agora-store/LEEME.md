# agora-store

> Persistencia JSON atómica para el `TrustGraph` de [agora](../LEEME.md), con re-verificación al cargar.

`save(ruta, &grafo)` escribe un snapshot versionado de forma atómica (tmp + fsync + rename). `load(ruta)` lee a una estructura espejo privada y **reconstruye** el grafo invocando `add_attestation` por cada entrada — así las firmas se re-verifican al cargar. Un archivo manipulado es error de carga, no corrupción silenciosa: el contrato de que *"el grafo sólo guarda evidencia comprobable"* se extiende al disco.

Lo que este crate **no** persiste: claves privadas. La seed/Keypair nunca cruza la superficie serde aquí — eso vive en [`agora-keystore`](../agora-keystore/LEEME.md), donde se cifra con la passphrase del usuario.

## Deps

- [`agora-core`](../agora-core/LEEME.md), [`agora-graph`](../agora-graph/LEEME.md), `serde`, `serde_json`, `thiserror`
