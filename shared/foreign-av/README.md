# foreign-av — puente de audio/video ajenos

Puente de **audio/video extranjero** (vía ffmpeg) al modelo de frames nativo del
suite. Demux / decode / encode ocurren **detrás de la frontera `shared/foreign-*`**
(regla #4 de `CLAUDE.md`): `media-core` trabaja siempre en frames nativos y no
sabe de ffmpeg. Ingiere cualquier códec; emite AV1/Opus.

## Qué expone

- Decodificación de contenedores/códecs ajenos a frames nativos.
- `transcode_a_av1` — reencode al formato de emisión nativo (AV1/Opus).

## No-objetivos

- No es el reproductor (eso es `media`); es sólo el puente de formato.
- No mete tipos de ffmpeg en el núcleo de las apps.

## Estado (2026-05-31)

### Hecho
- Puente ffmpeg movido a `shared/foreign-av` (cumple regla #4), extraído de media.
- Demux/decode de entrada + `transcode_a_av1` para emitir en formato nativo.

### Pendiente
- Cobertura amplia de códecs/contenedores de entrada (hoy lo que media necesita).
- Streaming/pipeline incremental sin materializar todo en memoria.
- Más tests de ida y vuelta por códec (hoy mínimos).

## Lugar en el repo

`shared/foreign-av` — puente de formato A/V. Consumidor: `media`.
