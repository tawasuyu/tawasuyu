# wawa-config — OS configuration bus

The desktop/OS **configuration bus**: a canonical TOML file
(`~/.config/wawa/config.toml`) + a watcher (`notify`) that re-emits live
changes, over a system layer (`/etc/wawa/config.toml`) that the user can
override. Consumers (desktop Llimphi apps) subscribe and react on the fly:
changing theme/accent propagates **without restarting**.

UI-agnostic: **it does not depend on `llimphi`**. The adapter that assembles an
effective `Theme` out of the `WawaConfig` lives in `wawa-config-llimphi`.

## What it exposes

- `WawaConfig` — the configuration (theme variant, accent override, …).
- Load with merge of `/etc/wawa` (system) under user override.
- Watcher (`notify`) that re-emits the config when the file changes.

## Status (2026-05-31)

### Done
- Canonical TOML file + `notify` watcher (live reload).
- System layer `/etc/wawa/config.toml` merged under the user override.
- Auto-apply of the accent to the global theme; ≈10 tests.
- Consumed by nada, cosmos, nakui, dominium, shuma, nahual, minga, arje,
  wawa-panel and `wawactl` (CLI).

### Pending
- Broader config schema (beyond theme/accent).
- TOML version validation/migration.
- Consumption from the bare-metal wawa OS (today it is the host desktop).

## Place in the repo

`shared/wawa-config` — UI-agnostic source of truth. Theme adapter:
`wawa-config-llimphi`. CLI: `wawactl`.
