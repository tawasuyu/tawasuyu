<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# takiy

> `takiy` (runa-simi: *takiy, takikuy*). Monorepupa música.

Síntesis, secuencia, playback. Tiempo-realpi puriy (medible xruns, mana mansuq) + determinista mañakuptin (kikin muhu → kikin WAV). `chasqui` buswan tinkun, huk apps-wan (notebook, dominium, supay) tinkuy.

## Churay

```sh
cargo run --release -p takiy-app-llimphi
```

## Tinkuy

- **Linux** — PulseAudio / PipeWire / ALSA.
- **macOS** — CoreAudio.
- **Windows** — WASAPI.
- **Wawa** — kernel driver (ñawpaqman).

Crateskuna: [`takiy-core`](takiy-core/README.md), [`takiy-synth`](takiy-synth/README.md), [`takiy-playback`](takiy-playback/README.md), [`takiy-app-llimphi`](takiy-app-llimphi/README.md).

## Yuyaykunaq

- **Latencia first-class.** Audio loop device periodo yupaychan; mana UIwan p'akin.
- **Mana VST3/AU.** Plugin catálogo crate sach'a; musuq synth = musuq crate.
- Offline (mana realtime) render hatun archivopaq: determinista WAV/FLAC qatin.
