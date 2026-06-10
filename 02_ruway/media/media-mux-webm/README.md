# media-mux-webm

**Native WebM/Matroska muxer** — the production counterpart of
`media-source-webm`. That crate *demuxes* an AV1+Opus `.webm` into its
tracks; this one *produces* it. With it, tawasuyu closes the full cycle
of the native path **without touching ffmpeg at either end**:

```
RGBA frames ─ media-encode-av1 (rav1e) ─→ AV1 packets
                                            │
                            media-mux-webm ─┴─→  .webm file
                                            │
            media-source-webm (matroska-demuxer) ─→ AV1 + Opus
                                            │
              media-source-av1 (rav1d) ─────┴─→ RGBA frames
```

## Why by hand (no deps)

The WebM container is a bounded subset of **EBML** (Matroska): a grammar
of `ID + VINT(size) + payload` elements. Just as the IVF muxer of
`media-encode-av1` was written byte by byte, here we serialize the EBML
tree without depending on any mux library — tawasuyu owns the format it
produces. The only deps are **dev** ones (round-trip).

## Strategy

Each element is serialized to a `Vec<u8>` and the parent wraps it with
its **already-known** size (no "unknown size"). The file ends up
seekable and the demuxer doesn't have to guess anything. The minimal
structure:

```
EBML header        (DocType "webm")
Segment
├─ Info            (TimestampScale 1ms · Duration · MuxingApp)
├─ Tracks
│  ├─ TrackEntry   V_AV1 · PixelWidth/Height · DefaultDuration (→ fps)
│  └─ TrackEntry   A_OPUS · CodecPrivate (OpusHead) · Sampling/Channels
└─ Cluster(s)      Timestamp + SimpleBlock per packet
```

The video and audio packets are mixed on a **common timestamp axis**
(ms): video derives its time from the framerate; audio, from the samples
per packet. The `SimpleBlock`s store the offset relative to the cluster
as `i16` (±32767 ms); when that range is exceeded a new cluster is
opened.

## API

```rust
use media_mux_webm::{WebmMuxConfig, OpusTrack, mux_webm_file};

let cfg = WebmMuxConfig { width: 320, height: 240, fps_num: 30, fps_den: 1 };

// Video only:
mux_webm_file("v.webm", &cfg, &video_packets, None)?;

// Video + Opus audio:
let audio = OpusTrack { head, sample_rate: 48_000, channels: 2,
                        samples_per_packet: 960, packets: opus_packets };
mux_webm_file("av.webm", &cfg, &video_packets, Some(&audio))?;
```

`video_packets: &[Vec<u8>]` are the raw AV1 packets in presentation
order (the `EncodedPacket::data` from `media-encode-av1`).

## Known limits

- **No AV1 `CodecPrivate`**: the sequence header OBU travels in the
  first packet, so `rav1d` decodes without it; some foreign player might
  require the `AV1CodecConfigurationRecord`. Out of scope today.
- **Keyframe flag**: we mark only the first frame as keyframe (we don't
  inspect the bitstream); it doesn't affect the per-OBU decode, only the
  fine seek. When there is a native Opus encoder, audio will stop
  needing packets provided from outside.

## Tests

```bash
cargo test -p media-mux-webm
```

- Unit: VINT/ID/uint/float EBML encoding + axis order and duration.
- Round-trip: encode AV1 → mux → native demux (`media-source-webm` +
  `matroska-demuxer`) → decode rav1d → dimensions and nº of frames.
