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

## Estado (2026-05-31)

### Hecho

- **5 crates** (`takiy-core/synth/playback/midi/app-llimphi`) compilando limpio, ≈150
  tests unitarios, determinismo verificado por hash BLAKE3 del WAV canónico
  (`check-takiy.sh` + `example smoke` headless).
- **Secuenciador Llimphi** con UX de edición madura: drag-to-move de notas, drag-resize,
  audition, scroll vertical, tonalidad consciente (key+scale en UI, F6), snap a la
  tonalidad (F11/Alt+K).
- **Automation per-track** (F9): curvas de vol/pan con visual, drag de dots, click en
  curva inserta, right-click borra, render sample-accurate.
- **Efectos de bus/master** (F8): delay + reverb Schroeder de master.
- **Export WAV desde UI** (F4, Ctrl+R) + render offline determinista (F10 + CI script).
- **Menús** (lote 3): menú principal + menús contextuales.
- Refactors regla #1: split del `model.rs` del lib en `model/` y del `main.rs` del
  binario en módulos bin-only.

### Pendiente

- **Acoplamiento al bus `chasqui`** para integrarse con otras apps (notebook, dominium,
  supay) — diseñado, aún no cableado.
- **Driver de audio en Wawa** (cuando exista el kernel driver).
- **`takiy-midi`**: ampliar import/export MIDI más allá del soporte actual de GENMIDI/synth.
