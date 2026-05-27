# 02 ruway · to make

`ruway` (Quechua: *to do, to make, to fabricate*). This is the **action** quadrant: interfaces, compositors, brokers, shells. What `unanchay` perceived and `yachay` modeled becomes here something a human uses, that composes with other pieces, that compiles to a binary that boots and responds.

The quadrant's rule is **the material rules**: a widget isn't designed against mockups, it's designed with what `vello` and `taffy` can do; a compositor isn't designed in the abstract, it's measured against `weston`. Materiality limits and guides.

## Applications

- **[chasqui](chasqui/README.md)** — message broker + typed bus. The monorepo's nervous system.
- **[llimphi](llimphi/README.md)** — native UI framework (hal · raster · layout · text · theme · ui) + widgets + modules. The graphical core all apps share.
- **[mirada](mirada/README.md)** — Wayland compositor (`mirada-compositor`) + XDG portal (`mirada-portal`) + login greeter (`mirada-greeter`). The display stack.
- **[nada](nada/README.md)** — file editor over Llimphi: file tree + LSP-aware editor + real clipboard + sessions. Test bench of the framework.
- **[nahual](nahual/README.md)** — everyday viewers: file shell, text viewer, image viewer.
- **[shuma](shuma/README.md)** — interactive shell (zsh/fish parity) with views in a Llimphi chassis (TopBar/Main/BottomBar/Drawer).
- **[supay](supay/README.md)** — DOOM-style renderer over Llimphi (FFI to `doomgeneric`, sprite atlas, WAD palettes).
- **[takiy](takiy/README.md)** — music. Capture, sequencing, audio render.
- **[wawa](wawa/README.md)** — control panel + `wawactl` for the Wawa stack (the userspace counterpart of `03_ukupacha/wawa`'s kernel).

## Manifesto

> **To make is to commit to matter.**
> An API doesn't exist until a second app uses it; a widget doesn't exist until it renders at 60 fps on a real screen.
>
> 1. **Zero graphical deps in `core`.** The engine decides; the UI shows — and they are different crates.
> 2. **The same scene tree in Wayland and Wawa.** Llimphi/HAL abstracts the surface; the rest of the stack is identical.
> 3. **The user sets the pace.** If the frame slips, we simplify before asking for more compute.
> 4. **Tools that respect the craftsperson.** Consistent shortcuts, reliable undo, clipboard that works with the real system.
