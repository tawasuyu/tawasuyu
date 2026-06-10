# wawa-config-llimphi — Llimphi adapter for wawa-config

Assembles the effective Llimphi `Theme` out of the `WawaConfig` (variant +
accent override). It exists to **avoid forcing `wawa-config` to depend on
`llimphi-theme`** — so the configuration bus stays UI-agnostic and this
crate bridges towards the render.

## What it exposes

- `WawaConfig` → Llimphi `Theme` conversion (variant + accent).
- Helper so a Llimphi app obtains its effective theme and reacts to the
  `wawa-config` watcher.

## Status (2026-05-31)

### Done
- `WawaConfig` → `Theme` adapter (variant + accent override).
- Wired to several consumers (cosmos, nakui, nahual-shell, nada, dominium,
  shuma).
- ≈4 tests.

### Pending
- Map more config fields to the theme as `wawa-config` grows.
- Transitions/animation on live theme change (today it applies directly).

## Place in the repo

`shared/wawa-config-llimphi` — theme frontend over `shared/wawa-config`.
