# wawa (root / kernel)

> Operating system from scratch. Kernel + boot + filesystem + apps.

## Try it in one minute — no toolchain

![wawa booted in QEMU: the genesis desktop — bitacora open, the app taskbar, RAM meter and clock, all painted by the kernel](https://tawasuyu.net/03_ukupacha/wawa/pantallazo.png)

Download the prebuilt demo image (~1.3 MB) and boot it in QEMU:

```sh
curl -LO https://tawasuyu.net/dist/wawa-latest.tar.zst
tar --zstd -xf wawa-latest.tar.zst && cd wawa-*/
./correr.sh        # needs qemu-system-x86_64 + OVMF (edk2-ovmf / ovmf package)
```

The launcher finds the OVMF firmware by itself (override with
`WAWA_OVMF=/path/OVMF.fd`); extra args are forwarded to QEMU. The package
ships `wawa.img` (UEFI-bootable as-is — `dd` it to a USB stick for real
metal), `SHA256SUMS`, and a `LEEME.md` with what's inside: the full genesis
userspace (pluma, ayni, bitacora, asistente, rimay, testigo…) embedded as a
ramdisk, so the session is ephemeral by design. Experimental software:
prefer the VM; flash real hardware at your own risk.

`wawa` (Quechua: *new creature, baby*) is the kernel side of the pair; userspace lives in `02_ruway/wawa/`. Filesystem is a **content-addressed DAG** over BLAKE3; apps are WASM modules isolated by `wasmi`, with capabilities gated at the linker; cooperative frame pacing; the host-side forge (`boot`) + AoE (network) + atlas (Fontdue) materialize the DAG. Wawa **never speaks NTFS/Ext4 directly** — everything enters the graph as BLAKE3-addressed objects (forge or AoE).

Gaming-grade north star: AOT WASM + GPU passthrough + BLAKE3 asset streaming (today: `wasmi` + cooperative frame pacing — see Status).

## Install

Toolchain: nightly with `rust-src`, targets `wasm32-unknown-unknown` and `x86_64-unknown-none`.

```sh
cd 03_ukupacha/wawa
cargo +nightly run -p boot -Z bindeps        # forges the UEFI image and boots it in QEMU

cd 03_ukupacha/wawa/wawa-kernel
cargo +nightly check --target x86_64-unknown-none -Z build-std=core,alloc   # kernel alone

./scripts/build-wawa-image.sh                # publishable QEMU/USB image: apps → image → dist/ (tar.zst)
```

## Compatibility

- **x86_64** — only target (`x86_64-unknown-none`).
- **UEFI-only** boot (`bootloader::UefiBoot`); no BIOS path.
- Userspace counterpart: `02_ruway/wawa/`.

## Crates: kernel

| Crate | Role |
|---|---|
| [`wawa-kernel`](wawa-kernel/README.md) | Kernel (scheduler, syscalls, capabilities). |
| [`wawa-boot`](wawa-boot/README.md) | Host-side forge: builds the kernel as artifact, seeds the genesis graph, emits the UEFI image (package `boot`). |
| [`wawa-fs`](wawa-fs/README.md) | Akasha-over-Ether network protocol (the crate is named `akasha`; the directory keeps its historical name). |

## Crates: apps (WASM in the kernel)

| App | Function |
|---|---|
| `asistente` | Conversational assistant; human-signature cycle over proposals (the machine proposes, the human signs). |
| `ayni` | P2P chat signed with Ed25519 over akasha, serverless. |
| [`pluma`](apps/pluma/README.md) | Markdown viewer inside the kernel. |
| [`bitacora`](apps/bitacora/README.md) | System logbook. |
| [`cronista`](apps/cronista/README.md) | Historical logger. |
| [`discola`](apps/discola/README.md) | Disk/media. |
| [`glotona`](apps/glotona/README.md) | Eats heavy tasks (batch). |
| [`hello_wasm`](apps/hello_wasm/README.md) | WASM hello-world. |
| [`memoriosa`](apps/memoriosa/README.md) | Persistent user memory. |
| [`mudanza`](apps/mudanza/README.md) | Migrate between snapshots. |
| [`pregon`](apps/pregon/README.md) | System announcements. |
| [`pulso`](apps/pulso/README.md) | Heartbeat/health checks. |
| [`rimay`](apps/rimay/README.md) | Bare-metal embeddings verb. |
| `testigo` | Witness of the `tinkuy` engine (exercises the `sys_tinkuy_*` capabilities). |
| `tinkuy` | DOD particle engine as a WASM module. |
| [`tonada`](apps/tonada/README.md) | Tonada → audio player. |
| [`tonalero`](apps/tonalero/README.md) | Tonada producer. |

## Considerations

- **WASM-first**: apps aren't native processes; they're WASM modules with explicit capabilities.
- **Byte immutability**: once the hash is computed, those bytes don't change; "edits" are new hashes.
- **Zero direct NTFS/Ext4**: the host-side forge seeds the graph; Wawa reads the DAG.
- **Cooperative pacing**: no app can hog the frame; the scheduler asks it to yield.

## Status (2026-06-09)

> Authoritative source for kernel status: [WAWA.md](../../WAWA.md) (§0–§14) and [SDD-capacidades.md](SDD-capacidades.md). Excluded from the root workspace: compiles `x86_64-unknown-none` with `panic = "abort"`.

### Done (subsystems on disk)
- **Boot + graphics**: **UEFI-only** via `wawa-boot` (`bootloader::UefiBoot`; consumes the kernel as `artifact = "bin"`), GOP framebuffer, compositor (refactored from 1980 LOC into a `compositor/` directory), console/text. **Boots end-to-end in QEMU** (kernel stack at 1 MiB: mounting virtio-sound overflowed the bootloader's default 80 KiB).
- **Multi-monitor** (Phase 64): native multi-scanout virtio-gpu driver + per-output render, gated by `RENASER_MONITORES`; Alt+O moves the focused window to the next monitor; `gop-probe` to probe GOP on metal.
- **`pata` desktop frame** (Phase 9): the kernel paints the frame (`compositor/pata_marco.rs`) — dynamic reserved strip, start button that opens the launcher, workspace switcher, bidirectional config over akasha, coverage of `WidgetView::TextRich` and `Moon`.
- **Publishable image**: `scripts/build-wawa-image.sh` chains apps → self-contained UEFI image (genesis graph as ramdisk) → `dist/` with `wawa.img` + portable QEMU launcher + SHA256SUMS + `.tar.zst` tarball.
- **Cooperative reactor** (`async_system/`): PIT 100Hz + IRQs, task executor; cooperative frame pacing.
- **Content-addressed storage** (`almacen.rs`): BLAKE3 + log + semantic GC mark/sweep/swap; configuration (language+theme) as a graph node injected into `ContextoCapacidades`.
- **WASM apps** (`wasm/`): isolated by `wasmi`, fuel/tick + memory ceiling, capabilities gated at the linker (physical boundary, not a table). 17 apps in `apps/` (asistente, ayni, bitacora, cronista, discola, glotona, hello_wasm, memoriosa, mudanza, pluma, pregon, pulso, rimay, testigo, tinkuy, tonada, tonalero).
- **Signed capabilities §14.1.3** (code-complete wiring): grant signed by bytecode hash (`claves::verificar_concesion_capacidad`), enforcement by live intersection (`permisos_efectivos_de`), genesis seam at boot that anchors offline grants, revocation wired to the agora overlay, automated ceremony (`scripts/wawa-conceder-genesis.sh`), and the strict flip as a named toggle `MODO_CAPACIDAD_ESTRICTO_GLOBAL`.
- **`akasha` network** (own EtherType, no TCP/IP) + Akasha-over-Ether bridge via TAP transport host↔guest; recursive download of the delta DAG (an announcement drags the channel→manifest→bytecodes cone); Accept/Reject loop of `mudanza`.
- **USB/HID**: initialization of all XHCI controllers, Port Power, native USB HID mouse (boot protocol).
- **Ed25519 signatures** in the kernel (`claves.rs`, zero-alloc `ed25519-compact`): `verificar_manifiesto_firmado` / `verificar_anuncio_canal` / `verificar_cuaderno_firmado` / `verificar_revocacion`.

### Pending
- Flip `MODO_CAPACIDAD_ESTRICTO_GLOBAL` to `true` after seeding the genesis grants (today `false`: without a grant the permissions declared in `EntradaApp` rule) — enforcement and the ceremony already exist (WAWA.md §14.1.3).
- Pending gaming optimizations: AOT WASM (cranelift) over the execution path, GPU passthrough, BLAKE3 asset streaming (today render is the base path).
- No automatic app restart (oneshot by design); Wawa's "process monitor" (executor census + compositor beacons) still to be built.
- The path on real metal hardware (beyond QEMU) keeps hardening (XHCI/GOP). No aarch64 port: the sole target is `x86_64-unknown-none`.
