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
| `uya-app` | Pegamento: transporte P2P soberano sobre **card-net** (`BrahmanNet`/libp2p, con relay/dcutr/autonat), captura de video (TestCard sintética por defecto; webcam v4l2 tras la feature `camara`), audio (mic→mezcla→`AudioSink`) y bus de eventos (`EventoUya`) hacia la UI. |
| `uya-cli` | Nodo headless: ejercita transporte + captura y reporta eventos por consola. Sin GPU. |
| `uya-llimphi` | Cara gráfica (bucle Elm): rejilla de caras (un tile por participante, su último cuadro RGBA con `View::image`) + barra de cámara/micrófono/colgar. |

## Probar

El transporte es libp2p: cada nodo imprime al arrancar su **multiaddr dialable**
(con `/p2p/<peerid>`). Para llamar a alguien, pasale esa dirección por
`UYA_CONECTAR`. Dos ventanas:

```bash
# 1) Arrancá Alicia y copiá la dirección dialable que imprime:
UYA_NOMBRE=Alicia UYA_ESCUCHAR=/ip4/0.0.0.0/tcp/7800 cargo run -p uya-llimphi --release
#    → "uya: dialable en /ip4/127.0.0.1/tcp/7800/p2p/12D3KooW..."

# 2) Conectá Beto a esa dirección:
UYA_NOMBRE=Beto UYA_ESCUCHAR=/ip4/0.0.0.0/tcp/7801 \
  UYA_CONECTAR=/ip4/127.0.0.1/tcp/7800/p2p/12D3KooW... cargo run -p uya-llimphi --release
```

Headless (sin ventana / sin GPU), mismo flujo con `uya-cli` (reporta cuadros y
muestras de audio recibidas):

```bash
UYA_NOMBRE=Alicia UYA_ESCUCHAR=/ip4/0.0.0.0/tcp/7810 cargo run -p uya-cli
UYA_NOMBRE=Beto UYA_CONECTAR=<dirección dialable de Alicia> cargo run -p uya-cli
```

Variables: `UYA_NOMBRE` (→ identidad), `UYA_ESCUCHAR` (multiaddr de escucha,
default `/ip4/0.0.0.0/tcp/0`), `UYA_CONECTAR` (multiaddr(s) dialable(s),
coma-separado), `UYA_TONO=1` (tono sintético si no hay micrófono).

**Entrar por nombre de sala** (en vez de pegar direcciones): todos con la misma
`UYA_SALA`, sembrando el DHT con un rendezvous (`UYA_BOOTSTRAP`, p. ej. el primer
nodo). Quien quiera puede dialear a cualquiera de la sala; la malla converge sola.

```bash
# Anfitrión/rendezvous (anotá su dirección dialable):
UYA_NOMBRE=Ana UYA_SALA=oficina UYA_ESCUCHAR=/ip4/0.0.0.0/tcp/7880 cargo run -p uya-cli
# Los demás: misma sala, bootstrap al anfitrión, SIN pegar direcciones:
UYA_NOMBRE=Bea UYA_SALA=oficina UYA_BOOTSTRAP=<dirección de Ana> cargo run -p uya-cli
```

## Estado (MVP)

Anda hoy, end-to-end y feo a propósito:

- ✅ **Transporte P2P soberano** sobre card-net (`BrahmanNet`/libp2p): Noise +
  Yamux + relay/dcutr/autonat — el mismo nodo que ayni/minga/agora, así que
  sirve cruzando NAT, no sólo en LAN. Streams multiplexados por `/uya/transporte/1.0.0`.
- ✅ **Identidad determinista por nombre**: el `ParticipanteId` de app y el
  keypair ed25519 del transporte derivan ambos de `BLAKE3(nombre)`, así que el
  **PeerId (y la multiaddr dialable) es estable entre arranques** — comparten raíz.
- ✅ Presencia: entrar / salir / estado de medios.
- ✅ **Video en ambos sentidos** + preview local. Comprimido con **JPEG por
  cuadro** (MJPEG): ~40× menos bytes que RGBA crudo (192×144: 110 KB → ~2,8 KB),
  sin estado inter-cuadro (baja latencia). El preview local va sin comprimir.
- ✅ **Audio en ambos sentidos**, comprimido con **Opus** (~57×: 20 ms = 3840 B PCM
  → ~67 B): captura de micrófono (`MicSource` a 48 kHz, o tono sintético con
  `UYA_TONO=1`), downmix + resampleo a 48 kHz mono, encode Opus por frame de 20 ms;
  en recepción un `OpusDecoder` por par decodifica a PCM y una `MezclaRemota`
  resamplea al dispositivo + suma a los N pares, reproducida por `AudioSink` (cpal).
  **Jitter buffer adaptativo**: latencia acotada (~120 ms, descarta ráfagas con
  catch-up suave) + prebuffer (~40 ms, sin chasquido al arrancar / tras underrun).
- ✅ **Llamadas grupales (malla N-a-N automática)**: uniéndote a un solo nodo
  anfitrión, todos se descubren y auto-conectan. Cada nodo gossipea las
  multiaddrs dialables que conoce (`Paquete::Pares`); el receptor disca las que
  le faltan, con desempate por PeerId (sólo el menor inicia) para no duplicar
  conexiones. Verificado con 3 nodos: cada uno ve y recibe video de los otros dos.
- ✅ Cámara sintética por defecto (TestCard); webcam real v4l2 con `--features camara` en `uya-app`.
- ✅ Toggle de cámara / micrófono y cuelgue.
- ✅ **Conectar desde la UI**: la app muestra tu dirección dialable (para
  compartir) y un campo donde pegás (Ctrl/Cmd+V) la de un par + Enter/botón —
  ya no hace falta `UYA_CONECTAR`. (Funciona también por env, como antes.)
- ✅ **Entrar por nombre de sala (DHT)**: `UYA_SALA=<nombre>` se anuncia como
  provider de `BLAKE3("uya/sala/<nombre>")` en la Kademlia de `BrahmanNet` y
  descubre a los demás providers, que entran a la malla solos. Verificado con
  3 nodos (con `UYA_BOOTSTRAP` para sembrar el DHT): los tres se ven, sin pegar
  direcciones.
- ✅ **Zero-config en LAN (baliza multicast)**: en una sala, uya emite una baliza
  UDP multicast `uya1\t<sala>\t<puerto>\t<peerid>` (grupo 239.255.42.99:7799) y
  escucha las ajenas; al recibir una de su sala, reconstruye la multiaddr usando
  la **IP de origen del datagrama** (resuelve el caso loopback → anda entre
  máquinas) y disca. Room-aware, sin `UYA_BOOTSTRAP` ni `UYA_CONECTAR`. Verificado:
  2 nodos misma sala → se descubren y conectan solos. (También se sumó mDNS a
  `shared/card/card-net` para poblar el DHT, pero la baliza propia es el camino
  fiable de uya en LAN.)

## Pendiente (por orden)

1. **Firma agora del `Hola`**: el PeerId ya es estable (deriva de `BLAKE3(nombre)`),
   pero el nombre es auto-declarado. Atar la identidad a `agora`: firmar el `Hola`
   con la clave agora y verificarla, para que nadie suplante un nombre.
2. **Bootstrap público para WAN**: en LAN ya hay descubrimiento por baliza;
   falta un rendezvous conocido por defecto para "entrar por nombre" entre redes
   distintas (la baliza multicast no cruza routers).
3. **Cancelación de eco acústico (AEC)**: cuando el micro capta el parlante.
   Necesita un AEC real (y oídos/hardware para evaluarlo). El jitter buffer ya
   es adaptativo.
4. **SFU / selective forwarding** para grupos grandes (hoy malla completa: N² streams).
