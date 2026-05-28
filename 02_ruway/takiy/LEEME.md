# takiy

> `takiy` (quechua: *cantar*). Música del monorepo.

Síntesis, secuenciación y playback. Diseñado para correr en tiempo real (xruns medibles, no hand-wavey) y para ser deterministic cuando se pide (mismo seed → mismo wav). Acopla con el bus `chasqui` para integrarse con otras apps (notebook, dominium, supay).

## Instalación

```sh
cargo run --release -p takiy-app-llimphi
```

## Compatibilidad

- **Linux** — PulseAudio / PipeWire / ALSA.
- **macOS** — CoreAudio.
- **Windows** — WASAPI.
- **Wawa** — driver propio del kernel (cuando esté).

## Crates

| Crate | Rol |
|---|---|
| [`takiy-core`](takiy-core/README.md) | Modelo musical: nota, secuencia, voz. |
| [`takiy-synth`](takiy-synth/README.md) | Synths (osciladores, filtros, envolventes). |
| [`takiy-playback`](takiy-playback/README.md) | Output a audio device. |
| [`takiy-app-llimphi`](takiy-app-llimphi/README.md) | UI Llimphi (secuenciador + síntesis). |

## Consideraciones

- **Latencia es first-class.** El loop de audio respeta el período del device; no se rompe por culpa del UI.
- **Sin VST3/AU.** El catálogo de plugins es el árbol de crates; cualquier nuevo synth se agrega como crate.
- Render offline (no-realtime) para archivos largos: dumpea WAV/FLAC determinista.

## Salud del crate

```sh
./scripts/check-takiy.sh         # check + tests + smoke headless + hash WAV
./scripts/check-takiy.sh fast    # sólo check + smoke (sin tests)
```

El script asegura que:

1. los 5 crates compilan limpios;
2. todos los tests unitarios pasan (≈150);
3. el `example smoke` corre la lógica del editor sin abrir ventana ni device de audio;
4. el WAV producido por el render canónico se mantiene byte-equal contra un hash BLAKE3 registrado (regresión silenciosa de la mezcla → error).

Si el hash cambia con un cambio intencional, actualizar `EXPECTED_BLAKE3` en `takiy-synth/tests/wav_determinism.rs` y justificar el ajuste en el mensaje del commit.
