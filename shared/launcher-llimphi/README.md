# launcher-llimphi

**The suite's one launcher renderer: bars, dock, global menu — declared as data, drawn by Llimphi.**

*Leé esto en español: [LEEME.md](LEEME.md).*

A desktop needs furniture: a bar with a clock, a dock with your apps, a
global menu, the occasional floating card. Instead of every shell
reimplementing that furniture, tawasuyu declares it as **data** — a
`Surface` (defined in `shared/launcher-core`, pure and `no_std`) — and this
crate renders that data to a Llimphi `View<Msg>`. Any app can mount the
launcher instead of rebuilding one.

## How it fits together

- **`launcher-core`** (shared): the declarative schema — `Surface`, `Bar`,
  `Dock`, `Module`, `AppMenuBar`, `FloatingCard`, plus `WidgetSpec`
  (kind + props). No rendering, no I/O.
- **`launcher-llimphi`** (this crate): the stateless renderer. The host's
  `Model` owns the state (which menu is open, which cards float); the API
  is `launcher_view(&LauncherSpec)` + `launcher_overlay(&LauncherSpec)`.
  Built-in modules (`app_menu`, `launch`, `dock`, `spacer`) are drawn here;
  live host modules (clock, cpu, ram, volume) are injected via
  `render_module`.
- **`app-bus`** (shared): app discovery (`AppRegistry`), app metadata
  (`AppEntry`), and real process launching (`ProcessLauncher`).
- **`tawasuyu-launcher`** (binary, here): the real thing — discovers apps,
  loads `~/.config/tawasuyu/launcher.toml` (falls back to
  `Surface::desktop_default`), launches processes. The dock supports
  tear-off: the ⤢ grip detaches an item as a floating card.

## Try it

```bash
# self-contained demo (hardcoded Surface, static modules)
cargo run -p launcher-llimphi --example launcher_demo

# the real launcher (app discovery + config + process launching)
cargo run -p launcher-llimphi --bin tawasuyu-launcher --release

# config schema reference
cat shared/launcher-llimphi/launcher.example.toml
```

## Status

Phase 3, stable: Surface rendering, real binary, discovery + seeding, real
launching, live host modules, dock tear-off, mountable API. Its historical
consumer (`mirada-launcher-llimphi`) was retired in 2026-06 when the
desktop frame moved to *pata*; the crate stays available for reuse.
Pending: tear-off persistence across sessions, real volume/brightness,
packaging as a desktop session, rendering the same Surface on the wawa
compositor.
