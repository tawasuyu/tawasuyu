# asistente-puente — puente Linux entre wawa y los LLMs externos

Recibe consultas de la app `asistente.wasm` (kernel wawa, vía Akasha),
las traduce a una consulta de LLM con `pluma-llm` autodetect, y devuelve
una propuesta interpretada lista para presentar al humano.

## Estado: tres modos de transporte

El binario ofrece tres modos según el flag de línea de comandos:

- **stdio** (default, sin args): un único turno
  `Consulta → Propuesta/Error` sobre stdin/stdout. Útil para tests o
  ejercicios con `printf` + `xxd`. Payload: `MensajeAsistente` postcard
  con prefijo `u32 LE`.
- **daemon Unix socket** (`--socket <path>`): listen + accept en serie,
  cada cliente puede mandar N turnos hasta EOF. Útil para que el
  asistente Linux lo consulte sin lanzar un proceso por pregunta. Mismo
  payload que stdio.
- **Akasha** (`--akasha <iface>`): bind a `AF_PACKET SOCK_DGRAM` sobre la
  interfaz física, filtrado por `ETHERTYPE_ASISTENTE = 0x88B6`. Payload
  binario corto (`format::TipoCable`: 12 B cabecera + bytes específicos
  del tipo). Es el protocolo que habla `asistente.wasm` desde el kernel
  wawa. Requiere permisos para abrir AF_PACKET (cap_net_raw, root, o
  `setcap cap_net_raw=ep target/release/asistente-puente`).

## Lo que ya funciona

- **Lógica pura testeada** (`src/lib.rs`, 12 tests): traducción
  JSON-del-LLM → `AccionPropuesta`, prompt sistema explícito, prompt
  usuario que pega el `Contexto` recibido del kernel + la pregunta
  humana. Sin red, sin grafo — el bloque más caliente del puente.
- **Binario stub** (`src/main.rs`): un único turno de
  `Consulta → Propuesta/Error`. Inicializa pluma-llm desde el env;
  sin credenciales cae al Mock.

## Lo que falta

- Para `InstalarApp` / `CambiarConfiguracion`: emitir el objeto
  `Manifiesto` / `Configuracion` por el grafo (otra trama Akasha) antes
  de proponer su hash. Hoy el LLM puede inventar hashes — el kernel los
  rechazará al verificar, pero deberíamos cazarlo aquí antes.
- Multiplexación entre nodos: hoy el modo `--akasha` responde al
  broadcast; un nodo recibe respuestas dirigidas a *cualquier* nodo de
  la misma red (las filtra por `id` en la app `asistente.wasm`).
  Mejorable con sendto unicast al remitente que `recvfrom` reveló.
- Contexto del nodo: la `Consulta` v3 viaja sin `Contexto` (apps
  disponibles, manifiesto vigente). El puente arma un `Contexto::default()`
  vacío. Cuando v4 sume el contexto al payload del cable, el puente lo
  pasa al LLM y las propuestas pueden referirse a apps reales.

## Probarlo localmente

Modo stdio, sin credenciales (cae al Mock):

```bash
# Necesitás un helper que escriba postcard. Para test rápido:
cargo run -p asistente-puente -- --help
```

Con credenciales reales, cualquiera de las que `pluma-llm` autodetecta:

```bash
ANTHROPIC_API_KEY=sk-... cargo run -p asistente-puente < consulta.bin
```

Modo daemon en un Unix socket:

```bash
cargo run -p asistente-puente -- --socket /tmp/asistente.sock
```

Cualquier cliente que abra ese socket y emita frames postcard puede
consultarlo.

Modo Akasha sobre una interfaz física:

```bash
# Build release y dar capacidad sin sudo (preferido):
cargo build -p asistente-puente --release
sudo setcap cap_net_raw=ep target/release/asistente-puente
target/release/asistente-puente --akasha eth0

# O directo con sudo:
sudo cargo run -p asistente-puente --release -- --akasha eth0
```

Este modo bindeará un socket `AF_PACKET SOCK_DGRAM` a la interfaz
indicada, filtrado por `EtherType 0x88B6`. Cada `Consulta` que
`asistente.wasm` emita desde un nodo wawa en la misma red llega aquí,
se traduce a un prompt para el LLM, y la respuesta vuelve por broadcast
en el mismo EtherType.

## Diseño completo

Ver `docs/ASISTENTE_WAWA.md`.
