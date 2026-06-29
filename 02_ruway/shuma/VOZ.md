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
| Config de voz global del SO | `wawa-config::VozSettings` (`ai.voz`) | **hecho** — STT/TTS/llamado/wake editables en wawa-panel (sección «Voz»); los hosts la leen para armar `VozConfig` + `OpcionesEscucha` |

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
- **F1 (compuerta dedicada, hecho):** `rimay-voz-core::wake` — trait
  `DetectorLlamado` (¿esta utterance suena al llamado?) **antes** del STT. Si no
  matchea, el audio **no se transcribe** (con STT de nube, no sale de la
  máquina) — cierra el agujero de privacidad del "transcribe-todo" de F0. Default
  sin modelo: `DetectorPlantilla`, *speaker-dependent* — se enrola con unas
  grabaciones del llamado y compara por **DTW** sobre rasgos baratos
  (log-energía + cruces por cero, sin FFT). El `Lazo` la consulta sólo estando
  **dormido** (despierto/dictando no gatea, dictás libre). Un wake-word neuronal
  *speaker-independent* (openWakeWord ONNX) entra como otra impl del trait, sin
  tocar el lazo. **Honestidad:** los tests certifican el *mecanismo*
  (idéntico-a-la-plantilla dispara, distinto no; el gateo corta el STT), no la
  precisión real sobre «shuma» — eso se afina con el enrolado en metal. Falta la
  **UX de enrolado** (grabar «shuma» N veces) en la app.

## Forma del código (Regla: un dominio = un crate raíz + subcrates)

Familia **`rimay-voz`** en el dominio `rimay`, molde de `rimay-verbo`:

- **`rimay-voz-core` (hecho, sync/puro/testeable):**
  - traits `Transcriptor` (STT) + `Locutor` (TTS) + `Audio`/`Transcripcion` —
    el contrato model-agnostic (gemelo del `Provider` de verbo).
  - máquina de estados `Dormido → Despierto → Dictando` (+ detección del
    llamado, + timeout de re-dormida).
  - **VAD + segmentador** (`vad`): trait `DetectorVoz` (¿voz en este frame? →
    prob) con default `DetectorEnergia` (RMS, sin modelo) — Silero entra como
    otra impl del trait. `Segmentador` puro convierte el flujo de probs en
    bordes de utterance (`PulsoVad::{Inicio,Sigue,Fin}`) con debounce de
    arranque + colgado (hangover). `Vad` junta detector+segmentador+acumulación
    y entrega el `Audio` al cerrar (recortando el silencio del colgado), listo
    para el STT.
  - **Wake-word** (`wake`): trait `DetectorLlamado` + default `DetectorPlantilla`
    (DTW sobre rasgos baratos, enrolable, sin modelo). La compuerta F1 — ver
    §Wake-word.
  - **política de lectura** discriminada: el consumidor mapea su tipo de bloque
    → `TipoBloque` (sólo la prosa se vocaliza).
  - clasificador prosódico determinista sobre features de f0.
  - sin sockets, sin `tokio`, sin cpal.
- **`rimay-voz-mock` (hecho):** STT/TTS deterministas sin modelo (CI/demos).
- **`rimay-voz` (hecho, fachada):** re-exporta core+mock, constructores
  `stt_mock`/`tts_mock`, convención del socket `voz.sock`. Demos canónicos:
  `cargo run -p rimay-voz --example escucha_mock` (lazo desde transcripts) y
  `--example pipeline_vad` (upstream completo: frames → VAD → STT → máquina,
  certificado por texto).
- **`VozConfig` (hecho, selector híbrido):** el híbrido configurable, gemelo de
  `pluma-llm::from_env`. STT y TTS se eligen por separado (`Backend::{Mock,
  Local,Nube}`), vía `RIMAY_VOZ_STT`/`RIMAY_VOZ_TTS` (`"local"`,
  `"nube:openai:whisper-1"`…). `construir_stt`/`construir_tts` (+ `_o_mock` con
  fallback). **El daemon es el brazo local, no compite con la nube.**
- **`rimay-voz-nube` (hecho, rama Nube del híbrido):** backend HTTP shape
  OpenAI. STT → `POST /audio/transcriptions` (Whisper): el PCM se empaqueta como
  WAV en memoria y sube por `multipart`. TTS → `POST /audio/speech` con
  `response_format:"pcm"` (16-bit LE mono 24 kHz), decodificado directo a
  `Audio`. `TranscriptorNube`/`LocutorNube` con `openai_from_env()` (lee
  `OPENAI_API_KEY`) + `con_modelo`/`con_voz`; `base` configurable → sirve
  cualquier proxy OpenAI-compatible. `VozConfig` cablea la rama `Nube{openai}`;
  sin credencial erra explícito (ningún constructor hace red). Certificado por
  codec WAV↔PCM y manejo de error, sin tocar red (11 tests entre crate+fachada).
- **`rimay-voz-daemon` + `rimay-voz-daemon-bin` (hecho, brazo local):** daemon
  que carga el par STT+TTS una vez y lo sirve por socket Unix; el `DaemonClient`
  lo consume desde otro proceso cumpliendo **ambos** traits (`Transcriptor` +
  `Locutor`), indistinguible de un backend local. Calcado de
  `rimay-verbo-daemon`: wire postcard con prefijo de largo, transporte
  Unix-socket / TCP-loopback por `cfg`, reintento corto ante transitorios,
  `serve_with_shutdown`. El daemon sirve dos traits a la vez (un proceso puede
  cargar whisper + piper, o mock en el lado sin backend real). `VozConfig` cablea
  `Backend::Local` → `DaemonClient::connect(socket)` (override `socket` o
  `voz.sock` por convención); sin daemon, `_o_mock` cae a mock. Binario
  `voz-daemon` (`--socket/--stt/--tts`, hoy sólo mock). Certificado por
  round-trip sobre socket Unix real (10 tests: STT/TTS/handshake/2-clientes/
  ping/shutdown/daemon-ausente).
- **`rimay-voz-{whisper,piper,…}` (falta):** backends **locales** reales que
  reemplazan el mock dentro del daemon — entran como variantes del `--stt`/
  `--tts` del binario, sin tocar protocolo ni cliente.
- **`rimay-voz-host` (hecho, host de captura):** corre el micrófono y empuja
  los frames por el lazo `VAD → STT → Maquina`, emitiendo `EventoEscucha`
  (`Escuchando`/`Desperto`/`Dictar`/`SeDurmio`) que la app dispatcha como `Msg`.
  **Es IA general (oír) → vive en `rimay`, no en shuma** (misma corrección que
  STT/TTS: shuma sólo cablea). Dos capas: el `Lazo` puro (muestras `i16` mono →
  framing → VAD → STT → máquina + `tick`), testeable sin micrófono; y el driver
  `escuchar()` detrás de la feature **`microfono` (ON por default, apagable con
  `--no-default-features`)**, que abre cpal en un **hilo dedicado** (el `Stream`
  es `!Send`), prepara el audio (`a_mono` + `Remuestreador` lineal con estado +
  `a_i16`, reusando la captura de `media-source-capture/mic`) y alimenta el
  `Lazo` desde una task async, emitiendo eventos por canal. **Palabra de llamada
  configurable** y **compuerta wake-word (F1) opcional** vía `OpcionesEscucha`
  (`escuchar_con`); el `Lazo` gatea el STT con el `DetectorLlamado` estando
  dormido. Certificado por texto: 15 tests (lazo: ruido/llamado/cola/re-dormida/
  silencio + gateo wake acepta/rechaza/no-gatea-despierto; prep: downmix/
  remuestreo/clamp). Demos: `escuchar_microfono` (en metal) y `wake_gateo`
  (gateo F1 sin micrófono, por texto). Lo único que quedará «de shuma»: mapear
  `shuma_agente::BloqueSalida` → `rimay_voz::TipoBloque` y dispatchar los
  eventos. Sin micrófono real, el lazo ya está demostrado en
  `rimay-voz/examples/pipeline_vad`.

## Dependencias candidatas

- **VAD:** la *lógica* (segmentación + trait `DetectorVoz`) ya vive en
  `rimay-voz-core::vad`, con default de energía. Para robustez, una impl Silero
  (`voice_activity_detector`, ONNX) entra como otro `DetectorVoz`; alt.
  `webrtc-vad`.
- **STT local:** `whisper-rs` (bindings whisper.cpp). **Nube:** ✅ aterrizado en
  `rimay-voz-nube` (shape OpenAI sobre `reqwest`) — el gap de "pluma-llm no tiene
  STT" se resolvió con fachada propia, no estirando `ChatClient`.
- **TTS local:** piper / espeak-ng. **Nube:** API por agente.
- **Captura:** reusar cpal vía los crates de `media/`.

## Gaps conocidos

- ~~`pluma-llm` no modela STT/TTS~~ — resuelto: la rama "nube" tiene su propia
  fachada (`rimay-voz-nube`), no se estiró `ChatClient`.
- Always-on + privacidad: el indicador de escucha debe ser **visible siempre**
  en el chasis (diente/rail), no oculto.
- Barge-in (hablar encima del TTS) queda para después de F0.
