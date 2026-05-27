# llimphi

> Native UI framework: HAL · raster · layout · text · theme · ui — plus widgets and modules.

`llimphi` is the graphics engine all monorepo apps share. Retained-mode declarative pipeline over `vello` + `wgpu` + `taffy`, with `fontdue`/`harfbuzz` shaping, `Dark/Light/Aurora/Sunset` themes, multi-platform HAL (Wayland · X11 · Win32 · Android · Wawa). Design detail in [SDD.md](SDD.md).

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

Crates listed in [README.md](README.md) (framework, widgets, modules, android).

## Considerations

- **Single API: declarative `View<Msg>`.** No imperative, no foreign vDOM.
- **Same scene tree on Wayland and Wawa**: HAL abstracts the surface.
- Widgets are **purely visual**; modules encapsulate state + behavior.
