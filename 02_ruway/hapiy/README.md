# hapiy — screen capture for the suite

*Léelo en español: [LEEME.md](LEEME.md).*

"hapiy" (Quechua: *to grab / to catch*). The suite's screenshot tool — the
"Spectacle" — it catches what **mirada** paints. Capture the whole screen, one
monitor or a region; save a PNG; and with `--edit`, open it straight in
**tullpu** (the image editor) to annotate or crop.

## Why

mirada implements the `zwlr_screencopy_v1` protocol server-side, so capturing the
desktop is already possible — `hapiy` is the *client*: a sovereign one (no
external grim required) with a clean handoff to the suite's own image editor.

## Crates

- **`hapiy-core`** — the agnostic engine, fully `cargo test`-able (no Wayland, no
  GPU, no UI): `Shot` (RGBA buffer) + `Region`/crop, PNG encoding, the `Capturer`
  trait (+ `MockCapturer`), and the **tullpu handoff** (`tullpu_launch` — tullpu
  already opens a PNG passed as its first arg).
- **`hapiy-capture`** — the capture backends behind the `Capturer` trait, shared
  by the CLI and the GUI:
  - **native** (default, feature `wayland`) — our own `zwlr_screencopy` client
    over `wayland-client`, buffer via `wl_shm` (tempfile + mmap). No grim.
  - **grim** — shells out to `grim` (which mirada already allows). Fallback.
  - `capturer(Backend::Auto)` tries native and falls back to grim.
- **`hapiy`** — the CLI binary (`hapiy`): scriptable capture for terminal/CI.
- **`hapiy-llimphi`** — the GUI (`hapiy-llimphi`, the "Spectacle window"):
  capture, live preview, **Save**, and **Edit in tullpu**, on Llimphi.

## Usage

```bash
hapiy                       # capture → ~/Pictures/hapiy-<ts>.png
hapiy -o /tmp/foo.png       # explicit destination
hapiy --region 100,80,640,480
hapiy --display eDP-1       # one monitor (see --list-displays)
hapiy --edit                # capture and open it in tullpu to annotate
hapiy --list-displays
hapiy --backend grim|native|auto
```

## Status

`hapiy-core` is covered by tests (crop, PNG roundtrip, tullpu handoff, mock
capture). The native `zwlr_screencopy` client is **verified working against
mirada**; `--backend auto` still degrades to grim on any failure. Both the CLI
and the GUI run.

The GUI does region select with a **live selection rectangle** (mark two corners
on the preview → crop), a capture **delay** (`⏱ Capturar 3s`), copy-to-clipboard,
save, and Edit-in-tullpu — and it **minimizes its own window during the shot** so
hapiy stays out of the capture (via `Handle::set_minimized`, added to llimphi-ui).
