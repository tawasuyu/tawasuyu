# llimphi

> Native UI framework: HAL · raster · layout · text · theme · ui — plus widgets and modules.

`llimphi` is the graphics engine all monorepo apps share. Retained-mode declarative pipeline over `vello` 0.7 + `wgpu` 27 + `taffy`, with `parley` 0.6 shaping (embedded DejaVu Sans as symbol fallback), `Dark/Light/Aurora/Sunset` themes, AccessKit accessibility, multi-platform HAL (Wayland · X11 · Win32 · Android · Wawa).

**Usage manual:** [MANUAL.md](MANUAL.md) — full reference (Elm loop, `View<Msg>` DSL, the ~44 widgets and 10 modules, GPU path, gotchas) for humans and AI. Design rationale and roadmap: [SDD.md](SDD.md).

Philosophy: **widgets aren't designed against mockups; they're designed with what `vello` and `taffy` can do.**

## Install

```sh
[dependencies]
llimphi-ui = { workspace = true }
llimphi-theme = { workspace = true }
llimphi-widget-... = { workspace = true }
```

## Compatibility

- **Linux/Wayland** — primary backend.
- **Linux/X11** — via XWayland.
- **macOS / Windows** — `winit` + `wgpu`.
- **Android** — HAL via `android` crates.
- **Wawa bare-metal** — alternative framebuffer HAL.

Full crate index (framework · widgets · modules · android) in [MANUAL.md](MANUAL.md) §19; per-crate tables in [LEEME.md](LEEME.md). The raster crate ships an opt-in `hybrid` feature (CPU+GPU renderer, no compute shaders) for targets without full compute support.

## Considerations

- **Single API: declarative `View<Msg>`.** No imperative, no foreign vDOM.
- **Same scene tree on Wayland and Wawa**: HAL abstracts the surface.
- Widgets are **purely visual**; modules encapsulate state + behavior.
