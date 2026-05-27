# takiy

> `takiy` (Quechua: *to sing*). The monorepo's music.

Synthesis, sequencing, playback. Designed to run in real time (measurable xruns, not hand-wavey) and to be deterministic when asked (same seed → same WAV). Couples with the `chasqui` bus to integrate with other apps (notebook, dominium, supay).

## Install

```sh
cargo run --release -p takiy-app-llimphi
```

## Compatibility

- **Linux** — PulseAudio / PipeWire / ALSA.
- **macOS** — CoreAudio.
- **Windows** — WASAPI.
- **Wawa** — kernel driver (when ready).

Crates: [`takiy-core`](takiy-core/README.md), [`takiy-synth`](takiy-synth/README.md), [`takiy-playback`](takiy-playback/README.md), [`takiy-app-llimphi`](takiy-app-llimphi/README.md).

## Considerations

- **Latency is first-class.** The audio loop respects the device period; doesn't break because of UI.
- **No VST3/AU.** Plugin catalog = crate tree; new synth = new crate.
- Offline (non-realtime) render for long files: dumps deterministic WAV/FLAC.
