# agora-app

> Llimphi UI of [agora](../README.md): identities, attestations, composer, policy.

Four draggable tiles over the same `TrustGraph`, swappable by title-bar drag:

- **Identities** — known + mine. New-identity button generates a CSPRNG seed, wraps it in [`agora-keystore`](../agora-keystore/README.md), and registers the public side in the graph.
- **Attestations** — the verified evidence pile. Filterable by subject / attester / predicate. Self-attestations marked distinctly.
- **Composer** — in-situ editor for `subject · predicate = value`, signed as the active local identity, added to the graph and persisted on commit.
- **Policy** — slider for `min_third_party`, toggle for `accept_self`, live verdict on the selected claim.

Persistence is automatic: every change goes through [`agora-store`](../agora-store/README.md); private keys live in `agora-keystore`, unlocked at startup with a passphrase.

## Usage

```sh
cargo run --release -p agora-app
```

## Deps

- All `agora-*` (core, graph, store, keystore)
- [`llimphi-ui`](../../../02_ruway/llimphi/llimphi-ui/) + Llimphi widgets
