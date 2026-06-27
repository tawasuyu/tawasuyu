# VOZ.md — voz manos-libres en shuma

> Propuesta 2026-06-27. Estado: **borrador para discusión** — nada comprometido.
> Cada pieza cita el artefacto real sobre el que se monta. Gemelo de
> `INTELIGENCIA.md`: misma doctrina (*determinista primero, modelo opcional
> después*; *el shell propone, el usuario acepta/habla, nunca es piloto*).

## Tesis

La voz es **otra superficie de E/S opt-in y rotulada**, no un asistente que
toma el mando. Tres capacidades separables — no son una:

1. **Dictar (STT):** voz → texto al input. Mismo molde que `:?`: el host corre
   el engine en un thread y dispatcha `Msg` al update Elm.
2. **Leer discriminado (TTS):** la doctrina prohíbe leer todo. Se lee **sólo
   los `BloqueSalida::Texto`** del agente (prosa), **nunca** código ni volcados
   de stdout, y sólo con toggle por-agente o tecla *«leéme esto»*.
3. **Entonación:** dos capas. (a) **determinista/barata** — contorno de f0 /
   subida final → ¿pregunta vs orden?, ¿urgencia? como *pista* de intención.
   (b) intención emocional rica → modelo, opt-in. No se promete (a) como magia.

Wake-word manos-libres es el **gate** de (1), no una cuarta capacidad.

## Decisiones tomadas (2026-06-27)

- **Engine híbrido:** wake-word + VAD **siempre local**; STT/TTS **configurable
  local o nube por agente** — espeja `LlmSettings` por agente que ya existe en
  `wawa-config` / `shuma-agente`.
- **Manos libres directo** (no push-to-talk primero). Primer corte sin entrenar
  modelo: **VAD-gated STT + match del llamado** (ver §Wake-word).

## STT/TTS son IA GENERAL → van en `rimay`, no en shuma

Corrección de fondo (2026-06-27): el habla no es de shuma. `rimay` (quechua
*hablar*) es el dominio de «lo que quiere decir algo»; ya hospeda
`rimay-verbo` (embeddings) con el patrón canónico de la suite — **fachada +
trait + mock fallback + daemon que carga el modelo una vez por socket**. La voz
es el gemelo: vive en **`rimay-voz`** y *cualquier* app la consume (shuma,
mirada, pluma). **shuma sólo cablea** — no aloja nada de IA general.

## Lo que ya está parido (cablear, no inventar)

| Pieza | Dónde | Nota |
|---|---|---|
| Contrato STT/TTS + lógica de escucha | `00_unanchay/rimay/rimay-voz-core` | **hecho** — traits `Transcriptor`/`Locutor` + máquina/lectura/prosodia |
| Backend mock determinista | `00_unanchay/rimay/rimay-voz-mock` | **hecho** — STT/TTS sin modelo, para CI/demos |
| Patrón fachada+daemon a copiar | `rimay-verbo` (embeddings) | molde exacto para el daemon de voz |
| Captura de micrófono (cpal + opus) | `02_ruway/media/media-recorder-wav`, `media-encode-opus`, `supay-audio` | la entrada de audio NO es código nuevo |
| Acciones se proponen, no se auto-ejecutan | `AccionPropuesta` + `atipay` | la voz no salta este gate |
| Backend configurable por agente | `wawa-config::LlmSettings` | el STT/TTS por agente copia el patrón |

Lo que **falta**: los backends reales (whisper/piper/nube), el daemon, y el
host que corre cpal+VAD.

## Pipeline

```
cpal frames ─► VAD (Silero, local, det.) ─► [hay voz] ─► STT del fragmento
                                                            │
              ┌──── ¿el texto arranca con el llamado? ──────┘
              │ sí                          │ no
              ▼                             ▼
     Despierto: dictado al input      descartar (nada sale de la máquina)
              │
              ├─► texto al input  (mismo Msg que el ghost/`:?`)
              └─► f0/contorno ─► pista de intención (pregunta/orden/urgencia)
```

Nada pesado corre hasta que el VAD ve voz; nada sale de la máquina hasta que
matchea el llamado.

## Wake-word — corte honesto por fases

- **F0 (manos libres ya, sin entrenar):** VAD local siempre-encendido. Cuando
  hay voz, STT sobre ese fragmento; si el transcript **empieza con el llamado**
  (`"shuma"` u otro quechua — Regla 6, *no* "Alexa"), se entra en `Despierto`.
  Cuesta más CPU (STT por utterance) pero en desktop es aceptable y es cero
  modelo nuevo.
- **F1 (bajar CPU):** detector dedicado (openWakeWord ONNX / KWS chico) que
  reemplaza el "STT-then-match" — sólo dispara STT tras el llamado.

## Forma del código (Regla: un dominio = un crate raíz + subcrates)

Familia **`rimay-voz`** en el dominio `rimay`, molde de `rimay-verbo`:

- **`rimay-voz-core` (hecho, sync/puro/testeable):**
  - traits `Transcriptor` (STT) + `Locutor` (TTS) + `Audio`/`Transcripcion` —
    el contrato model-agnostic (gemelo del `Provider` de verbo).
  - máquina de estados `Dormido → Despierto → Dictando` (+ detección del
    llamado, + timeout de re-dormida).
  - **política de lectura** discriminada: el consumidor mapea su tipo de bloque
    → `TipoBloque` (sólo la prosa se vocaliza).
  - clasificador prosódico determinista sobre features de f0.
  - sin sockets, sin `tokio`, sin cpal.
- **`rimay-voz-mock` (hecho):** STT/TTS deterministas sin modelo (CI/demos).
- **`rimay-voz` (hecho, fachada):** re-exporta core+mock, constructores
  `stt_mock`/`tts_mock`, convención del socket `voz.sock`. Demo canónico del
  lazo completo: `cargo run -p rimay-voz --example escucha_mock`.
- **`VozConfig` (hecho, selector híbrido):** el híbrido configurable, gemelo de
  `pluma-llm::from_env`. STT y TTS se eligen por separado (`Backend::{Mock,
  Local,Nube}`), vía `RIMAY_VOZ_STT`/`RIMAY_VOZ_TTS` (`"local"`,
  `"nube:openai:whisper-1"`…). `construir_stt`/`construir_tts` (+ `_o_mock` con
  fallback). Local/Nube erran hasta que los backends aterricen. **El daemon es
  el brazo local, no compite con la nube.**
- **`rimay-voz-{whisper,piper,…}` + `rimay-voz-daemon` (falta):** backends
  reales + daemon que carga el modelo una vez por socket (copiar
  `rimay-verbo-daemon`).
- **host de shuma (falta, en `shuma-agente-host`):** corre cpal + VAD; consume
  `rimay-voz` y dispatcha `Msg` al update Elm. Lo único «de shuma»: mapear
  `shuma_agente::BloqueSalida` → `rimay_voz::TipoBloque`.

## Dependencias candidatas

- **VAD:** `voice_activity_detector` (Silero ONNX) — robusto; alt. `webrtc-vad`.
- **STT local:** `whisper-rs` (bindings whisper.cpp). **Nube:** backend nuevo —
  hoy `pluma-llm` NO tiene STT; es un gap a abrir si se quiere nube.
- **TTS local:** piper / espeak-ng. **Nube:** API por agente.
- **Captura:** reusar cpal vía los crates de `media/`.

## Gaps conocidos

- `pluma-llm` no modela STT/TTS — la rama "nube" del híbrido necesita fachada
  nueva (mismo espíritu que `ChatClient`).
- Always-on + privacidad: el indicador de escucha debe ser **visible siempre**
  en el chasis (diente/rail), no oculto.
- Barge-in (hablar encima del TTS) queda para después de F0.
