# media-recorder-av1

Captures the frames of any `FrameSource` to a **native AV1** `.ivf`
(via `media-encode-av1`). It is the **video counterpart** of
`media-recorder-wav`: that one tees the audio to WAV, this one tees the
frames to AV1 — without ffmpeg.

`Av1Recorder` is a clonable handle (`Arc<Mutex<…>>`); `RecordedFrameSource`
wraps it over a `FrameSource` and encodes each frame **if it is armed**.
Unarmed, the wrapper is a transparent no-op — the same composition
pattern as `RecordedAudioSource`.

## Usage

```rust
use media_recorder_av1::{Av1Recorder, RecordedFrameSource};
use media_core::FrameSource;

let rec = Av1Recorder::new();                       // 30fps, medium quality by default
let mut src = RecordedFrameSource::new(mi_fuente, rec.clone());

let mut buf = Vec::new();
src.tick(dt, &mut buf);                             // one frame to discover dimensions
rec.start("captura.ivf")?;                          // freezes dims + arms the encoder
for _ in 0..frames { src.tick(dt, &mut buf); }      // each frame is encoded on the fly
let (path, n) = rec.stop()?;                        // drains the pipeline + writes the .ivf
```

The resulting `.ivf` is played back by `media-source-av1` (or
`media-app`) without touching ffmpeg again.

## Codec particularities

- **Fixed dimensions**: discovered from the first frame that passes
  through the wrapper (like sr/channels in the audio recorder) and
  frozen at `start()`. Frames of another size are dropped
  (`dropped_frames()`).
- **fps declared, not measured**: `Av1RecorderSettings { fps_num,
  fps_den, quantizer, speed }` fixes the cadence that goes to the IVF
  header; it is not inferred from the real `dt`.
- **Deferred close**: rav1e buffers (lookahead), so the packets pile up
  in RAM and the `.ivf` is written whole at `stop()`, once the frame
  count is known. For very long recordings it would be better to write
  incrementally (num_frames=0); today simplicity is prioritized.
- The `tick` holds the lock while it encodes — same tradeoff as hound's
  sync writer in `media-recorder-wav`.

## Tests

```bash
cargo test -p media-recorder-av1
```

- states (`NoFormatYet` / `NotArmed`) + transparency when unarmed.
- **`record_then_decode_preserves_color`** (`tests/roundtrip.rs`) — tee
  of a solid `FrameSource` → `.ivf` → **decode with `media-source-av1`**
  → verify the color. The capture path closes without ffmpeg.
