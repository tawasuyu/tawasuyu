# 02 ruway · to make

`ruway` (Quechua: *to do, to make, to fabricate*). This is the **action** quadrant: interfaces, compositors, brokers, shells. What `unanchay` perceived and `yachay` modeled becomes here something a human uses, that composes with other pieces, that compiles to a binary that boots and responds.

The quadrant's rule is **the material rules**: a widget isn't designed against mockups, it's designed with what `vello` and `taffy` can do; a compositor isn't designed in the abstract, it's measured against `weston`. Materiality limits and guides.

## Applications

- **[ayni](ayni/README.md)** — sovereign person-to-person chat, local-first, serverless: the conversation as a reproducible cryptographic graph (BLAKE3 + DAG), `agora` identity, `chasqui`/`minga` transport.
- **[cards](cards/README.md)** — one way to read every kind of Card: projects the suite's Card documents (runtime, semantic, UI) onto a single canonical structure.
- **[chasqui](chasqui/README.md)** — message broker + typed bus. The monorepo's nervous system.
- **[llimphi](llimphi/README.md)** — native UI framework (hal · raster · layout · text · theme · ui) + widgets + modules. The graphical core all apps share.
- **[media](media/README.md)** — the suite's audio/video domain: player, decoders, visualizers, recorder.
- **[mirada](mirada/README.md)** — Wayland compositor (`mirada-compositor`) + XDG portal (`mirada-portal`) + login greeter (`mirada-greeter`). The display stack.
- **[nada](nada/README.md)** — file editor over Llimphi: file tree + LSP-aware editor + real clipboard + sessions. Test bench of the framework.
- **[nahual](nahual/README.md)** — everyday viewers: file shell, text viewer, image viewer.
- **[paloma](paloma/LEEME.md)** — native mail client over Llimphi: IMAP in, SMTP out, no browser in between.
- **[pata](pata/README.md)** — the desktop frame: declarative bars, panels and a dock from one config file; same model on Linux and Wawa.
- **[raymi](raymi/LEEME.md)** — native calendar + contacts (CalDAV/CardDAV), paloma's companion; reuses its account layer.
- **[shuma](shuma/README.md)** — interactive shell (zsh/fish parity) with views in a Llimphi chassis (TopBar/Main/BottomBar/Drawer).
- **[supay](supay/README.md)** — DOOM-style renderer over Llimphi (FFI to `doomgeneric`, sprite atlas, WAD palettes).
- **[takiy](takiy/README.md)** — music. Capture, sequencing, audio render.
- **[tullpu](tullpu/README.md)** — layered image editor where nothing is destroyed: the layer stack is a content-addressed DAG; derived layers go stale instead of overwriting.
- **[uya](uya/README.md)** — sovereign video calls (`uya` = "face" in Quechua): agnostic `uya-core` + Llimphi frontends over the suite's P2P node (`card-net`).
- **[wawa](wawa/README.md)** — control panel + `wawactl` for the Wawa stack (the userspace counterpart of `03_ukupacha/wawa`'s kernel).

## Manifesto

> **To make is to commit to matter.**
> An API doesn't exist until a second app uses it; a widget doesn't exist until it renders at 60 fps on a real screen.
>
> 1. **Zero graphical deps in `core`.** The engine decides; the UI shows — and they are different crates.
> 2. **The same scene tree in Wayland and Wawa.** Llimphi/HAL abstracts the surface; the rest of the stack is identical.
> 3. **The user sets the pace.** If the frame slips, we simplify before asking for more compute.
> 4. **Tools that respect the craftsperson.** Consistent shortcuts, reliable undo, clipboard that works with the real system.
