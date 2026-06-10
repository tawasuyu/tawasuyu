# khipu

> `khipu` (Quechua: knotted cord recordkeeping). Notes with temporal gravity.

Quick-note capture where forgetting is part of the model: each note has a mass that decays with time and reinforces with each access. What's recurrent stays visible; what isn't touched fades until it falls off the horizon.

## Install

```sh
cargo run --release -p khipu-app
```

## Compatibility

- **Linux / macOS / Windows** — Llimphi UI (Wayland/X11/Win32 via `winit`).
- Local persistence in `$XDG_DATA_HOME/khipu/`.

## Crates

| Crate | Role |
|---|---|
| [`khipu-core`](khipu-core/README.md) | Note model + store; no UI. |
| [`khipu-gravity`](khipu-gravity/README.md) | Mass/decay algorithm; reinforcement on access. |
| `khipu-share` | Note envelopes, Ed25519-signed and content-addressed (BLAKE3) over agora; TCP/LAN transport + UDP discovery + encrypted identity. |
| `khipu-brahman` | Envelope transport over libp2p (BrahmanNet): encrypted stream + DHT discovery. |
| [`khipu-app`](khipu-app/README.md) | Llimphi UI over the core: the mind map is the interface. |

## The map is the interface (mind map)

The canvas of thoughts fills the whole window; list and editor are overlays that appear only when needed (a «☰ notes» drawer on the left, a floating editor on the right; `Esc` closes in cascade: naming → editor → drawer → focus). What makes it inhabitable:

- **Persisted anchor**: each note gets its home once (`Note.pos`), at the barycenter of its semantic kin (cosine affinity) with minimum separation — clusters emerge and nothing rearranges itself, so spatial memory can remember where things live. Pan/zoom camera over an infinite canvas.
- **The map breathes**: rendering uses live mass (`khipu-gravity`) for size and brightness — the recent burns, the abandoned fades. Selecting a note lights up filaments by diffusion activation.
- **Semantic zoom**: up close (zoom ≥ 1.6) a node stops being a dot and opens *in its place on the map* as an editable card that travels with pan/zoom; far away, the editor falls back to the side panel.
- **Emergent regions**: when a dense cluster gathers ≥3 visible notes with no toponym nearby, the map offers a «✛ name zone» chip at its centroid; the name stays as a faint landmark behind the nodes (belonging by neighborhood, not folders). You name *after* seeing the pattern.

## Semantic gravity (embeddings)

The map groups notes by affinity. Vectors come from the `verbo-daemon` if it's running; otherwise from a local hash-trigram embedder.

```sh
# Real embeddings (true semantic clusters and neighbors):
cargo run -p rimay-verbo-daemon-bin -- --provider fastembed   # listens on $XDG_RUNTIME_DIR/verbo.sock
cargo run --release -p khipu-app                              # auto-detects it on startup
```

Without the daemon, khipu falls back to the 16d trigram embedder — deterministic and offline. The computation never blocks the UI: it travels to a worker and re-enters the loop when done. If the vector space changes between two launches (daemon came up/down, different model), vectors are recomputed automatically.

## Sharing (agora)

`export` seals into `compartido.khipu` an Ed25519-signed envelope with the notebook's identity, addressed by its BLAKE3 content hash.

The identity lives **encrypted** (Argon2id + ChaCha20-Poly1305, via `agora-keystore`) in `<data>/keys/` — the private seed never sits in cleartext on disk. On the first share attempt khipu asks for a passphrase: created the first time, used to decrypt afterwards. `KHIPU_PASSPHRASE` in the environment unlocks without a prompt (headless). A cleartext `identidad.seed` from old versions is migrated into the keystore (and the cleartext deleted) automatically. It shares **whatever the search box is filtering** (empty = the whole notebook). `import` verifies signature + hash and, if they match, ingests the notes, tagging them with a provenance label `de:<author>`.

What travels is the **content** (title, body, tags), never the temporal physics: imported notes are born fresh (full mass, access = now) — their gravity starts in the receiving notebook. Wiki-links `[[Title]]` re-resolve by title. Re-importing the same envelope doesn't duplicate. A tampered envelope or a foreign signature is rejected whole, with no central authority.

For live sharing without copying files: `publicar` starts a TCP server that serves the notebook (port `KHIPU_BIND`, default `127.0.0.1:7700`) **and announces a UDP beacon** for LAN discovery. `recibir` opens a panel with an editable **address field** (`host:port`) and, below, the **peers discovered on the LAN** (name · author · address): click one to pull its notebook, or type an address. The transport is pure `std::net` and **doesn't need to be trusted** — the receiver verifies signature + hash before ingesting; the beacon only says *where*, not *what*.

**WAN / libp2p**: the address field of `recibir` accepts two auto-detected forms: `host:port` (direct TCP) or a libp2p **multiaddr** — direct `/ip4/…/p2p/<id>` or circuit `/ip4/…/p2p/<relay>/p2p-circuit/p2p/<id>`. On `publicar`, khipu also serves over libp2p (Noise-encrypted stream on `BrahmanNet`, protocol `/khipu/sobre/1.0.0`, via `khipu-brahman`) and shows your dial address.

**NAT traversal**: `BrahmanNet` ships **Circuit Relay v2 + DCUtR** (`card-net`). A reachable node acts as relay; one behind NAT reserves a circuit there and becomes reachable via the circuit address. Set `KHIPU_RELAY=/ip4/…/tcp/…/p2p/<relay-id>` before `publicar` and khipu reserves the circuit and shows the shareable address. External addresses aren't trusted blindly: **AutoNAT** confirms them via dial-backs from other peers, and only confirmed ones are announced.

**DHT discovery**: with `KHIPU_BOOTSTRAP=/ip4/…/p2p/<id>` (any node of the mesh), khipu joins the Kademlia DHT at startup; `publicar` announces itself under the khipu key and `recibir` lists — besides LAN peers — the peers found via DHT, pulled by peer-id. Two khipus find each other without exchanging IPs by hand, only sharing a common bootstrap. Verified end-to-end on localhost (4 tests in `khipu-brahman`).

The logic lives in `khipu-share`: `net` (TCP transport), `discovery` (UDP beacon) and `identity` (keystore). 19 tests + an integration test covering the full discover→pull→verify chain on loopback.

## Status (2026-06-10)

### Done

- `khipu-core` (note model + store) + `khipu-gravity` (mass/decay with reinforcement on access).
- `khipu-app`: Llimphi UI over the core, with main and contextual menus; split into modules (`main` / `map` / `panels` / `net`).
- Full mind-map redesign: canvas-as-root (list/editor float), persisted anchor with pan/zoom camera, live mass as size/brightness, in-place semantic zoom and nameable emergent regions.
- Semantic gravity: clustering via `verbo-daemon` embeddings (rimay), with offline 16d trigram fallback; computed in a worker that never blocks the UI.
- Sharing via agora (`khipu-share`): Ed25519-signed + BLAKE3-addressed envelopes, encrypted identity (Argon2id + ChaCha20-Poly1305 keystore), selective sharing + author provenance; TCP/LAN transport + UDP beacon discovery (19 tests + loopback integration).
- WAN/P2P (`khipu-brahman` over libp2p/BrahmanNet): Noise-encrypted stream, NAT traversal (Circuit Relay v2 + DCUtR), AutoNAT, Kademlia DHT discovery (4 e2e localhost tests).

### Pending

- Bidirectional sync / conflict resolution between notebooks (today import is unidirectional envelopes).
- Optionally transferring temporal physics when sharing (today content is born fresh at the receiver — a design decision, not a bug).
- Hardening the DHT mesh on real WAN (tested on localhost/LAN).

## Considerations

- **It's not a "todo" system** — no due dates, no reminders; it's a notebook with its own physics.
- Decay is transparent: each note shows its current mass; the user decides whether to save it.
- Plays well with the [agora](../../03_ukupacha/agora/README.md) network: notes can be shared without losing their local gravity.
