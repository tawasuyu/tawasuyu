# wawa (userspace)

> Userspace counterpart of `03_ukupacha/wawa`: control panel + CLI.

This is what the Wawa operator uses **from a Linux host** (not from inside the kernel): the Llimphi panel for state/config, and `wawactl` for terminal ops. The kernel/bootloader/filesystem side is in `03_ukupacha/wawa/`. Detail in [SDD.md](SDD.md).

## Install

```sh
cargo run --release -p wawa-panel-llimphi
cargo run --release -p wawactl
```

## Compatibility

- **Linux** — primary host. Talks to `wawa-kernel` via virtio-console or Unix socket.
- **macOS / Windows** — only if Wawa runs in an accessible VM (TCP).

## Crates

| Crate | Role |
|---|---|
| [`wawa-panel-llimphi`](wawa-panel-llimphi/README.md) | Llimphi control panel: app state, config, resources. |
| [`wawactl`](wawactl/README.md) | CLI: `wawactl status`, `wawactl deploy`, etc. |

## Considerations

- **Userspace, not kernel.** Boot/fs/proc tweaks → `03_ukupacha/wawa`.
- Panel and `wawactl` share the config model with the desktop shell (via `shared/wawa-config`).
