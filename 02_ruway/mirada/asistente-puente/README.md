# asistente-puente — puente Linux entre wawa y los LLMs externos

Recibe consultas de la app `asistente.wasm` (kernel wawa, vía Akasha),
las traduce a una consulta de LLM con `pluma-llm` autodetect, y devuelve
una propuesta interpretada lista para presentar al humano.

## Estado: scaffolding (sin Akasha real todavía)

El binario hoy lee/escribe `MensajeAsistente` por **stdin/stdout en
postcard binario** con un prefijo de longitud `u32 LE`. Eso permite
probar el contrato end-to-end con tests o con `printf` + `xxd`. El
socket raw Akasha (multiplexación, broadcast, dedup) viene en una vuelta
posterior; el contrato del payload ya queda estable porque vive en
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

Sin credenciales, con el Mock (que responde con frases canned):

```bash
# Construir una Consulta de prueba (necesitas un helper, no hay uno
# todavía — se haría en Rust o con un script que escriba postcard).
cargo run -p asistente-puente
```

Con credenciales reales, cualquiera de las que `pluma-llm` autodetecta:

```bash
ANTHROPIC_API_KEY=sk-... cargo run -p asistente-puente < consulta.bin
```

## Diseño completo

Ver `docs/ASISTENTE_WAWA.md`.
