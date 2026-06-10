# media-recorder-app

**Llimphi** screen recorder — the UI integration of the INPUT side of
`media`. A Rec/Stop button, a stopwatch and the recording state;
underneath, the native loop:

```text
ScreenSource (X11)  ─▶ RecordedFrameSource ─┐
                                            ├─▶ media-recorder-webm ─▶ .webm AV1+Opus
MicSource   (cpal)  ─▶ RecordedAudioSource ─┘   (without ffmpeg)
```

```bash
cargo run -p media-recorder-app --release   # X11 ($DISPLAY) or Wayland wlroots ($WAYLAND_DISPLAY)
```

It chooses the screen backend at **runtime**: Wayland (`wlr-screencopy`)
if there is `$WAYLAND_DISPLAY` —with fallback to X11/XWayland if the
compositor doesn't expose it (GNOME/KDE)—, otherwise X11. The microphone
is **optional**: without an input device, it records video-only. The
file comes out as `media-rec-<epoch>.webm` in the current directory.

## The pattern: heavy work outside the Elm loop

Llimphi's Elm loop (`update`/`view`) runs on the UI thread and must not
block. Recording is long work (AV1 encode per frame), so it lives on a
background thread launched with `Handle::spawn`: the closure runs the
loop until the stop flag (`Arc<AtomicBool>`) is raised and, on closing,
**returns** a `Msg::Finished` that the Elm loop receives in `update`.
There is no state shared with the UI except the clonable handle of the
`WebmRecorder` — which is already `Arc<Mutex>` inside.

The stopwatch is refreshed with a `Handle::spawn_periodic` that
dispatches `Msg::Tick` every 500 ms (no-op when not recording).

## Why a crate separate from `media-app`

`media-app` is the **player**; this is the **recorder**. Keeping them
separate avoids the player dragging in the system backends (`x11rb` +
`cpal` with the `screen`/`mic` features of `media-source-capture`) in
its build, and respects the repo's rule: interchangeable UIs over
agnostic cores, one role per crate.
