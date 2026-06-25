# arje

> `arje` (Greek *ἀρχή*: origin, principle). Bootloader and the system's early life.

`arje` covers from "you pressed POWER" to "the kernel is running": seeds, packaging, installation, absorption of an existing system, its own kernel, minimal networking, rules + audit, CAS, snapshots, soma, WASM init.

## Install

```sh
# initramfs (cpio.gz) from a canonical seed + static binaries
cargo run --release -p arje-packager -- --seed 03_ukupacha/arje/seeds/arje-qemu.card.json --bin arje-zero=target/release/arje-zero --out /tmp/arje-qemu.cpio.gz

# install to an ESP partition (`to-partition`) or build a bootable USB (`to-usb`)
cargo run --release -p arje-installer -- to-partition --esp /boot --kernel bzImage --seed 03_ukupacha/arje/seeds/arje-host.card.json

# absorb an existing init's config (sysvinit/runit/dinit/OpenRC) into a Seed
cargo run --release -p arje-absorb -- --from auto --root / --output seed.card.json
```

## Compatibility

- **Linux x86_64** — primary target.
- **aarch64** — `arje-kernel` supports it (limited).
- **Wawa** — `arje` is the natural bootloader for `wawa-kernel`.

## Crates

| Crate | Role |
|---|---|
| [`arje-zero`](init/arje-zero/README.md) | Point zero: the first thing that runs (PID 1). |
| [`arje-loader`](init/arje-loader/README.md) | Own EFI bootloader (uefi-rs): loads the EFISTUB kernel + initramfs + cmdline. |
| [`arje-kernel`](init/arje-kernel/README.md) | arje's minimal kernel (separate from wawa-kernel). |
| [`arje-incarnate`](init/arje-incarnate/README.md) | Materializes processes. |
| [`arje-bus`](runtime/arje-bus/README.md) | Internal bus (arje IPC, Unix socket + postcard): the init's control wire. |
| [`arje-soma`](init/arje-soma/README.md) | The system's "body" at runtime. |
| [`arje-cas`](runtime/arje-cas/README.md) | Content-addressed store. |
| [`arje-snapshot`](init/arje-snapshot/README.md) | System snapshot. |
| [`arje-echo`](runtime/arje-echo/README.md) | Early logging. |
| [`arje-net-bring-up`](init/arje-net-bring-up/README.md) | Minimal network bring-up. |
| [`arje-wasm`](runtime/arje-wasm/README.md) | Init's WASM runtime. |
| [`arje-compat`](arje-compat/README.md) | POSIX userspace compat (shims). |
| [`arje-getty-stub`](init/arje-getty-stub/README.md) | Minimal login. |
| [`arje-card`](arje-card/README.md) | Historical alias of `card-core` (re-exports `EntityCard ≡ Card`); not UI. |
| [`arje-card-llimphi`](arje-card-llimphi/) | Desktop card (arje's state): init isolation capabilities, on Llimphi. |
| [`arje-packager`](init/arje-packager/README.md) | Packager (initramfs cpio.gz from a Seed Card). |
| [`arje-installer`](init/arje-installer/README.md) | Installer (ESP partition / bootable USB). |
| [`arje-absorb`](init/arje-absorb/README.md) | Absorbs an existing init's config → arje Seed. |
| [`arje-brain`](runtime/arje-brain/README.md) | Rules + audit of the init. |
| [`arje-brain-rules`](runtime/arje-brain-rules/README.md) | Declarative rules. |
| [`arje-brain-cognitive`](runtime/arje-brain-cognitive/README.md) | Reasoner. |
| [`arje-brain-audit`](runtime/arje-brain-audit/README.md) | Reasoner's audit. |

Canonical seeds (`arje-host`, `arje-qemu`) live in [`seeds/`](seeds/).

## Considerations

- **`arje` is for boot, not for governing the running system**. That's the `wawa-kernel`'s job.
- `absorb` destroys nothing: read-only over the host, produces an independent object.
- Each crate justifies its `Cargo.toml` line by line — the root doesn't accept frivolous deps.

## Estado (2026-06-10)

### Hecho
- Cadena de arranque/vida temprana sobre subcrates `init/` (`arje-zero` PID 1, `arje-incarnate`, `arje-loader`, `arje-installer`, `arje-absorb`, `arje-packager`, `arje-snapshot`, `arje-soma`, `arje-net-bring-up`, `arje-kernel`, `arje-getty-stub`) y `runtime/` (`arje-bus`, `arje-cas`, `arje-echo`, `arje-wasm`, `arje-brain` + `-rules`/`-cognitive`/`-audit`).
- Integración con el plano de control sandokan: `arje-zero` adopta `sandokan-lifecycle::Backoff` (dedup #1), el bus gana mensajes de observación `EnteStatus`/`EnteTelemetry` (dedup #3 paso A), y `arje-zero` queda alcanzable como `sandokan-core::Engine` vía `sandokan-arje-engine`.
- Process monitor visible: card de unidades en `arje-card-llimphi` con estado en vivo vía Engine, panel del brain (reglas/entropía/audit log), card de escritorio con aislamiento del init, y restarts visibles end-to-end (`↻N`, monitor Fase 2/3).
- `arje-compat`: shims `machined`/`hostnamed`, `list_unit_files` contra el card store. Cards de seed (`arje-host`, `arje-qemu`). Menús principal + contextuales (lote 5).
- Cierres post-31/05: `RunCard{card}` arbitraria por el bus (`Engine::run` transmite la Card por el wire, gateada por `Capability::Spawn` + inhibiciones + audit), cleanup defensivo del socket del bus (distingue stale vs vivo), y `LocalEngine` con `RestartTracker` por Entity + CPU% real en telemetry. El socket de `sandokan-daemon` **se mantiene** por decisión del SDD de sandokan (frontea un `LocalEngine` no-PID1; no es redundante con arje-bus).
- **Robustez de arranque a prueba de fallos tontos:** el bootstrap de montajes (`arje-kernel::surface`) es idempotente y convivente — crea el dir target, salta lo ya montado (no apila sobre `/run`/`/proc`/… del initramfs u OpenRC) y nunca aborta. **Watchdog de hardware** (`arje-kernel::watchdog`): PID 1 acaricia `/dev/watchdog` desde el bucle primordial; si el bucle se cuelga, el kernel reinicia en vez de quedar muerto. Default 30 s, `ARJE_WATCHDOG_SECS=0` lo desactiva; desarme con cierre mágico en shutdown limpio. Para probar en QEMU: `-watchdog i6300esb` o `modprobe softdog`.

### Pendiente
- Endurecer la ruta aarch64 del `arje-kernel` (soporte aún limitado) y el camino de instalación/absorción sobre hardware real más allá de QEMU.
