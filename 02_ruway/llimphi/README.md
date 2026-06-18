# llimphi

> Native UI framework — 2D **and** 3D: HAL · raster · layout · text · theme · ui · 3D voxel engine — plus widgets and modules.

[![Llimphi showreel — real widgets (switch, slider, progress, segmented control, buttons, radial) animating live on the Tawa theme, then reflowing across layouts](https://tawasuyu.net/02_ruway/llimphi/llimphi_showreel.gif)](https://tawasuyu.net/02_ruway/llimphi/llimphi_showreel.mp4)

`llimphi` is the graphics engine all monorepo apps share. Retained-mode declarative pipeline over `vello` 0.7 + `wgpu` 27 + `taffy`, with `parley` 0.6 shaping (Inter as the default UI font, DejaVu Sans as symbol fallback), `Dark/Light/Aurora/Sunset/Tawa` themes, AccessKit accessibility, multi-platform HAL (Wayland · X11 · Win32 · Android · Wawa).

**Usage manual:** [MANUAL.md](MANUAL.md) — full reference (Elm loop, `View<Msg>` DSL, the ~44 widgets and 10 modules, GPU path, gotchas) for humans and AI. Design rationale and roadmap: [SDD.md](SDD.md).

Philosophy: **widgets aren't designed against mockups; they're designed with what `vello` and `taffy` can do.**

## Not just 2D — a 3D voxel engine

![A procedural voxel world (sand, cactus, carved rivers) orbiting — rendered by llimphi-3d](https://tawasuyu.net/02_ruway/llimphi/llimphi_voxel.gif)

`llimphi-3d` adds a **3D engine** that composes voxels (GPU ray-march) and triangle meshes in one shared depth pass (`Scene3d` + `VoxelRenderer` + `Renderer3d` + a keyframed cinema camera). It mounts straight into any 2D `View` via the GPU paint node, so a 3D viewport can live inside a panel next to ordinary widgets.

On top of it, `llimphi-voxel` brings the *content* layer — procedural world-gen (`WorldRecipe`), articulated characters (age + animation clips), and a scripted scene **director** — and `llimphi-voxel-studio` is a full **world creator**: edit worlds, cast characters and direct scenes (with a filmed camera) from a live-preview UI, describe any of them in plain language to an LLM assist, and **export the scene to a video** with a synced soundtrack. The GIF above is one such world, rendered headless frame-by-frame.

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

The showreel above is rendered **headless, frame-by-frame, fully deterministic** (no clock; each frame is a pure function of `t ∈ [0,1]`). Source: [`llimphi-compositor/examples/showreel.rs`](llimphi-compositor/examples/showreel.rs); regenerate with [`scripts/showreel.sh`](../../scripts/showreel.sh).

A dense static shot — a full UI composed only of compositor primitives (tabbed top bar, sidebar, code editor, rich text, metric cards with gradients and shadows, a bar chart of pure rects, a floating toast):

![a dense UI composed only of compositor primitives](https://tawasuyu.net/02_ruway/llimphi/pantallazo.png)

Full tour: **<https://tawasuyu.net/02_ruway/llimphi/demo/>** — a self-contained slide deck (arch, Elm loop, widget kit, and headless screenshots of ~10 real apps running on llimphi: cosmos · pluma · nada · takiy · tullpu · supay · dominium · nahual · shuma…). Source under [`demo/`](demo/index.html).

## Considerations

- **Single API: declarative `View<Msg>`.** No imperative, no foreign vDOM.
- **Same scene tree on Wayland and Wawa**: HAL abstracts the surface.
- Widgets are **purely visual**; modules encapsulate state + behavior.
