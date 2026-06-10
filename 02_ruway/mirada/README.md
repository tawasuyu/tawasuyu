# mirada

> Tawasuyu's display stack: compositor + portal + greeter + launcher.

`mirada` (Spanish *look, gaze*) delivers what the user sees on boot: the Wayland compositor, the XDG portal (file pickers, screenshare), the login greeter and a minimal launcher. All UI runs on Llimphi; the `bar-*` crates provide swappable status bars.

## Install

```sh
cargo run --release -p mirada-compositor
cargo run --release -p mirada-greeter
cargo run --release -p mirada-launcher
```

## Compatibility

- **Linux DRM/KMS** — native compositor.
- **Linux nested** — runs inside a host Wayland (dev mode).
- **Wawa** — minimal compositor on the kernel's framebuffer.

Crates listed in [LEEME.md](LEEME.md).

## Considerations

- **Doesn't replace `weston`/`sway`** in stability; replaces them in *Llimphi-HAL compatibility*. For full-stack monorepo, you want `mirada`.
- DRM/KMS requires permissions: launch from a greeter, not a user terminal.
- XDG portal is **complete**: `pluma`, `nada`, etc. can request file pickers via portal with no app-specific code.

## Status (2026-06-09)

### Done
- **Session persistence** (`mirada-brain/src/session.rs`): the desktop's *shape* (tiling per virtual desktop, which desktop each output showed, focus) survives restart in RON; *window homes* re-place reopened windows on their desktop, anchored by `app_id`.
- **Zoom-Z**: group windows into sub-spaces as a **multilevel fractal tree** (enter/exit arbitrary depth), with dormant layers (suspends the frames of the deep ones), grouping persisted by `app_id`, and **constellations** by process lineage (stable PID via `SO_PEERCRED`) with Alt-Tab per constellation.
- **Per-window capabilities** (`mirada-brain/src/permisos.rs`): the clipboard (`zwlr_data_control`) and key injection (`zwp_virtual_keyboard`) are denied **per executable** via denylists in config.
- **Background frame throttle**: visible unfocused windows receive their `frame` callbacks at 1 of every N vblanks (configurable divisor, `1` = off) — they stop burning GPU behind the focus.
- **Drag-to-zone**: configurable drag zones (`config.ron` → `zones` / `zone_presets`); dropping outside a zone leaves the window floating (overflow); `mirada-ctl cycle-zones` cycles presets.
- **Spatial view (Prezi)** (`mirada-app-llimphi/src/overview.rs` + base in the Brain): jump between desktops over a spatial plane.
- **Config hot-reload** (`mirada-brain/src/watch.rs`): keymap, config and rules are RON in `~/.config/mirada/` that reload hot, without restarting.
- **Full multi-monitor**: hotplug applied hot (create/destroy `OutputCtx`), scale + transform per output (mixed HiDPI, rotation), layer-shell and exclusive reservations per output, configurable layout (order + direction) and a cursor with no dead-zones between outputs.
- **`mirada-wallpaper`**: automatic wallpaper daemon (Bing/NASA/local folder + dynamic-desktop-style solar provider) that rewrites `wallpaper_path` in `config.ron` and lets the compositor's hot-reload apply it; procedural wallpaper by default with no embedded bytes.
- **The desktop shell migrated to `pata`** (`02_ruway/pata`, Phase 10, 2026-06-03): the old `mirada-launcher-llimphi` was retired. Its role —declarative bars/panels/dock, builtin widgets (clock/UTC, brightness, volume, clipboard, tray, gradient meters, astro), Quake drawer (shell via shuma-exec + AI), KDE-style task manager, floating conky cards, start button with native menu, tooltips— is covered and exceeded by `pata`, portable Linux/wawa. See `02_ruway/pata/SDD.md`.
- **System tray** (`tray`): hosted by `pata` (an `org.kde.StatusNotifierWatcher`, zbus on a separate thread) and paints the modern applets (nm-applet, blueman, chat clients) with their icon; click → activates the item over D-Bus.
- Desktop **wallpaper** (`config.ron` → `wallpaper_path`): PNG/JPEG/WebP scaled to the output, composited to the background (DRM backend). **Multi-monitor**: `outputs: [(name: "HDMI-A-1", wallpaper_path: "…", wallpaper_fit: "fill", order: 1)]` allows a different background per connector and lets you choose which monitor is primary (lower `order` → primary). `output_direction: "horizontal"` / `"vertical"` decides how the outputs are laid out. Anything unspecified falls back to the global. Hot-reload applies the wallpaper change without restarting (the layout does require a restart).
- **Openbox-style root menu**: right-click on the background unfolds the user's commands (`config.ron` → `menu`), with **nested submenus** in cascade (hover opens the child column); clicking a leaf launches it (DRM backend).
- **Auto-hiding bottom bar** (pata's `autohide`): at rest only a thin strip at the edge reveals it on pointer-over; moving into the free area hides it.
- `mirada-layout::outputs`: pure multi-monitor layout geometry, now **multi-DPI** (`Salida` + `disponer_logico`: lays out in logical coordinates according to each output's fractional scale, so a 1× and a 2× share a continuous plane). Ready for when scanout enumeration lands.
- `asistente-puente` / `mirada-asistente-llimphi`: end-to-end proposal pipeline (Unix-socket daemon mode + tested codec, human signing of proposals by hash — Phase 60).
- Compositor/portal/greeter over Llimphi-HAL; complete XDG portal (generic file pickers with no per-app code). Main + context menu (batches 4–6).
- **Greeter MVP closed**: remembers last user and desktop between logins, «Enter» button, `↑`/`↓` change desktop, pinned window (not draggable) and a configurable *Matrix* rain background (rusty rain). Real PAM backend + mock for iteration.
- **Robust VT switching** (`Ctrl+Alt+F1…F12`): the DRM backend honors both the dedicated keysym `XF86Switch_VT_n` and literal `Ctrl+Alt+Fn`, with session pause/resume cycling (libseat) — independent of the active keymap.

### Pending
- Compositor stability versus `weston`/`sway` (not yet a replacement in robustness).
- Minimal compositor over `wawa`'s framebuffer (depends on the winit-free Llimphi runtime).
- Hardening of the production DRM/KMS path beyond the MVP (multi-GPU/proprietary NVIDIA; today validated on Intel).
- Closing the assistant stack (beyond the base pipeline) and swappable `bar-*` as a product.
