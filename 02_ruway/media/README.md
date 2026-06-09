# media

*Leé esto en español: [LEEME.md](LEEME.md).*

```
   ╭───────╮
   │ ◉   ◉ │
   │   ─   │   media · the suite's audio/video domain
  ╭┤       ├╮  ─────────────────────────────────────────
  │└───────┘│  player, decoders, visualizers, recorder
  ╰╤───────╤╯  · its mascot is a sock ·
   │       │
   ╰───────╯
```

Audio + video for the suite. It lives in `02_ruway/` (DO) because it
produces and moves frames; it doesn't decide *what* plays (that's for the
apps above) nor *how* it's rendered (that's Llimphi's job).

The name deliberately steps outside quechua: "media" is the universal Latin
root of the formats this domain handles (mp4, mp3, wav, srt) — no quechua
word covered both senses without colliding with other domains (`mirada` is
already vision, `takiy` already song). Mascot: a sock — it stores things,
gets lost, keeps you warm.

## The native stack, in one sentence

tawasuyu both **plays and produces** its own patent-free A/V format —
**AV1 video + Opus audio in WebM**, encoded, muxed, demuxed and decoded in
pure Rust, with ffmpeg allowed only at the border as a foreign-format
bridge (`shared/foreign-av`, hard rule #4).

## Crates

| crate | role |
|---|---|
| `media-core` | `FrameSource` / `AudioSource` traits + common primitives (probe, spectrum, pause, volume, mixer, switcher, waterfall, levels, subtitles) |
| `media-source-wav` | WAV (hound) → `AudioSource + Seekable` |
| `media-source-mp3` | MP3 (symphonia, `mp3` feature) → `AudioSource + Seekable` |
| `media-source-flac` | **native FLAC** (pure Rust) → `AudioSource + Seekable`. Patent-free lossless; the lossless twin of Opus. |
| `media-source-opus` | **native Opus** (pure Rust, opus-wave) over Ogg → `AudioSource + Seekable`. tawasuyu's native audio format, twin of AV1 video. |
| `media-encode-opus` | **native Opus encode**: PCM f32 → Opus packets + `OpusHead`. tawasuyu PRODUCES its native audio. Feeds the Opus track of `media-mux-webm`. Round-trip verified. |
| `media-source-vorbis` | **native Vorbis** (pure Rust) over Ogg → `AudioSource + Seekable`. The classic patent-free lossy; third of the Opus/FLAC/Vorbis trio. |
| `media-source-webm` | **native Matroska/WebM demux**: a `.webm`/`.mkv` AV1+Opus feeds the native decoders → 100% pure-Rust playback. |
| `media-mux-webm` | **native WebM/Matroska mux** (hand-written EBML, **zero deps**): AV1 packets (+ optional Opus) → `.webm`, no ffmpeg. Round-trip mux→demux→decode verified. |
| `shared/foreign-av` | MP4/WebM/MKV/MOV/AVI/FLV via ffmpeg subprocess — one process per file (audio + video over dup'ed fds 3/4). Lives in `shared/foreign-*` (hard rule #4). Also offers `transcode_a_av1` (ingest into the native format). |
| `media-source-av1` | **native AV1** (pure Rust, rav1d) over IVF → `FrameSource + Seekable`. tawasuyu's native video format. |
| `media-encode-av1` | **native AV1 encode** (pure Rust, rav1e): RGBA frames → IVF. tawasuyu PRODUCES its native video. Round-trip verified. Feeds `media-mux-webm`. |
| `media-source-capture` | **live capture** (INPUT side): v4l2 camera · **X11 + Wayland screen** (→ `FrameSource`) + **cpal microphone** (→ `AudioSource`). Agnostic cores (`LiveSource`/`LiveSink`, audio ring) + pure pixel-format conversion. Four opt-in backends: `camera`, `screen` (x11rb), `wayland` (wlr-screencopy), `mic` (cpal, 48 kHz Opus-ready). All feed `media-recorder-webm`. |
| `media-source-gif` | animated GIF (image) → `FrameSource + Seekable` |
| `media-source-image` | PNG/JPEG/WebP/BMP/TIFF (image) → `FrameSource` (single frame) |
| `media-audio-cpal` | realtime sink over cpal (default output device) |
| `media-recorder-wav` | taps the audio stream to WAV (hound, PCM 16) — transparent wrapper |
| `media-recorder-av1` | taps the video stream to native AV1 `.ivf` — video counterpart of the WAV recorder. |
| `media-recorder-webm` | **unified recorder**: tees video (`FrameSource`→AV1) + audio (`AudioSource`→Opus) into a single muxed `.webm` on `stop()`. Non-Opus sample rates degrade to video-only. Record→playback round-trip verified, no ffmpeg. |
| `media-app` | Llimphi player with visualizers; `examples/analyze.rs` analyzes offline |
| `media-recorder-app` | Llimphi **screen recorder** (Rec/Stop button + timer): `ScreenSource`+`MicSource` → native `.webm` AV1+Opus. Records on a background thread via `Handle::spawn`. |

The `media-source-*` crates are leaves: they depend only on `media-core`
and their decoder. The wrappers (pause, volume, recorder, probe) compose
over any `AudioSource` as trait objects — the sink chain stays layered.

## Typical audio chain (what `media-app` builds)

```text
inner producer (Wav / Mp3 / FfmpegAudio / Tone)
  ↓ Box<dyn AudioSource + Send>
SharedAudio          ← Arc<Mutex<Playlist>>, exposes Seekable to the UI
  ↓
PausableAudio        ← silences when Pause::is_paused()
  ↓
VolumeAudio          ← linear gain per sample
  ↓
RecordedAudioSource  ← tees into the WavWriter when armed
  ↓
ProbedAudioSource    ← tees into the ring buffer for the visualizers
  ↓
cpal sink
```

Every layer preserves the format (sample rate, channels). Order matters:
**Pause** sits below Volume so pausing silences before the gain (the
recorder records the silence, just like the sink); **Probe** sits on top so
the visualizer reflects exactly what plays (post-pause, post-volume,
post-mix). To mix several sources, give each its own `VolumeAudio` into a
`MixerAudio` that sums and clamps to [-1, 1].

## Video — ffmpeg as a bridge

`shared/foreign-av` is the only crate in the workspace that knows `ffmpeg`
exists. It spawns ONE subprocess per file decoding audio AND video
simultaneously; streams exit over extra fds (3 and 4) wired via
`pre_exec` + `dup2`. A clonable `MediaSession` coordinates —
`FfmpegVideoSource` / `FfmpegAudioSource` are views that grab fresh pipes
when the session respawns on seek. Unix-only for now.

## Visualizers

`media-core` provides the primitives; apps paint them wherever they want.

| primitive | input | output |
|---|---|---|
| `AudioProbe` | samples per callback | ring snapshot of the latest span (chronological) |
| `Levels` | snapshot | smoothed peak + RMS |
| `Spectrum` | snapshot + log bands | per-band magnitudes (Goertzel) |
| `Waterfall` | snapshot + log bands + rows | 2D history grid (newest-first) |
| `SubtitleTrack` | SRT **+ WebVTT** parser (autodetected) + timestamp query | active cue (synced to the seekable handle) |

All use immediate-attack + exponential-release where it applies, so the
bars don't flicker between frames. The WebVTT parser strips headers,
`NOTE`/`STYLE`/`REGION` blocks, cue ids, inline tags and common HTML
entities — plain text out.

## Playlist + transport

`media-app` keeps a `Playlist` with manual prev/next, auto-advance,
`RepeatMode {Off, One, All}` and optional shuffle (Fisher-Yates over its
own xorshift64, no rand dep). Tracks can be
`LoadedTrack {Wav | Mp3 | FfmpegAudio}` — prev/next cycle, seek works
modulo duration, speed persists across switches.

## Environment variables (media-app)

| variable | effect |
|---|---|
| `MEDIA_WAV=path` | use a WAV as the main audio source |
| `MEDIA_MP3=path` | use an MP3 (WAV wins if both are set) |
| `MEDIA_PLAYLIST=m3u` | load a simple m3u list (one file per line, `#` = comment, paths relative to the file) |
| `MEDIA_SRT=path` / `MEDIA_VTT=path` | load SRT or WebVTT subtitles (autodetected) synced to playback |
| `MEDIA_MUTE=1` | don't open the cpal sink (visualizers keep running, no sound) |
| `MEDIA_MIX_TONE=g` | overlay an A4 tone at gain `g` (0..1) via MixerAudio |

The first positional arg is the video; the extension picks the source
(`.gif` → animation, `.png/.jpg/.webp/.bmp/.tiff` → still image,
`.mp4/.webm/.mkv/.mov/.avi/.flv/.m4v/.ogv` → real video via ffmpeg). For a
video file, audio and video come from the SAME ffmpeg.

## Offline demo (no cpal, no Llimphi)

```bash
cargo run -p media-app --example analyze --release -- track.mp3
# writes track-waterfall.png next to the file
```

The example composes `WavSource`/`Mp3Source` + `Waterfall` + image in ~150
lines — the reference for embedding the domain elsewhere (notebook kernel,
batch agent, CI).

## Notebook integration

`00_unanchay/pluma/pluma-notebook-kernel-media` exposes the domain in pluma
as a reactive kernel. Language `media`; each cell is `key = value`. Ops:
`info`, `levels`, `waveform`, `waterfall`. The kernel returns
`OutputPayload::Image{png}` or `Text` per op — the notebook DAG works as an
audio patch-bay.

## Tests

```bash
cargo test -p media-core              # pure primitives (Spectrum, Levels, AudioProbe, Mixer, Waterfall, Subtitles)
cargo test -p media-recorder-wav      # recording round-trip
cargo test -p media-recorder-webm     # unified recorder: record .webm AV1+Opus → native playback
cargo test -p media-source-capture    # live capture: pure conversion + audio ring + camera/screen→.webm loop (no hardware)
cargo test -p media-encode-opus       # Opus encode + encode→decode round-trip
cargo test -p media-mux-webm          # low-level EBML + mux→demux→decode round-trip
cargo test -p foreign-av              # parse + clamp (without invoking ffmpeg)
cargo test -p pluma-notebook-kernel-media   # mini-DSL parse
```

The pure crates (core, recorder, kernel) never touch the sound device nor
the ffmpeg binary — they run in CI without hardware.
