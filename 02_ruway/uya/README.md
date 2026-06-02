# uya — videollamada soberana

**uya** es "cara/rostro" en quechua: una videollamada es, ante todo, ver caras
en vivo. Es la app nativa que reemplaza a Zoom/Meet (la "web de aplicación" de
tiempo real), sobre el mismo principio del resto de la suite — un `*-core`
agnóstico + frontends Llimphi intercambiables, sin navegador ni JIT de por medio
(ver `APPS-NATIVAS.md`, Tanda 2).

## Crates

| Crate | Rol |
|---|---|
| `uya-core` | Modelo agnóstico: protocolo de cable (`Paquete`), identidad determinista (`ParticipanteId = BLAKE3(nombre)`) y roster de la sala (`Sala`). No sabe de transporte ni de UI. |
| `uya-app` | Pegamento: transporte TCP punto-a-punto (`Enlace`), captura de video (TestCard sintética por defecto; webcam v4l2 tras la feature `camara`) y bus de eventos (`EventoUya`) hacia la UI. |
| `uya-cli` | Nodo headless: ejercita transporte + captura y reporta eventos por consola. Sin GPU. |
| `uya-llimphi` | Cara gráfica (bucle Elm): rejilla de caras (un tile por participante, su último cuadro RGBA con `View::image`) + barra de cámara/micrófono/colgar. |

## Probar

Dos ventanas, una llamada local (uno escucha, el otro conecta):

```bash
UYA_NOMBRE=Alicia UYA_ESCUCHAR=127.0.0.1:7800 cargo run -p uya-llimphi --release
UYA_NOMBRE=Beto   UYA_ESCUCHAR=127.0.0.1:7801 \
  UYA_CONECTAR=127.0.0.1:7800 cargo run -p uya-llimphi --release
```

Headless (sin ventana), para probar la señalización entre procesos:

```bash
UYA_NOMBRE=Alicia UYA_ESCUCHAR=127.0.0.1:7810 cargo run -p uya-cli
UYA_NOMBRE=Beto   UYA_ESCUCHAR=127.0.0.1:7811 UYA_CONECTAR=127.0.0.1:7810 cargo run -p uya-cli
```

Variables: `UYA_NOMBRE` (→ identidad), `UYA_ESCUCHAR` (bind), `UYA_CONECTAR`
(par(es) a conectar al arrancar, coma-separado).

## Estado (MVP)

Anda hoy, end-to-end y feo a propósito:

- ✅ Identidad determinista por nombre (BLAKE3, estilo agora/ayni).
- ✅ Presencia: entrar / salir / estado de medios.
- ✅ **Video en ambos sentidos** (cuadros RGBA enmarcados sobre TCP) + preview local.
- ✅ **Audio en ambos sentidos**: captura de micrófono (`MicSource`, o tono sintético
  con `UYA_TONO=1` sin micro), `Paquete::Audio` PCM `f32`, y una `MezclaRemota` que
  baja a mono + resamplea linealmente al formato del dispositivo + suma a los N pares,
  reproducida por `AudioSink` (cpal).
- ✅ Cámara sintética por defecto (TestCard); webcam real v4l2 con `--features camara` en `uya-app`.
- ✅ Toggle de cámara / micrófono y cuelgue.

## Pendiente (por orden)

1. **Mudar el transporte a card-net** (P2P soberano: relay/dcutr/autonat ya hechos)
   sin tocar `uya-core` ni la UI — sólo otra impl de `Enlace`. Identidad por `agora`.
2. **Compresión** de video y audio (hoy RGBA + PCM crudos; sirve en LAN, no en WAN).
   Reusar `media-encode-av1` (video) y `media-encode-opus` (audio).
3. **Malla N-a-N** automática (hoy es manual: cada par se conecta) y/o un SFU mínimo.
4. **Marcar/conectar desde la UI** (hoy el par se pasa por `UYA_CONECTAR`).
5. **Eco/jitter**: cancelación de eco acústico y un jitter buffer adaptativo (hoy fijo ~1 s).
