<p align="center"><img src="docs/brand/chakana-512.png" alt="tawasuyu" width="116"></p>

# tawasuyu

**A vertical software suite, built from the metal up.**

*Leé esto en español: [LEEME.md](LEEME.md).*

tawasuyu is one person's answer to a simple question: *what would computing
look like if you owned every layer of it?* It is a single Rust workspace of
~520 crates that contains, among other things:

- an **operating system** that boots on bare metal, with no Linux underneath (*wawa*),
- a **2D + 3D GUI engine** with its own widgets, layout, text, GPU pipeline and a voxel renderer (*llimphi*),
- a **Wayland compositor and window manager** (*mirada*),
- a **web browser engine** (*puriy*),
- a **writing environment** where one document lives as many parallel bodies (*pluma*),
- an **ERP**, an **astronomy engine**, a **physics DSL**, a **P2P note system**,
  an **image editor**, a **music engine**, a **native mail client**, a
  **terminal**… and the glue that makes them one coherent system instead of a
  pile of programs.

Everything is built on the same native foundations — content-addressed
storage (BLAKE3 + DAG + postcard), Ed25519 identity, a P2P layer — and
foreign formats (psd, xlsx…) enter only through explicit bridges. No
Electron, no web stack in the desktop apps, no inherited UI toolkit.

It is a working system in motion, not a polished product. The code is the
documentation of an architecture; this README is the front door.

## Try it in five minutes

You need stable Rust (plus nightly only for the bare-metal OS). Then:

```bash
git clone https://git.tawasuyu.net/tawasuyu/tawasuyu.git
cd tawasuyu
cargo check --workspace   # the suite's minimal smoke test
```

Pick something to run:

| You want to see… | Run |
|---|---|
| The GUI engine's widget gallery | `cargo run -p llimphi-gallery --release` |
| A voxel world creator (edit worlds/characters/scenes, AI assist, export to video) | `cargo run -p llimphi-voxel-studio --release` |
| A fast file-tree + text editor | `cargo run -p nada --release` |
| One document as many parallel bodies (translation/tone/summary) | `cargo run -p pluma-editor-llimphi --example multilienzo_completo_demo --release` |
| The astronomy/astrology workbench | `cargo run -p cosmos-app-llimphi --release` |
| Particle physics from a DSL | `cargo run -p tinkuy-llimphi --example tinkuy_demo --release` |
| A layered, non-destructive image editor | `cargo run -p tullpu-app-llimphi --release` |
| The terminal / workspace shell | `cargo run -p shuma-shell-llimphi --release` |
| A native mail client (IMAP/SMTP, semantic search, signed P2P "rail") | `cargo run -p paloma-app --release` |
| A process manager (Linux units, live controls) | `SANDOKAN_MONITOR_SEED=1 cargo run -p sandokan-monitor-llimphi --release` |
| A desktop launcher (bars, dock, global menu) | `cargo run -p launcher-llimphi --example launcher_demo` |
| **The operating system booting in QEMU** | `cd 03_ukupacha/wawa && cargo +nightly run -p boot -Z bindeps` |

Many crates ship more `examples/*_demo.rs` — they are the intended way to
try a feature without standing up the whole suite.

No toolchain at all? There is a **prebuilt wawa demo image** (~1.3 MB):
download [wawa-latest.tar.zst](https://tawasuyu.net/dist/wawa-latest.tar.zst),
extract, `./correr.sh` — the OS boots in QEMU in under a minute (needs
`qemu-system-x86_64` + OVMF).

## The map

The filesystem *is* the architecture. The workspace is organized as four
quadrants that mirror the four phases of the information cycle:

```
tawasuyu/
├── 00_unanchay/   PERCEIVE — pluma · khipu · rimay · chaka · pineal · puriy
├── 01_yachay/     KNOW     — cosmos · dominium · nakui · iniy · tinkuy
├── 02_ruway/      DO       — mirada · shuma · nahual · chasqui · takiy · llimphi · paloma
│                             supay · media · nada · tullpu · churay · hapiy · cards · wawa (host)
├── 03_ukupacha/   ROOT     — arje · wawa (kernel + WASM apps) · agora · minga
│                             sandokan · wawa-explorer
├── shared/        cross-cutting cores — sandokan · format · card · auth · ssh
│                             foreign-psd · rimay-localize · app-bus · launcher
└── web/           the landing you may have arrived from (not a product)
```

Moving a domain between quadrants changes its nature — these are not
administrative folders. A quick who-is-who:

- **pluma** — living documents: one material, many bodies (language, tone,
  audience), aligned paragraph-by-paragraph; plus a reactive notebook.
- **khipu** — notes that fade unless attention keeps them alive; P2P, local-first.
- **rimay** — language: embeddings daemon, localization.
- **puriy** — a web browser engine (CSS/layout/JS via QuickJS).
- **cosmos** — astronomy + astrology: ephemeris, sky-watching, tides, charts,
  and a 3D celestial sphere running on llimphi's GPU engine.
- **dominium** — simulation; **tinkuy** — physics DSL; **nakui** — an ERP;
  **iniy** — claim verification.
- **llimphi** — the GUI engine everything graphical is built on
  (`wgpu` + `vello` + `taffy` + `parley`, Elm loop, ~44 widgets).
- **mirada** — Wayland compositor / window manager / display manager.
- **shuma** — terminal and workspace runtime; **nada** — a fast editor;
  **nahual** — universal viewers; **tullpu** — image editor; **takiy** — music;
  **media** — audio/video; **supay** — a retro 3D engine; **chasqui** — message broker.
- **paloma** — native mail: IMAP/SMTP, semantic search, LLM-native
  summarize/draft (local-first), Ed25519-signed messages, and a sovereign P2P
  *rail* where the address *is* the public key — no From-spoofing.
- **churay** — an Office-style graphical installer/updater for the suite on any
  Linux (app catalog, one-click install, `.desktop` entries); shares a
  content-addressed hash format with **hammer**.
- **hapiy** — screen capture (the "Spectacle"): a sovereign `zwlr_screencopy`
  client that catches what mirada paints and hands the shot to tullpu to annotate.
- **arje** — init; **agora** — identity and Ed25519 signatures end-to-end;
  **minga** — P2P collaboration; **sandokan** — the control plane (who starts,
  stops, supervises and observes units on Linux and on wawa).
- **wawa** — the operating system: a SASOS kernel for `x86_64-unknown-none`,
  cooperative reactor, WASM apps isolated by capability bits, content-addressed
  storage, and its own network protocol (*akasha*) on a raw EtherType — no TCP/IP.

Each domain folder has its own `README.md` (English) and `LEEME.md`
(Spanish); complex domains also carry an `SDD.md` — the authoritative design
document. These same files are what [tawasuyu.net](https://tawasuyu.net)
serves.

## Architecture, briefly

Five rules shape everything:

1. **One domain = one root crate with plugin subcrates.** No lateral
   proliferation; crates split past ~1,500–2,000 LOC.
2. **UIs are interchangeable frontends over UI-agnostic `*-core` crates.**
   Domain logic never knows who paints it.
3. **All graphics go through llimphi.** One engine, one Elm-style loop
   (`input → update → view → layout → raster → present`), shared widgets,
   shared theme.
4. **Foreign formats enter through `shared/foreign-*` bridges**, never into
   an app's core. Apps work in the native format: BLAKE3-addressed DAGs
   serialized with postcard.
5. **`cargo check --workspace` must always pass on `main`.** CI guards it.

Types that cross boundaries — over the *akasha* network, into
content-addressed disk, or between kernel and userspace — live in `no_std`
crates (`format`, `akasha`, `mirada-layout`, `forth-emisor`,
`pluma-notebook-core`), validated by `./scripts/check-shared-cores.sh`.

## Building the unusual parts

The workspace builds with stable Rust. Two pieces are special:

**The OS (`03_ukupacha/wawa`)** is excluded from the root workspace: the
kernel targets `x86_64-unknown-none` with `panic = "abort"`. It needs
nightly with `rust-src`, plus the `wasm32-unknown-unknown` and
`x86_64-unknown-none` targets:

```bash
cd 03_ukupacha/wawa/wawa-kernel
cargo +nightly check --target x86_64-unknown-none -Z build-std=core,alloc

cd 03_ukupacha/wawa
cargo +nightly run -p boot -Z bindeps      # forge UEFI image and boot QEMU
./scripts/build-wawa-image.sh              # publishable QEMU/USB image
```

**The web landing (`web/tawasuyu-web`)** is the only JS-bridge crossing in
the repo (wasm-bindgen):

```bash
./scripts/build-tawasuyu-web.sh dev        # or `release`
```

## Status

Active personal research system, moving fast, with honest rough edges.
The kernel boots end-to-end in QEMU; the compositor runs real sessions on
Intel GPUs; most apps are usable MVPs ("ugly but working" is a design
stance here, not an accident). Standalone extracts of some domains are
published as front-door repos: [llimphi](https://git.tawasuyu.net/tawasuyu/llimphi),
[mirada](https://git.tawasuyu.net/tawasuyu/mirada), and others.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Two things to know up front:
commit messages are written in **Spanish** (repo convention), and names
with strong semantic load (quechua/Spanish: *khipu*, *rimay*, *wawa*…)
are never translated.

## License

Triple-licensed by area, see [LICENSE.md](LICENSE.md): the workspace
default is **MIT OR Apache-2.0**; six foundational crates (`format`,
`forth-emisor`, `foreign-fs`, `wawa`, `wawa-kernel`, `wawa-fs`) are
**MPL-2.0**.

## Links

- **Site:** [tawasuyu.net](https://tawasuyu.net) — serves these very documents.
- **Source:** [git.tawasuyu.net/tawasuyu/tawasuyu](https://git.tawasuyu.net/tawasuyu/tawasuyu)
- **Plan & design:** [PLAN.md](PLAN.md), [WAWA.md](WAWA.md), per-domain `SDD.md`.
