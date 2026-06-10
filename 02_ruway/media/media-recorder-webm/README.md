# media-recorder-webm

**Unified recorder** of the domain: records video **and** audio to a single
native `.webm` AV1+Opus. Where `media-recorder-av1` leaves an `.ivf` of video
and `media-recorder-wav` a `.wav` of audio in separate files, this one joins
them into tawasuyu's **native container** — without ffmpeg.

```
FrameSource ─ RecordedFrameSource ─→ media-encode-av1 (rav1e) ─┐
                                                               ├─ media-mux-webm ─→ .webm
AudioSource ─ RecordedAudioSource ─→ media-encode-opus (opus) ─┘   (on stop())
```

## Pattern

Identical to the other recorders: a **clonable** handle (`WebmRecorder`,
`Arc<Mutex<…>>`) plus two transparent wrappers that plug into the pipeline:

- `RecordedFrameSource<S: FrameSource>` — tees each frame to the AV1 encoder.
- `RecordedAudioSource<S: AudioSource>` — tees each block to the Opus encoder.

Both are **no-ops** when the recorder is not armed: the inner never even
notices. It is armed with `start(path)` and closed with `stop()`, which
returns the path and a `RecordingSummary { video_frames, audio_packets, … }`.

```rust
let rec = WebmRecorder::new();
let mut vsrc = RecordedFrameSource::new(mi_video, rec.clone());
let mut asrc = RecordedAudioSource::new(mi_audio, rec.clone());

// A frame must pass before arming (discovers the dimensions).
vsrc.tick(dt, &mut buf);
rec.start("captura.webm")?;
// … run the pipeline …
let (path, resumen) = rec.stop()?;
```

## Codec decisions

- **Video.** Dimensions are discovered from the first frame and frozen at
  `start()` (as in `media-recorder-av1`). rav1e buffers: the packets
  live in RAM and the `.webm` is written entirely on `stop()`.
- **Audio.** The Opus encoder is created **lazily** on the first audio block
  during recording, capturing its sample-rate and channels. Since Opus
  requests exact frames (e.g. 960 samples @ 48 kHz), the incoming audio is
  accumulated in a buffer and drained by complete frames; the remaining
  partial is padded with silence only at `stop()`.
- **Clean degradation.** Opus only admits 8/12/16/24/48 kHz and mono/stereo.
  A format outside that **breaks nothing**: the audio is discarded (reflected
  in `RecordingSummary::audio_packets == 0`) and the `.webm` is left video-only.

## Tests

```bash
cargo test -p media-recorder-webm
```

Round-trip: records synthetic frames + tone → `.webm` → native demux +
decode of both tracks (`media-source-webm`). Includes the degradation case
to video-only with a non-Opus rate (44.1 kHz).
