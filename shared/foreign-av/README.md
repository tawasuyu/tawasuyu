# foreign-av — foreign audio/video bridge

Bridge from **foreign audio/video** (via ffmpeg) to the suite's native frame
model. Demux / decode / encode happen **behind the `shared/foreign-*` boundary**
(rule #4 of `CLAUDE.md`): `media-core` always works in native frames and doesn't
know about ffmpeg. It ingests any codec; it emits AV1/Opus.

## What it exposes

- Decoding of foreign containers/codecs into native frames.
- `transcode_a_av1` — reencode to the native emission format (AV1/Opus).

## Non-goals

- It is not the player (that's `media`); it's only the format bridge.
- It doesn't put ffmpeg types into the apps' core.

## Status (2026-05-31)

### Done
- ffmpeg bridge moved to `shared/foreign-av` (complies with rule #4), extracted from media.
- Input demux/decode + `transcode_a_av1` to emit in native format.

### Pending
- Broad coverage of input codecs/containers (today whatever media needs).
- Streaming/incremental pipeline without materializing everything in memory.
- More roundtrip tests per codec (today minimal).

## Place in the repo

`shared/foreign-av` — A/V format bridge. Consumer: `media`.
