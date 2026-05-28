# asistente-puente — puente Linux entre wawa y los LLMs externos

Recibe consultas de la app `asistente.wasm` (kernel wawa, vía Akasha),
las traduce a una consulta de LLM con `pluma-llm` autodetect, y devuelve
una propuesta interpretada lista para presentar al humano.

## Estado: scaffolding (sin Akasha real todavía)

El binario hoy ofrece dos modos de transporte, según los flags:

- **stdio** (default, sin args): un único turno
  `Consulta → Propuesta/Error` sobre stdin/stdout. Útil para tests o
  ejercicios con `printf` + `xxd`.
- **daemon Unix socket** (`--socket <path>`): listen + accept en serie,
  cada cliente puede mandar N turnos hasta EOF. Útil para que el
  asistente Linux lo consulte sin lanzar un proceso por pregunta.

El payload en ambos modos es `MensajeAsistente` en postcard binario con
un prefijo de longitud `u32 LE`. El socket raw Akasha (multiplexación
real entre nodos wawa, broadcast, dedup) viene en una vuelta posterior;
el contrato del payload ya queda estable porque vive en
`shared/format::MensajeAsistente`.

## Lo que ya funciona

- **Lógica pura testeada** (`src/lib.rs`, 12 tests): traducción
  JSON-del-LLM → `AccionPropuesta`, prompt sistema explícito, prompt
  usuario que pega el `Contexto` recibido del kernel + la pregunta
  humana. Sin red, sin grafo — el bloque más caliente del puente.
- **Binario stub** (`src/main.rs`): un único turno de
  `Consulta → Propuesta/Error`. Inicializa pluma-llm desde el env;
  sin credenciales cae al Mock.

## Lo que falta

- Bind a un socket raw Akasha (EtherType propio). El kernel wawa filtra
  paquetes con `CANAL_ASISTENTE = 0x4153` hacia los suscriptores;
  necesitamos `cap_net_raw` o equivalente.
- Multiplexación: un puente sirviendo varios nodos wawa tiene que
  enrutar respuestas por `id` de la `Consulta`.
- Para `InstalarApp` / `CambiarConfiguracion`: emitir el objeto
  `Manifiesto` / `Configuracion` por el grafo (otra trama Akasha) antes
  de proponer su hash. Hoy el LLM puede inventar hashes — el kernel los
  rechazará al verificar, pero deberíamos cazarlo aquí antes.
- Daemon mode: en lugar de "una consulta por proceso", correr
  indefinidamente atendiendo el socket.

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
consultarlo. Util para iterar con el asistente Linux mientras el bind
Akasha no esté listo.

## Diseño completo

Ver `docs/ASISTENTE_WAWA.md`.
