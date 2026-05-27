# 03 ukupacha · root

`ukupacha` (Quechua: *inner world, root, what is underground*). This is the **invisible infrastructure** quadrant: kernel, bootloader, filesystem, deep network protocols, the community that holds it all up. What no user sees directly but that decides whether the system boots or not.

The quadrant's rule is **invariants before features**: in `ukupacha`, breaking changes cost migrations across the whole tree; that's why each decision is thought of as "in ten years, is this still true?". Change here is slow and deliberate.

## Applications

- **[agora](agora/README.md)** — public square. Forum, conversation, deliberation with minimal identity.
- **[arje](arje/README.md)** — bootloader and the system's early life. `arje-seeds` (seeds), `arje-packager` (packaging), `arje-installer` (install), `arje-absorb` (ingest an existing system).
- **[minga](minga/README.md)** — collaboration between nodes. Andean tradition of communal work, applied to the network.
- **[wawa](wawa/README.md)** — operating system from scratch (`wawa-kernel`, `wawa-boot`, `wawa-fs`, `apps/`). POSIX → BLAKE3 ingest; filesystem as content-addressed DAG; gaming-grade (AOT WASM + GPU passthrough + cooperative frame pacing).
- **[wawa-explorer](wawa-explorer/README.md)** — host-side viewer of Wawa's DAG: reads `.img`, speaks the Akasha protocol over raw sockets, shows the tree with detail in Llimphi.

## Manifesto

> **The root holds quiet.**
> What lasts is what doesn't call attention when it works. A good kernel is the one no one notices.
>
> 1. **No frivolous dependencies at the root.** Each `ukupacha` crate justifies every `Cargo.toml` line.
> 2. **Content-addressed by default.** BLAKE3 is identity — bytes are truth, names are hints.
> 3. **The user is not the kernel's client.** The kernel's client is the operator. User-friendly tools live in `02_ruway`.
> 4. **Document as if the next reader were an archaeologist twenty years from now.** The SDDs, the WHY, the written reasons — they are the only way for something to survive its author.
