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
| [`wawactl`](wawactl/README.md) | CLI: `wawactl show`, `wawactl set`, `wawactl gc`, `wawactl daemon-firma`, etc. |

## Considerations

- **Userspace, not kernel.** Boot/fs/proc tweaks → `03_ukupacha/wawa`.
- Panel and `wawactl` share the config model with the desktop shell (via `shared/wawa-config`).

## Status (2026-06-09)

### Done

- **`wawa-panel-llimphi`**: configuration panel navigated by a rail of **dientes**
  (`llimphi-widget-dock-rail`) with a 3-level hierarchy (tab → items in sidebar →
  canvas): **System** tabs (Appearance · Language · Interface · Boot · Modules) and
  **Information**, plus dientes for subscribed apps (**mirada** — incl. keymap editable as
  a table — and **pata**). Renders with `llimphi-module-allichay` (in-situ editing of
  table/list cells, `#RRGGBB` hex field in the color-picker, resizable/hideable sidebar);
  producer and consumer of the `shared/wawa-config` bus, with debounced saving.
- **`wawactl`**: CLI over the same bus — `path`, `show`, `get`, `set`, `module`,
  `reset`, `watch`, `firmar-cuaderno`, `claves`, `gc`, `daemon-firma`; with `--system`
  for the `/etc/wawa/config.json` layer and `--layer system|user|effective` in `show`.
- **Two-layer configuration bus** (`shared/wawa-config`): system (`/etc/wawa`) +
  user (`$XDG_CONFIG_HOME/wawa`), deep-merge on `modules`, atomic save (tmp+rename),
  `notify` watcher with 200 ms debounce over both layers. Adapter
  `shared/wawa-config-llimphi` (`theme_from_wawa`, 4 tests).
- **Real consumers** already wired: `nada`, `nahual-shell-llimphi`,
  `dominium/cosmos/nakui-explorer` app-llimphi (live theme/accent/lang).
- **`wawactl` ↔ kernel** (Phases 38–63): external signer channel, multi-author crypto
  customs + CRL + pre-authorization window, bidirectional daemon + audit ring,
  daemon pubkey reveal + multi-slot compositor envelope, Boot Trust Ceremony
  with real sovereign keys, virtio-console + high-speed crypto HAL,
  `wawactl gc` (remote control of the GC over virtio-console), `daemon-firma` distinguishes
  notebook and configuration.
- **i18n docs**: README EN (default) + LEEME ES + README.qu QU, with live reload.
- **Menus** (batch 6): main menu + contextual menus in the panel.

### Pending

- **Module toggles with real effect**: today they persist state and hide the app's diente
  in the panel, but they don't start/stop daemons (awaits the contract with the OS
  supervisor: arje/mirada-compositor/shuma).
- **Permissions**: any user process can touch the file; missing
  `getpeercred`/`SO_PEERCRED` for multiuser/sandboxes.
- **Migration to wawa-OS**: `system_config_path()` will return arje's native mechanism
  instead of `/etc/wawa` (stable public API).
- Bus and flag detail in [SDD.md](SDD.md).
