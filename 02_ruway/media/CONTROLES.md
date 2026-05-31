# Controles configurables de media (estilo VLC, más flexible)

> Estado: **plan vivo**. Fase A ✅ · Fase B+C ✅ · Fase D ⏳ (futuro).
> Autoritativo sobre cómo se mapean entradas → acciones en el dominio `media`.

## Problema

Hasta hoy los controles de `media-app` estaban **hardcodeados**: un `enum Msg`
con una variante por acción (`TogglePause`, `VolUp`, `SeekFwd`…), constantes
fijas (`VOLUME_STEP = 0.1`, `SEEK_STEP_SECS = 5`, `SPEED_STEPS = [..]`) y botones
de UI atados a un `Msg` concreto. No había:

- atajos de teclado (VLC se maneja casi todo con teclas),
- forma de reasignar qué tecla hace qué,
- forma de cambiar los pasos (saltar 10 s en vez de 5, volumen de 5 % en vez de 10 %),
- archivo de configuración editable por el usuario.

El PLAN.md (§6.quinquies) sólo preveía widgets visuales
(`llimphi-widget-{transport,timeline,waveform}`), que son contenedores de pintura,
no un sistema de mapeo de entrada. Configurabilidad tipo VLC **no estaba planeada**.

## Principio de diseño (regla #2 del repo)

La lógica de dominio no sabe quién la pinta **ni qué teclas la disparan**. Por eso
el vocabulario de control vive en `media-core` (agnóstico) y la UI sólo traduce su
evento de teclado/click a un `KeyChord`/comando y despacha. Más flexible que VLC
porque los comandos son **parametrizados** (`SeekBy { secs }`, `VolumeBy { delta }`,
`SetSpeed { mult }`) — el mismo comando sirve para salto corto o largo según el
binding, y se puede atar una tecla directamente a "velocidad 1.0×" o "volumen 100 %".

## Arquitectura

```
media-core::control            (agnóstico, serde)
  MediaCommand   — acción semántica parametrizada (12 variantes)
  KeyChord       — tecla normalizada (String) + ctrl/shift/alt, sin winit
  Binding        — { chord, command }
  Keymap         — Vec<Binding> + resolve(&chord) -> Option<&MediaCommand>
  ControlSettings — { volume_step, seek_step_secs, speed_steps, keymap }
                    Default = keymap inspirado en VLC con los pasos por defecto

media-app                       (frontend Llimphi)
  settings_slot()  — OnceLock<ControlSettings> cargado al arrancar
  carga RON desde  $XDG_CONFIG_HOME/gioser/media/controles.ron
                   (si no existe, escribe el default para que el usuario lo edite)
  Msg::Command(MediaCommand)  — único punto de despacho de acciones
  apply_command(cmd)          — ejecuta sobre pause()/volume()/playlist/recorder
  on_key(KeyEvent)            — chord_from_event → keymap.resolve → Msg::Command
  botones                     — construyen el comando con los pasos de settings
```

El formato en disco es **RON** (los enums de Rust se serializan legibles):

```ron
ControlSettings(
    volume_step: 0.1,
    seek_step_secs: 5,
    speed_steps: [0.5, 0.75, 1.0, 1.25, 1.5, 2.0],
    keymap: Keymap(bindings: [
        Binding(chord: KeyChord(key: "Space"),       command: TogglePause),
        Binding(chord: KeyChord(key: "ArrowRight"),   command: SeekBy(secs: 5)),
        Binding(chord: KeyChord(key: "ArrowLeft"),    command: SeekBy(secs: -5)),
        Binding(chord: KeyChord(key: "s", shift: true), command: Snapshot),
        // ...
    ]),
)
```

## Mapa por defecto (inspirado en VLC)

| Tecla        | Comando             | Notas                          |
|--------------|---------------------|--------------------------------|
| `Space`      | TogglePause         |                                |
| `→` / `←`    | SeekBy ±step        | step = `seek_step_secs`        |
| `↑` / `↓`    | VolumeBy ±step      | step = `volume_step`           |
| `n` / `p`    | NextTrack / PrevTrack | como VLC                     |
| `l`          | CycleRepeat         | loop                           |
| `r`          | ToggleShuffle       | random (VLC)                   |
| `]` / `[`    | SpeedStep +1 / −1   | cicla `speed_steps`            |
| `=`          | SetSpeed 1.0×       | reset (más flexible que VLC)   |
| `c`          | ToggleRecord        | capture                        |
| `Shift+s`    | Snapshot            | como VLC (Shift+S)             |

## Fases

- **A ✅ — vocabulario agnóstico en `media-core::control`** + tests (resolve,
  cobertura del default, round-trip RON).
- **B ✅ — wiring en `media-app`**: `Msg::Command`, `apply_command`, `on_key`,
  botones derivan el comando de `settings`. Adiós a los `Msg` por-acción y a las
  constantes hardcodeadas.
- **C ✅ — config persistente**: carga/escritura de `controles.ron` en XDG.
- **D ⏳ — más flexible que VLC** (futuro, no bloqueante):
  - **Paleta de comandos** (overlay con búsqueda fuzzy de todas las acciones) —
    descubribilidad sin memorizar teclas. Encaja en `App::view_overlay`.
  - **Comandos Rhai** (`MediaCommand::Script(nombre)` → snippet con una API de
    reproductor bindeada) — calco del Rhai de las `Transformacion` de pluma; es
    el verdadero "más flexible que VLC".
  - **Layout de controles configurable**: qué tiles/chips aparecen y su orden
    (ya hay base con `TileId` + drag-to-swap; falta persistirlo en settings).
  - **Recarga en caliente** del `controles.ron` (watch del archivo).
  - Reusar el sistema cuando se materialicen los widgets
    `llimphi-widget-{transport,timeline,waveform}` y/o `nahual-video-viewer-llimphi`.
