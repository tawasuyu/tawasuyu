# brahman-card-wit

> **DORMIDO (2026-05-30).** Capa 3 de Brahman. Ver `/BRAHMAN.md`.

Parser opcional de contratos WIT (`.wit` texto → `Vec<card_core::WitInterface>`, uno por `world`),
sobre `wit-parser` (sin `wasm-tools`/`wit-component`).

## Estado: relegado, no borrado

La visión original de Brahman incluía **módulos agnósticos descritos por interfaz WIT** (eventualmente WASM).
Esa capa **nunca se ejecutó**:

- No existe ningún archivo `.wit` en el workspace.
- Ningún crate de producción depende de este crate — sólo `examples/brahman-wit-info.rs` y la
  **dev-dependency** de `card-sidecar`.

Se conserva (funciona, 210 LOC, reversible) por si algún día aparecen `.wit` reales. **No asumir que está
en ninguna ruta de build.**

## El contrato agnóstico real y vigente

```
shared/card (formato Card)  +  card-handshake (handshake nativo Rust)  +  DhtKey (namespacing en la DHT)
```

El tipo de metadata `WitInterface` vive en **`card-core`** (no aquí) y **sí** lo usa el broker
(`chasqui-broker`) para matching estructural — es metadata opcional viva. Lo único dormido es *este parser*
de archivos `.wit` inexistentes.

## Si se decide revivir WIT

Requeriría: (1) que el Init lea `<modulo>/wit/protocol.wit` en el descubrimiento y construya
`ResolvedCard::from_conscious(card, wit)`; (2) que algún módulo de producción publique un `.wit`.
Hasta entonces, este crate es tooling latente (`cargo run -p card-wit --example brahman-wit-info -- <archivo.wit>`).
