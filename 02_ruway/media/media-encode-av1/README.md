# media-encode-av1

**Native AV1** encoder (pure-Rust, via [`rav1e`](https://crates.io/crates/rav1e))
from RGBA frames → **IVF** container. It is the **exact counterpart** of
`media-source-av1`: that crate *decodes* AV1; this one *produces* it. It
closes the encode↔decode cycle of tawasuyu's native video format
(`PLAN.md` §6.quinquies) **without touching ffmpeg at either end**.

`rav1e` is the reference AV1 encoder in Rust (Xiph/AOMedia). With
`default-features = false` you get the scalar path (no nasm) — same
criterion as `rav1d` in the decoder; it compiles to WASM and runs on
wawa.

## Why there is no H.264/H.265 encoder

It is not an effort or source-license restriction: it is **patents**.
H.264/H.265/AAC are covered by pools (Via LA / Access Advance) that levy
the *bitstream techniques*, no matter the language you implement them
in. AV1/Opus were designed royalty-free on purpose — that is why they
are the native format and the only ones tawasuyu *produces* in its own
code. H.264 enters/exits through the `shared/foreign-av` (ffmpeg)
bridge, and is transcoded to AV1 on import.

## Usage

```rust
use media_encode_av1::{Av1Encoder, Av1EncoderConfig};

let cfg = Av1EncoderConfig { width: 320, height: 240, fps_num: 30, fps_den: 1, ..Default::default() };
let mut enc = Av1Encoder::new(cfg.clone())?;
let mut packets = Vec::new();
for frame_rgba in &frames {            // each one width*height*4 bytes RGBA
    packets.extend(enc.encode_rgba(frame_rgba)?);  // rav1e has latency: the first frames emit no packet
}
packets.extend(enc.finish()?);         // drains the pipeline
media_encode_av1::write_ivf_file("salida.ivf", &cfg, &packets)?;
```

Or in one shot with `encode_rgba_to_ivf_file(path, cfg, frames)`.

- **Input**: RGBA8 row by row, `width*height*4` bytes — the same format
  the decoder's `FrameSource` spits out.
- **RGBA→YUV420 conversion**: exact inverse of the decoder's (**BT.601
  full range**, no 16-235 scaling); luma is per pixel, chroma averages
  each 2×2 block. This way the round-trip preserves color.
- **`quantizer`** 0..=255 (constant quantizer mode, lower = better
  quality); **`speed`** 0..=10 (higher = faster).
- **IVF output**: `DKIF` header + FourCC `AV01` + dims + framerate + nº
  of frames; per packet 12 bytes (size u32 + timestamp u64) + bitstream.

## Demo

```bash
cargo run -p media-encode-av1 --example gradient --release
# writes /tmp/gradient.ivf — play it back with media-app or media-source-av1
```

## Tests

```bash
cargo test -p media-encode-av1
```

- `rgba_to_yuv_solid_colors` — color matrix (white/red).
- `encodes_to_valid_ivf` — N frames in → N packets out + valid header.
- **`encode_then_decode_preserves_color`** (`tests/roundtrip.rs`) —
  real round-trip: encode here → write IVF → **decode with
  `media-source-av1`** → verify that the central color survives. It
  proves that the native cycle closes without ffmpeg.
