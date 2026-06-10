# llimphi

> Native UI framework: HAL · raster · layout · text · theme · ui — plus widgets and modules.

![a dense UI composed only of compositor primitives: dark theme, tabbed top bar, sidebar, syntax-highlighted code editor, rich text, metric cards with gradients and shadows, a bar chart made of pure rects and a floating toast](https://tawasuyu.net/02_ruway/llimphi/pantallazo.png)

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

## Demo

A self-contained prezi-style tour lives in [`demo/`](demo/): arch, Elm loop, widget kit, and headless screenshots of ~10 real apps running on llimphi (cosmos · pluma · nada · takiy · tullpu · supay · dominium · nahual · shuma…). Open `demo/index.html` in any browser, or serve it (`python3 -m http.server -d demo`). Space / arrows / click to navigate; auto-advances every 6 s — ready to screen-record.

## Considerations

- **Single API: declarative `View<Msg>`.** No imperative, no foreign vDOM.
- **Same scene tree on Wayland and Wawa**: HAL abstracts the surface.
- Widgets are **purely visual**; modules encapsulate state + behavior.
