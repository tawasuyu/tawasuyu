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
