# arje

> `arje` (Greek *ἀρχή*: origin, principle). Bootloader and the system's early life.

`arje` covers from "you pressed POWER" to "the kernel is running": seeds, packaging, installation, absorption of an existing system, its own kernel, minimal networking, rules + audit, CAS, snapshots, soma, WASM init.

## Install

```sh
cargo run --release -p arje-packager -- build --target iso
cargo run --release -p arje-installer
cargo run --release -p arje-absorb -- /path/to/system
```

## Compatibility

- **Linux x86_64** — primary target.
- **aarch64** — `arje-kernel` supports it (limited).
- **Wawa** — `arje` is the natural bootloader for `wawa-kernel`.

Crates listed in [README.md](README.md).

## Considerations

- **`arje` is for boot, not for governing the running system**. That's the `wawa-kernel`'s job.
- `absorb` destroys nothing: read-only over the host, produces an independent object.
- Each crate justifies its `Cargo.toml` line by line — the root doesn't accept frivolous deps.

## Estado (2026-05-31)

### Hecho
- Cadena de arranque/vida temprana sobre subcrates `init/` (`arje-zero` PID 1, `arje-incarnate`, `arje-loader`, `arje-installer`, `arje-absorb`, `arje-packager`, `arje-snapshot`, `arje-soma`, `arje-net-bring-up`, `arje-kernel`, `arje-getty-stub`) y `runtime/` (`arje-bus`, `arje-cas`, `arje-echo`, `arje-wasm`, `arje-brain` + `-rules`/`-cognitive`/`-audit`).
- Integración con el plano de control sandokan: `arje-zero` adopta `sandokan-lifecycle::Backoff` (dedup #1), el bus gana mensajes de observación `EnteStatus`/`EnteTelemetry` (dedup #3 paso A), y `arje-zero` queda alcanzable como `sandokan-core::Engine` vía `sandokan-arje-engine`.
- Process monitor visible: card de unidades en `arje-card-llimphi` con estado en vivo vía Engine, panel del brain (reglas/entropía/audit log), card de escritorio con aislamiento del init, y restarts visibles end-to-end (`↻N`, monitor Fase 2/3).
- `arje-compat`: shims `machined`/`hostnamed`, `list_unit_files` contra el card store. Cards de seed (`arje-host`, `arje-qemu`). Menús principal + contextuales (lote 5).

### Pendiente
- Cleanup del transporte: deprecar/borrar el socket propio de `sandokan-daemon` (redundante con arje-bus); `run` arbitrario (`RunCard{card}`) aún mapeado a spawn store-based.
- `RestartTracker::count` en `LocalEngine` (hoy deja restarts en 0 fuera de PID 1).
- Endurecer la ruta aarch64 del `arje-kernel` (soporte aún limitado) y el camino de instalación/absorción sobre hardware real más allá de QEMU.
