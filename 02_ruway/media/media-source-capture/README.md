# media-source-capture

**Live** capture as `media_core::FrameSource` — the INPUT side of the
domain. While the file-based `media-source-*` crates play back bytes on
disk, this one produces frames from a real-time device. Its reason for
being: feed `media-recorder-webm` to record the camera to a **native**
AV1+Opus `.webm`, without ffmpeg.

```text
v4l2 camera ──thread──▶ convert (YUYV/MJPEG→RGBA) ──▶ LiveSink
                                                       │  (latest-frame slot + version)
render loop ◀── FrameSource::tick ◀── LiveSource ──────┘
                         │
                         ▼
            media-recorder-webm ──▶ native AV1+Opus .webm
```

## The two pieces

### Agnostic core — `LiveSource` / `LiveSink` (always compiled)

A **latest-frame slot**: `Arc<Mutex>` + atomic version. The producer
pushes frames from its own thread/timing (`push_rgba` or `push_raw`);
the consumer reads them in `tick` **without blocking** — if there is no
new frame since the last read, `tick` returns `None` and does not touch
the buffer. This is the correct discipline for a live source inside a
render loop:

- the render **never stalls** waiting on the device;
- an old frame is **never re-emitted** (it doesn't inflate the recorder
  to screen fps when the camera runs slower);
- if the producer is faster than the consumer, **only the last** frame
  survives (we want the now, not a queue).

It is reusable by **any** grabber: camera today, screen capture tomorrow
(without a new crate), a compute shader, or the network.

### v4l2 camera backend — `CameraSource` (feature `camera`, opt-in)

Opens `/dev/videoN`, negotiates a format, and runs a dedicated thread
that converts each frame to RGBA and pushes it to the `LiveSink`. It
stops and joins itself on drop. `open()` blocks until the format is
negotiated — so "no camera" / "invalid format" arrives synchronously,
not silently mid-playback.

```rust
use media_source_capture::{CameraSource, CameraOptions};

let cam = CameraSource::open_default()?;          // /dev/video0, 640×480, YUYV
println!("{}×{} {:?}", cam.width(), cam.height(), cam.format());
// cam: FrameSource — plug into the pipeline or the recorder.
```

### X11 screen backend — `ScreenSource` (feature `screen`, opt-in)

Same mold as the camera, but the source is the **server's
framebuffer**: a dedicated thread does a `GetImage` of the X11 root
window, converts to RGBA and pushes to the `LiveSink`. The screen does
not set the pace (unlike the camera, which gets it from the driver), so
an internal timer caps it at `fps` so as not to re-record a framebuffer
that didn't change.

```rust
use media_source_capture::{ScreenSource, ScreenOptions};

let scr = ScreenSource::open_default()?;           // $DISPLAY, full screen, 30 fps
println!("{}×{} {:?}", scr.width(), scr.height(), scr.format());
// scr: FrameSource — same recorder, now you record the screen to .webm.
```

It keeps the core's promise: "camera today, screen capture tomorrow
**without a new crate**" — it reuses `LiveSource`/`LiveSink` as-is. X11
only for now; Wayland (portal + PipeWire) would be another backend on
the same core. `GetImage` copies the framebuffer over the socket every
frame (MVP); MIT-SHM (shared memory) is the natural optimization when it
starts to hurt.

### Wayland screen backend — `WaylandScreenSource` (feature `wayland`, opt-in)

Same mold as X11, but over the `wlr-screencopy` protocol
(`zwlr_screencopy_manager_v1`): a thread copies the output into an shm
buffer (memfd+mmap), converts to RGBA and pushes to the `LiveSink`.
**Pure-Rust** (`wayland-client` + `wayland-protocols-wlr`, with `dlopen`
it doesn't even link libwayland at build) — same ethos as x11rb.

Wayland forbids by design that a client read the screen without a
sanctioned protocol. `wlr-screencopy` is exposed by **wlroots**
compositors (Sway, Hyprland, river); **GNOME/KDE don't** — there the way
is xdg-desktop-portal + PipeWire (another backend, which would drag in
libpipewire in C). The `media-recorder-app` chooses X11 or Wayland at
runtime depending on `$WAYLAND_DISPLAY`/`$DISPLAY`.

```rust
use media_source_capture::WaylandScreenSource;
let scr = WaylandScreenSource::open_default()?;  // first output, 30 fps
```

The full screen→`.webm` loop (without ffmpeg) is provided as a runnable
example, in two variants:

```bash
# screen only (AV1). Needs $DISPLAY.
cargo run -p media-source-capture --example grabar_pantalla \
    --features screen --release -- 5 pantalla.webm 30

# screen + microphone (AV1+Opus). Needs $DISPLAY + input device.
cargo run -p media-source-capture --example grabar_pantalla_audio \
    --features "screen mic" --release -- 5 pantalla.webm 30
```

The pixel-format conversion (`convert`) is **pure and testable without
any device** — it lives separate from the backends. It supports `YUYV`
(YUV 4:2:2, BT.601 limited range — the v4l2 convention), `MJPG` (via the
`image` crate), `RGB3`, `BGR3` and the 32-bit packings
(`Bgrx32`/`Xrgb32` from X11 + `Rgbx32` from Wayland's XBGR8888, padding
ignored).

### Audio side — `AudioLiveSink`/`AudioLiveSource` + `MicSource`

The audio mirror of the live core. The difference from video is the
discipline: an old frame is dropped (we want the now), but audio is
**not dropped** — the slot is a **ring buffer** that is drained in order
(`AudioSource::fill`), filling with silence on underrun. The ring is
bounded (~4 s): if the consumer hangs, the oldest is dropped and the
overrun is counted, instead of growing without limit.

`MicSource` (feature `mic`, opt-in) opens the default input device via
cpal and pushes the samples to the `AudioLiveSink` from the realtime
callback. It asks for **48 kHz** (Opus's native rate) so the recorder
doesn't degrade; a device that only gives 44.1 kHz records video-only.

```rust
use media_source_capture::{ScreenSource, MicSource};

let scr = ScreenSource::open_default()?;   // video: FrameSource
let mic = MicSource::open_default()?;       // audio: AudioSource (48 kHz)
// both → media-recorder-webm → native AV1+Opus .webm screencast.
```

The full screen+mic→`.webm` loop is in the `grabar_pantalla_audio`
example (below).

## Why the backends are opt-in

`v4l` drags in `v4l2-sys-mit` → `bindgen` → `libclang`, a heavy and
fragile **build** dependency on parallel builds (the `cargo check
--workspace` smoke test blew up with *"libclang not loaded on this
thread"*). Same logic as the `foreign-*` bridges: hardware/foreign
enters opt-in, the domain core stays light. `screen` (x11rb), `wayland`
(wayland-client + wlr-protocols) and `mic` (cpal) are all pure-Rust and
without a C lib at build, but they stay opt-in anyway: they are system
backends (they need an X server / wlroots compositor / input device) and
not every platform wants them.

```bash
cargo test  -p media-source-capture                    # pure core (21 tests) + integration (2)
cargo check -p media-source-capture --features camera  # v4l2 backend (needs libclang)
cargo check -p media-source-capture --features screen  # X11 backend (x11rb, pure-Rust)
cargo check -p media-source-capture --features wayland # Wayland wlr-screencopy backend (pure-Rust)
cargo check -p media-source-capture --features mic     # microphone backend (cpal)
```

`camera` compiles wherever there are `videodev2` headers + `libclang`
and **running** needs a real `/dev/videoN`; `screen` compiles anywhere
and running needs a `$DISPLAY`; `mic` needs an input device. In all of
them the backend layer is thin and the testable logic (conversion +
latest-frame slots / audio ring) lives outside — just like
`media-audio-cpal` needs a sound sink to make sound but not to compile.

## Tests

- `convert::tests` — conversion round-trips (gray/red YUYV, RGB/BGR, the
  32-bit `Bgrx32`/`Xrgb32`/`Rgbx32` with padding ignored, rejection of
  truncated buffers, FourCC mapping).
- `live_audio::tests` — audio ring: drains in order, underrun fills
  silence, partial fill keeps continuity, overrun drops the old and
  counts it, orphan detection.
- `lib::tests` — `LiveSource` contract: starts empty, emits only new
  frames, drops intermediates, detects orphan.
- `tests/captura_a_webm.rs` — the star loop without hardware: `LiveSink`
  (synthetic) → `RecordedFrameSource` → `media-recorder-webm` produces a
  `.webm` with a valid EBML header; and the no-re-emission guarantee for
  stale frames.
