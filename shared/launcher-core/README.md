# launcher-core — layout of the single launcher engine

The **configurable layout** of a single reusable launcher engine. Not three
launchers (mirada / shuma / wawa) but **one** data structure mounted
anywhere. What varies per environment does NOT live here: the render (Llimphi in
host/shuma, compositor in wawa) and the execution instruction
(`app_bus::Launcher`) are injected adapters. Here lives only the *what gets
drawn and where*, as pure `no_std` data.

It generalizes the `WidgetSpec { kind, props }` from `mirada-launcher`: each `Module`
is a `kind` + arbitrary props that the render interprets.

## What it exposes

- `Surface` — the complete surface: `bars` + `docks` + `floating` + `app_menu`.
  Described in `~/.config/tawasuyu/launcher.toml`, identical in host/shuma/wawa.
- `Bar` (anchored to an `Edge`, start/center/end slots), `Dock` (with `tear_off`),
  `FloatingCard` (materialized tear-off), `AppMenuBar` (mac-style global menu).
- `Module` (`kind` + `props: BTreeMap<String, Prop>`) with typed accessors.
- `Surface::desktop_default()` — a sensible startup desktop.

`#![no_std] + alloc`; depends only on `serde`. The render is `launcher-llimphi`.

## Status (2026-05-31)

### Done
- Complete schema for `Surface`/`Bar`/`Dock`/`FloatingCard`/`AppMenuBar`/`Module`.
- Portable `Prop` (bool/int/float/str) and accessors `str_prop`/`f64_prop`/`bool_prop`.
- Desktop default + TOML/JSON roundtrip; schema tests.

### Pending
- Render in wawa (compositor) — today only `launcher-llimphi` consumes it on host.
- Persistence of tear-offs / floating positions across sessions.
- Extra module kinds beyond the documented builtins.

## Place in the repo

`shared/launcher-core` — data schema. Frontend: `launcher-llimphi`. App catalog
+ bus: `app-bus`.
