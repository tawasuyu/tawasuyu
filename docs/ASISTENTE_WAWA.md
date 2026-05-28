# Asistente conversacional en wawa — diseño técnico

> Documento de diseño. **Estado: hito 1-2 (formato del protocolo) ya
> implementado en `shared/format`**. La contraparte Linux
> (`mirada-asistente-llimphi`) ya existe y sirve como referencia operativa
> del flujo "propuesta → confirmación humana → ejecución".

## 0. Por qué un documento y no código

Tres frenes hacen que la versión wawa NO sea un puerto directo del asistente
Linux:

1. **El kernel no habla TCP/IP.** wawa tiene `akasha` (EtherType propio,
   capa-2). Para llegar a un LLM externo (Anthropic, Gemini, Ollama local en
   otra máquina) hace falta un *puente* que vive fuera de wawa.
2. **Las apps son WASM aisladas por bits de permiso.** Una `asistente.wasm`
   con permiso "ejecutar todo" violaría el modelo de capacidades del kernel.
   Hay que limitarla a *proponer*, no a *ejecutar*.
3. **La autoridad para mutar el sistema vive en `AGORA_AUTH_RING`.** Tres
   pubkeys Ed25519. Una IA no puede actuar como autora — sólo como
   sugerente. Cualquier propuesta atómica (re-anclar manifiesto, instalar
   app) tiene que ir firmada por una de esas pubkeys, que sólo el humano
   posee.

Esos tres puntos definen la arquitectura.

## 1. Las tres piezas

```
┌─────────────────────┐    Akasha     ┌─────────────────────┐    HTTP    ┌─────────┐
│  asistente.wasm     │ ────────────► │  asistente-puente   │ ─────────► │  LLM    │
│  (en wawa kernel)   │ ◄──────────── │  (en una máquina    │ ◄───────── │ (Anthr. │
│                     │   propuesta   │   Linux de la       │   resp.    │  Gemini │
│  - input de texto   │   firmada     │   misma red Akasha) │            │  Ollama │
│  - muestra propuesta│               │                     │            │  ...)   │
│  - el humano firma  │               │  - habla akasha     │            │         │
└─────────────────────┘               │  - habla HTTP/TLS   │            └─────────┘
                                      │  - pluma-llm        │
                                      └─────────────────────┘
```

Tres procesos en tres dominios diferentes. El puente es el único que conoce
HTTP — el kernel y la app WASM viven en Akasha pura.

## 2. La app `asistente.wasm`

### 2.1 Permisos

```
Permisos = RED | GRAFO_LECTURA
```

- **RED**: necesita enviar y recibir `MensajeAkasha`s. Esto cubre el
  diálogo con el `asistente-puente`.
- **GRAFO_LECTURA**: para leer el manifiesto vigente, el catálogo de apps,
  la configuración activa — el LLM necesita contexto del estado del sistema
  para sugerir algo razonable.

**Lo que NO se le da**:

- ❌ `RAIZ`: no puede re-anclar el manifiesto. Esa es la firma humana.
- ❌ `GRAFO_ESCRITURA`: no puede escribir nodos al grafo. (Discutible: si
  permitimos que ofrezca redactar notas tipo "pluma", quizá sí, pero como
  *propuesta* tipada — ver §3.)
- ❌ `COMPACTAR`: no puede invocar GC.
- ❌ `ALTAVOZ` / `RAW_INPUT` / cualquier capacidad sensible.

### 2.2 ABI nueva: canal Akasha "asistente" — ✅ implementado

`shared/format/src/lib.rs` define los tipos del protocolo y un canal
Akasha bien conocido. Importables como `format::CANAL_ASISTENTE`,
`format::MensajeAsistente`, `format::AccionPropuesta`, `format::Contexto`.
Round-trip postcard verificado por 7 tests en `mod pruebas`.

- **`CANAL_ASISTENTE: u16 = 0x4153`** (ASCII `"AS"`). El kernel filtra
  frames con este canal hacia los suscriptores del oficio asistente; el
  puente Linux abre un socket raw que suscribe al mismo número.

- **`MensajeAsistente`** con variantes:
  - `Consulta { id, prompt, contexto }` — la app pregunta. `id` es
    `u64` para correlación; el puente sirviendo varios nodos los
    distingue por id antes de cualquier RTT extra.
  - `Propuesta { id, accion, explicacion, confianza: f32 }` — el
    puente responde. `confianza` es `1.0` si el LLM produjo JSON limpio
    y la acción está en lista blanca; menos si tuvo que adivinar.
  - `Error { id, motivo }` — el puente reporta fallo de transporte o
    parseo.

- **`AccionPropuesta`**: `LanzarApp { plantilla: u32 }`, `InstalarApp
  { manifiesto_propuesto: Hash }`, `CambiarConfiguracion {
  config_propuesta: Hash }`, `Notar { texto: String }`. Las dos del
  medio referencian objetos del grafo por hash — el puente los preparó
  e ingestó por Akasha; el kernel los verifica al aplicar (la firma
  humana vía `daemon-firma` sigue siendo obligatoria para `InstalarApp`
  y `CambiarConfiguracion`).

- **`Contexto { apps, manifiesto_actual, configuracion_activa }`** —
  acotado deliberadamente para que la consulta no infle la tarifa de
  tokens. Si más adelante hace falta enviar workspace activo, modo de
  teselado, foco vigente, etc., se agregan campos al struct (postcard
  tolera extensión hacia atrás siempre que sea sufijo).

`MensajeAsistente` deriva `PartialEq` pero NO `Eq` porque `confianza:
f32` no es Eq por NaN. Aceptable: el operador no compara mensajes por
igualdad estricta en runtime — el round-trip de tests usa `assert_eq!`
con valores literales, donde el f32 es bit-exacto.

### 2.3 Flujo desde la app

1. El humano escribe en el input (mismo widget Llimphi-style que el
   asistente Linux, adaptado a wawa: pintar caracteres pixel-a-pixel sobre
   el lienzo de la app, igual que `pluma`).
2. La app empaqueta `MensajeAsistente::Consulta` y lo envía vía
   `sys_red_enviar` al canal del puente.
3. Espera la respuesta. Si llega `Propuesta`, la pinta en pantalla. Si
   llega `Error`, lo muestra al humano.
4. Si el humano confirma, la app entrega la propuesta al kernel pidiendo la
   firma del operador — ver §4.

## 3. El puente `asistente-puente`

### 3.1 Dónde corre

En cualquier máquina Linux que comparta la red Akasha física (misma VLAN /
mismo broadcast EtherType). En desarrollo: la propia máquina host del
QEMU. En producción: una máquina dedicada que también funciona como
"chaski" (correo) para el cluster de wawas.

### 3.2 Qué hace

- Suscribe al canal `CANAL_ASISTENTE` con un socket raw (cap_net_raw o
  equivalente).
- Por cada `Consulta` recibida:
  1. Construye un `ChatRequest` con prompt de sistema que explica:
     - "Eres asistente de un nodo wawa".
     - "Las acciones posibles son: lanzar app, instalar app, cambiar
       configuración, anotar".
     - "Responde con JSON estructurado en este shape: {...}".
     - El `contexto` del mensaje como info de estado.
  2. Llama `pluma_llm::from_env()` (autodetecta backend).
  3. Parsea la respuesta del LLM. Si es un JSON válido y la acción está en
     la enumeración, construye un `MensajeAsistente::Propuesta`.
  4. Si la acción exige material adicional (un nuevo manifiesto, una nueva
     configuración), **el puente lo prepara**: arma el objeto serialized
     (con `format::Manifiesto` o `format::Configuracion`), lo emite por
     Akasha como un objeto del grafo (igual que cualquier nodo viajero),
     y referencia su `Hash` en la propuesta.
- Por cada `Propuesta` que tampoco se materializa: emite el error.

### 3.3 Compatibilidad de seguridad

El puente **no tiene** la llave del anillo `AGORA_AUTH_RING`. Genera
*objetos del grafo* (que cualquier nodo puede generar — sólo escribir bytes
direccionados por contenido), pero NO genera firmas Ed25519 sobre ellos.

Esto significa: aunque el puente sea comprometido o el LLM sea adverso,
*lo más que pueden hacer es proponer*. La propuesta llega al humano; el
humano firma o no firma.

## 4. La firma humana

Hoy la app `mudanza` ya implementa este patrón: recibe un sobre
`ManifiestoFirmado` por Akasha, lo presenta al humano, y si éste pulsa
"aceptar" la app invoca `sys_manifiesto_proponer` con el sobre — el kernel
verifica contra `AGORA_AUTH_RING` y procede.

El asistente reusa esa máquina:

1. La app `asistente.wasm` recibe `Propuesta { accion: InstalarApp {
   manifiesto_propuesto: Hash } }`.
2. Pinta "El LLM sugiere instalar la app X (porque dijiste 'Y').
   ¿Aceptar?"
3. Si el humano acepta, la app emite un Akasha `RequestFirma` al **otro
   lado** — un demonio host-side de firmas (`wawactl daemon-firma`, que ya
   existe en Fase 49) que pide al humano confirmación interactiva por
   prompt y firma con la seed del slot indicado.
4. La firma vuelve por Akasha, la app construye el `ManifiestoFirmado` y
   llama `sys_manifiesto_proponer`. El kernel verifica y re-ancla.

Para acciones que NO exigen firma (`Notar`, o un `LanzarApp` de una
plantilla ya instalada), el flujo se acorta: la app simplemente invoca la
syscall correspondiente (`PARTOS_POR_INDICE` para lanzar) sin pasar por
`daemon-firma`.

## 5. Lista de hitos para implementar esto

Mostrados en orden de dependencia, no de complejidad:

1. ~~**Definir `MensajeAsistente`** en `shared/format` como tipos `no_std
   + serde`. Reusables por la app, el kernel y el puente.~~ ✅ HECHO en
   `shared/format/src/lib.rs` (commit `c6eb9bd`, Fase 60 v1).
2. ~~**Reservar `CANAL_ASISTENTE = 0x4153`** en el catálogo de canales
   Akasha.~~ ✅ HECHO junto con el §1. Documentar en `WAWA.md §20` queda
   como nota de mantenimiento cuando se cierre la familia de canales.
3. **Escribir el puente** como crate Linux `02_ruway/mirada/asistente-
   puente`. **Parcialmente HECHO** (scaffolding stdio): la lógica pura
   (PROMPT_SISTEMA_WAWA, `traducir_propuesta_llm`,
   `construir_prompt_usuario`) ya vive en
   `02_ruway/mirada/asistente-puente/src/lib.rs` con 12 tests; el
   binario lee/escribe `MensajeAsistente` por stdin/stdout en postcard
   con prefijo `u32 LE`. **Falta** el bind al socket raw Akasha
   (cap_net_raw), la multiplexación por id, y el modo daemon.
   ~1-2 sesiones restantes.
4. **Escribir `asistente.wasm`** como app cdylib en
   `03_ukupacha/wawa/apps/asistente/`. **v1+v2 HECHO** (Fase 60 v3+v4):
   cdylib `no_std + panic=abort`, `init()` y `tick()` pintan título +
   barra + roadmap dentro de la región 480×240. v2 sumó input de texto
   local: `sys_get_scancode` + edge-detect anti-rebote + tabla mínima
   de scancodes set 1 → ASCII mayúsculas + buffer `QUERY` de 64 chars
   con backspace y cursor visible. Enter es no-op (v3 lo conectará a
   `sys_red_enviar`). Verificado: compila a wasm32-unknown-unknown sin
   warnings, artefacto release ~3.8 KB. **Falta v3**
   (`sys_red_enviar`/`sys_red_recibir` sobre `CANAL_ASISTENTE`) y
   **v4** (presentar propuestas y disparar firma humana). ~2 sesiones
   restantes.
5. ~~**Cablear `daemon-firma`** para que también firme objetos
   `ConfiguracionFirmada` (hoy sólo firma manifiestos).~~ ✅ HECHO (Fase
   60 v2). `wawactl daemon-firma` ahora reconoce dos prefijos paralelos
   por transporte: `wawa::sign_request::` (cuaderno/manifiesto, legacy)
   y `wawa::sign_config::` (configuración, nuevo); equivalentes
   virtio-console `wawactl::sign_pci::` y `wawactl::sign_cfg::` con
   igual largo (19 B) para que la ventana deslizante del parser binario
   no cambie de tamaño. El prompt al operador y el log de auditoría
   incluyen el campo `TIPO: cuaderno|configuracion`. 6 tests cubren
   el clasificador.
6. **Sembrar `asistente.wasm` en GENESIS** o, mejor, dejar que el operador
   la instale en vivo vía `mudanza` (la palanca de v9/v10 del launcher).

Estimado restante: 4-8 sesiones (las fases 1-2 y 5 están hechas; 3 y 4
están a la mitad). El asistente Linux (`mirada-asistente-llimphi`) que
ya corre cubre el caso de uso "asistente conversacional para gioser"
para el operador humano de hoy; la versión wawa avanza en paralelo
según prioridades.

## 6. Modos de fallo

Conscientes:

- **El puente está caído**: la app espera respuesta que nunca llega, la
  abandona tras un timeout y muestra "asistente fuera de servicio". El
  kernel sigue, las otras apps siguen. Sin daño.
- **El LLM alucina una acción inexistente**: el parseo del puente falla,
  emite `Error`. La app lo muestra al humano sin proponer nada.
- **El LLM propone una acción destructiva** ("apaga el sistema",
  "borra todo el grafo"): la propuesta llega al humano con su
  `explicacion`. El humano lee y decide. Si confirma, *es decisión humana*
  — el modelo no actuó autónomamente. Si no confirma, la propuesta muere.
- **Un atacante envía propuestas al canal del asistente**: las propuestas
  son aceptables únicamente con firma del anillo en el último paso. Sin
  firma válida, `sys_manifiesto_proponer` rechaza. El asistente puede
  *ver* propuestas adversas, pero no ejecutarlas.

## 7. Lo que NO hace este diseño

Por elección, no por descuido:

- **No tiene memoria conversacional persistente** entre sesiones. Cada
  consulta es independiente. Si más adelante hace falta, el puente puede
  guardar historial — pero entonces el historial entra al alcance de
  ataque y hay que cuidarlo.
- **No ejecuta código generado** por el LLM. Sólo selecciona entre
  acciones pre-definidas en la enumeración `AccionPropuesta`. No hay
  "ejecutar este snippet de Rhai/Lua/whatever".
- **No habla con LLMs internos al kernel**. Todo modelo corre fuera, en
  un proceso Linux normal. El kernel jamás carga pesos de un modelo —
  eso explotaría el techo de memoria y el `wasmi` jail.
- **No requiere multi-monitor** (§59 del kernel). El asistente es una
  ventana más del compositor; el escritorio mono-output le basta hoy.

## 8. Referencias en el código actual

- `02_ruway/mirada/mirada-asistente-llimphi/src/main.rs` — contraparte
  Linux ya operativa. Mismo bucle Elm de "consulta → propuesta →
  confirmación → ejecución" — sirve como prototipo de UX.
- `03_ukupacha/wawa/apps/mudanza/` — app que ya implementa el patrón
  "propuesta firmada por Akasha → confirmación del humano → syscall
  `sys_manifiesto_proponer`". El asistente reusa exactamente esa máquina.
- `02_ruway/wawa/wawactl/src/main.rs::cmd_daemon_firma` — demonio
  host-side que firma con seeds del `AGORA_AUTH_RING`. El asistente lo
  invoca para conseguir firmas humanas en flujos no-interactivos.
- `00_unanchay/pluma/pluma-llm/src/lib.rs::from_env` — fachada que el
  puente reusa idéntica.

## 9. Estado

**Cerrados**: hitos 1-2 (formato del protocolo y canal Akasha) y 5
(daemon-firma discrimina cuaderno/configuración).

**A medio camino**:
- Hito 3 (puente Linux): `asistente-puente` crate con la lógica pura
  (PROMPT_SISTEMA_WAWA, traducir_propuesta_llm, construir_prompt_usuario)
  y dos modos de transporte (stdio + daemon Unix socket). Falta el bind
  raw Akasha (cap_net_raw, multiplexación entre nodos).
- Hito 4 (asistente.wasm): scaffolding v1 instalado en
  `03_ukupacha/wawa/apps/asistente/`. Compila a wasm32-unknown-unknown
  (~2.6 KB). Falta input de texto (v2), red Akasha (v3), propuesta +
  firma humana (v4).

**Abiertos**: hito 6 (siembra en GENESIS). Cero código: depende de v4
del hito 4.

Sin urgencia: el asistente Linux cubre el caso de uso "asistente
conversacional para gioser" para el operador humano de hoy; la versión
wawa es para cuando wawa sea el daily driver, que aún no lo es.
