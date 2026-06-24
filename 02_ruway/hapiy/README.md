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
- **`hapiy`** — the `hapiy` binary. CLI + two capture backends behind the
  `Capturer` trait:
  - **native** (default, feature `wayland`) — our own `zwlr_screencopy` client
    over `wayland-client`, buffer via `wl_shm` (tempfile + mmap). No grim.
  - **grim** — shells out to `grim` (which mirada already allows). Fallback.
  - `--backend auto` (default) tries native and falls back to grim.

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
capture). The grim backend works today. The native `zwlr_screencopy` client
compiles and follows the standard flow (capture_output → buffer → copy → ready →
read mmap); it still needs **live verification against mirada** on a machine with
a running compositor — hence `--backend auto` degrades to grim on any failure.

Next: a Llimphi GUI (`hapiy-llimphi`) — region select with live preview, delay,
copy-to-clipboard, and an "Edit in tullpu" button over `hapiy-core`.
