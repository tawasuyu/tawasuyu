# brahman-card-wit

> **DORMANT (2026-05-30).** Layer 3 of Brahman. See `/BRAHMAN.md`.

Optional parser for WIT contracts (`.wit` text → `Vec<card_core::WitInterface>`, one per `world`),
over `wit-parser` (without `wasm-tools`/`wit-component`).

## Status: shelved, not deleted

Brahman's original vision included **agnostic modules described by a WIT interface** (eventually WASM).
That layer **never ran**:

- No `.wit` file exists in the workspace.
- No production crate depends on this crate — only `examples/brahman-wit-info.rs` and the
  **dev-dependency** of `card-sidecar`.

It is kept (it works, 210 LOC, reversible) in case real `.wit`s ever appear. **Don't assume it's
on any build path.**

## The real and current agnostic contract

```
shared/card (formato Card)  +  card-handshake (handshake nativo Rust)  +  DhtKey (namespacing en la DHT)
```

The `WitInterface` metadata type lives in **`card-core`** (not here) and **is** used by the broker
(`chasqui-broker`) for structural matching — it's live optional metadata. The only dormant thing is *this parser*
of nonexistent `.wit` files.

## If WIT is to be revived

It would require: (1) the Init reading `<modulo>/wit/protocol.wit` at discovery and building
`ResolvedCard::from_conscious(card, wit)`; (2) some production module publishing a `.wit`.
Until then, this crate is latent tooling (`cargo run -p card-wit --example brahman-wit-info -- <archivo.wit>`).
