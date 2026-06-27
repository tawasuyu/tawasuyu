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

## Lo que ya está parido (cablear, no inventar)

| Pieza | Dónde | Nota |
|---|---|---|
| Captura de micrófono (cpal + opus) | `02_ruway/media/media-recorder-wav`, `media-encode-opus`, `supay-audio` | la entrada de audio NO es código nuevo |
| Reparto core sync / host corre red+threads | `shuma-agente` + `shuma-agente-host` | la voz respeta el mismo split |
| Acciones se proponen, no se auto-ejecutan | `AccionPropuesta` + `atipay` | la voz no salta este gate |
| Backend configurable por agente | `wawa-config::LlmSettings` | el STT/TTS por agente copia el patrón |

Lo que **no** existe en el repo: STT, TTS, wake-word, VAD. Territorio nuevo,
chico y delimitado.

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

Subcrate nuevo **`shuma-voz`**, molde de `shuma-agente`:

- **`shuma-voz` (core, sync/puro/testeable):**
  - máquina de estados `Dormido → Despierto → Dictando → Cerrado` (+ timeout de
    re-dormida).
  - **política de lectura** (qué `BloqueSalida` se vocaliza) — puro, con tests.
  - clasificador prosódico determinista sobre features de f0.
  - sin sockets, sin `tokio`, sin cpal.
- **host (en `shuma-agente-host` o gemelo `shuma-voz-host`):** corre cpal + VAD
  + engine STT/TTS en threads; dispatcha `Msg` al update Elm.

NO va dentro de `shuma-agente` (no contaminar el núcleo conversacional), ni en
`rimay` (eso es embeddings) ni `pineal` (charts).

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
