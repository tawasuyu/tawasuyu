# agora

> Public square. Forum, conversation, deliberation with minimal identity.

`agora` is where monorepo nodes talk openly. Gossip protocol over `chasqui` for distribution; ed25519-pubkey identity; thread graph persisted locally. Designed to survive without a central server, without corporate accounts and without algorithmic moderation.

## Install

```sh
cargo run --release -p agora-app
```

## Compatibility

- **Linux / macOS / Windows / Wawa** — all crates are pure Rust.
- Local persistence; optional sync with peers from the `minga` network.

## Crates

| Crate | Role |
|---|---|
| [`agora-core`](agora-core/README.md) | Model: thread, message, author, signature. |
| [`agora-graph`](agora-graph/README.md) | Thread graph + relations. |
| [`agora-store`](agora-store/README.md) | Local persistence. |
| [`agora-gossip`](agora-gossip/README.md) | Gossip protocol over chasqui. |
| [`agora-app`](agora-app/README.md) | Llimphi UI. |

## Considerations

- **Pubkey identity**, not email or phone.
- No algorithmic feed: order is chronological or by thread; the user decides what to follow.
- Plays well with `minga`: agora runs over `minga`'s peer network when both are active.
