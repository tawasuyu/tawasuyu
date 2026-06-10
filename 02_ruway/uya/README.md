# uya — sovereign video call

**uya** is "face" in Quechua: a video call is, above all, seeing faces
live. It is the native app that replaces Zoom/Meet (the real-time
"application web"), on the same principle as the rest of the suite — an
agnostic `*-core` + interchangeable Llimphi frontends, with no browser or JIT
in the way (see `APPS-NATIVAS.md`, Batch 2).

## Crates

| Crate | Role |
|---|---|
| `uya-core` | Agnostic model: wire protocol (`Paquete`), deterministic identity (`ParticipanteId = BLAKE3(name)`) and the room roster (`Sala`). Knows nothing of transport or UI. |
| `uya-app` | Glue: sovereign P2P transport over **card-net** (`BrahmanNet`/libp2p, with relay/dcutr/autonat), video capture (synthetic TestCard by default; v4l2 webcam behind the `camara` feature), audio (mic→mix→`AudioSink`) and an event bus (`EventoUya`) toward the UI. |
| `uya-cli` | Headless node: exercises transport + capture and reports events to the console. No GPU. |
| `uya-llimphi` | Graphical face (Elm loop): grid of faces (one tile per participant, their latest RGBA frame via `View::image`) + camera/microphone/hang-up bar. |

## Trying it

The transport is libp2p: each node prints its **dialable multiaddr** on
startup (with `/p2p/<peerid>`). To call someone, pass them that address via
`UYA_CONECTAR`. Two windows:

```bash
# 1) Start Alicia and copy the dialable address it prints:
UYA_NOMBRE=Alicia UYA_ESCUCHAR=/ip4/0.0.0.0/tcp/7800 cargo run -p uya-llimphi --release
#    → "uya: dialable en /ip4/127.0.0.1/tcp/7800/p2p/12D3KooW..."

# 2) Connect Beto to that address:
UYA_NOMBRE=Beto UYA_ESCUCHAR=/ip4/0.0.0.0/tcp/7801 \
  UYA_CONECTAR=/ip4/127.0.0.1/tcp/7800/p2p/12D3KooW... cargo run -p uya-llimphi --release
```

Headless (no window / no GPU), same flow with `uya-cli` (reports received
frames and audio samples):

```bash
UYA_NOMBRE=Alicia UYA_ESCUCHAR=/ip4/0.0.0.0/tcp/7810 cargo run -p uya-cli
UYA_NOMBRE=Beto UYA_CONECTAR=<Alicia's dialable address> cargo run -p uya-cli
```

Variables: `UYA_NOMBRE` (→ identity), `UYA_ESCUCHAR` (listen multiaddr,
default `/ip4/0.0.0.0/tcp/0`), `UYA_CONECTAR` (dialable multiaddr(s),
comma-separated), `UYA_TONO=1` (synthetic tone if there is no microphone).

**Joining by room name** (instead of pasting addresses): everyone with the same
`UYA_SALA`, seeding the DHT with a rendezvous (`UYA_BOOTSTRAP`, e.g. the first
node). Anyone can dial anyone in the room; the mesh converges on its own.

```bash
# Host/rendezvous (note its dialable address):
UYA_NOMBRE=Ana UYA_SALA=oficina UYA_ESCUCHAR=/ip4/0.0.0.0/tcp/7880 cargo run -p uya-cli
# Everyone else: same room, bootstrap to the host, WITHOUT pasting addresses:
UYA_NOMBRE=Bea UYA_SALA=oficina UYA_BOOTSTRAP=<Ana's address> cargo run -p uya-cli
```

## Status (MVP)

Works today, end-to-end and ugly on purpose:

- ✅ **Sovereign P2P transport** over card-net (`BrahmanNet`/libp2p): Noise +
  Yamux + relay/dcutr/autonat — the same node as ayni/minga/agora, so it
  works across NAT, not just on the LAN. Streams multiplexed over `/uya/transporte/1.0.0`.
- ✅ **Deterministic name-based identity**: the app's `ParticipanteId` and the
  transport's ed25519 keypair both derive from `BLAKE3(name)`, so the
  **PeerId (and the dialable multiaddr) is stable across startups** — they share a root.
- ✅ Presence: join / leave / media state.
- ✅ **Two-way video** + local preview. Compressed with **per-frame JPEG**
  (MJPEG): ~40× fewer bytes than raw RGBA (192×144: 110 KB → ~2.8 KB),
  with no inter-frame state (low latency). The local preview goes uncompressed.
- ✅ **Two-way audio**, compressed with **Opus** (~57×: 20 ms = 3840 B PCM
  → ~67 B): microphone capture (`MicSource` at 48 kHz, or synthetic tone with
  `UYA_TONO=1`), downmix + resample to 48 kHz mono, Opus encode per 20 ms frame;
  on receive, an `OpusDecoder` per peer decodes to PCM and a `MezclaRemota`
  resamples to the device + sums the N peers, played back by `AudioSink` (cpal).
  **Adaptive jitter buffer**: bounded latency (~120 ms, drops bursts with
  smooth catch-up) + prebuffer (~40 ms, no click on startup / after underrun).
- ✅ **Group calls (automatic N-to-N mesh)**: by joining a single host node,
  everyone discovers and auto-connects. Each node gossips the dialable
  multiaddrs it knows (`Paquete::Pares`); the receiver dials the ones it is
  missing, with PeerId tie-break (only the lower one initiates) so as not to
  duplicate connections. Verified with 3 nodes: each one sees and receives video from the other two.
- ✅ Synthetic camera by default (TestCard); real v4l2 webcam with `--features camara` in `uya-app`.
- ✅ Camera / microphone toggle and hang-up.
- ✅ **Connect from the UI**: the app shows your dialable address (to
  share) and a field where you paste (Ctrl/Cmd+V) a peer's + Enter/button —
  `UYA_CONECTAR` is no longer needed. (Also works via env, as before.)
- ✅ **Join by room name (DHT)**: `UYA_SALA=<name>` announces itself as a
  provider of `BLAKE3("uya/sala/<name>")` in `BrahmanNet`'s Kademlia and
  discovers the other providers, which join the mesh on their own. Verified with
  3 nodes (with `UYA_BOOTSTRAP` to seed the DHT): all three see each other, without pasting
  addresses.
- ✅ **Zero-config on LAN (multicast beacon)**: in a room, uya emits a UDP
  multicast beacon `uya1\t<room>\t<port>\t<peerid>` (group 239.255.42.99:7799) and
  listens for others'; on receiving one from its room, it reconstructs the multiaddr using
  the **source IP of the datagram** (resolves the loopback case → works between
  machines) and dials. Room-aware, without `UYA_BOOTSTRAP` or `UYA_CONECTAR`. It joins and
  emits on **all IPv4 interfaces** (robust on desktops with wifi+eth+docker+
  VPN, where multicast on the default interface does not arrive). Verified: 2 nodes
  same room → they discover and connect on their own. (mDNS was also added to
  `shared/card/card-net` to populate the DHT, but uya's own beacon is the reliable
  path on LAN.)

## Pending (in order)

1. **agora signature of `Hola`**: the PeerId is already stable (derives from `BLAKE3(name)`),
   but the name is self-declared. Tie identity to `agora`: sign the `Hola`
   with the agora key and verify it, so no one can impersonate a name.
2. **Public bootstrap for WAN**: on LAN there is already beacon discovery;
   what's missing is a known default rendezvous to "join by name" across
   different networks (the multicast beacon does not cross routers).
3. **Acoustic echo cancellation (AEC)**: when the mic picks up the speaker.
   Needs a real AEC (and ears/hardware to evaluate it). The jitter buffer is already
   adaptive.
4. **SFU / selective forwarding** for large groups (today it's a full mesh: N² streams).
