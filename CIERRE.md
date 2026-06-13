# Cierre del monorepo → suite publicable

> Plan de trabajo por niveles para llevar tawasuyu de "compila y se usa" a
> "publicable". Anclado en `INVENTARIO_APPS.md` (2026-05-31), `PLAN.md` y el
> estado real del árbol al **2026-06-01**.
>
> **Smoke test hoy:** `cargo check --workspace` pasa limpio (exit 0). El cuelgue
> transversal de Llimphi está **resuelto** (era el solver de Kepler en
> `cosmos-ephemeris`, no infra). Único warning vivo: `russh` future-incompat.

## Cómo leer este documento

Dos ejes cruzados:

- **Niveles** (0→5): de lo que *bloquea publicar* hacia abedentro hasta lo que
  *embellece*. No saltes el Nivel 0.
- **Quién**: 🧑 = lo retocás vos a mano (UI/UX, juicio de producto, hardware
  real). 🤖 = automatizable por Claude (infra, tests, integración mecánica,
  docs). 🤝 = mixto: Claude prepara, vos decidís.

Y tres cajones que el inventario ya separa y aquí se respetan:

- **PRIMERO** — hay que atenderlo antes de publicar (legal, build, datos).
- **ABIERTO** — trabajo de core conocido, acotable, sin bloqueo externo.
- **NO DETALLABLE AÚN** — depende de hardware real, upstream, o decisión de
  alcance que todavía no tomaste. Se nombra, no se planifica.

---

## Nivel 0 — Bloqueadores de publicación (PRIMERO, antes que nada)

Esto no es código de producto: es lo que hace que el repo *pueda* hacerse
público sin romperte legal o reproduciblemente. Es barato y desbloquea todo.

| # | Tarea | Quién | Estado |
|---|---|---|---|
| 0.1 | **Archivos de licencia.** El default del workspace es `MIT OR Apache-2.0`; 6 crates de base (`format`, `forth-emisor`, `foreign-fs`, `wawa`/`wawa-kernel`/`wawa-fs`) overridean a `MPL-2.0`. | 🤖 | ✅ `LICENSE-{APACHE,MIT,MPL}` verbatim (SPDX) + `LICENSE.md` (commit `80224d36`) |
| 0.2 | **Untracked sin commitear.** `crates/apps/` + `web/tawasuyu-web/pkg/` eran artefactos; `nahual-svg-viewer-llimphi` lo está creando **otro agente** (stub). | 🤝 | ✅ `pkg/` y `/crates/` a `.gitignore`; svg-viewer se deja al otro agente |
| 0.3 | **CI mínimo.** Workflow que corre `cargo check --workspace` + `check-shared-cores.sh`. Guardián de la regla dura #5 y de la simetría no_std. | 🤖 | ✅ `.github/workflows/ci.yml` |
| 0.4 | **Política `publish`.** Sólo relevante para crates.io (eje opcional). Para el repo público en GitHub no hace falta. Si algún día se sube a crates.io: marcar bins/demos/sandboxes con `publish = false`. | 🤖 | ⏸️ diferido — no bloquea v1 |
| 0.5 | **Decisión de alcance público.** | 🧑 | ✅ **TODO el workspace público** (2026-06-01). Sin recortes, nada oculto/experimental — ni mirada. |

> **Salida del Nivel 0:** ✅ árbol limpio, licencias presentes, CI verde, alcance
> decidido (todo público). Nivel 0 **cerrado**.
>
> **Aclaración de alcance (0.5).** "Publicable" = **repo entero público en
> GitHub** bajo las licencias, clonable y compilable por cualquiera. Publicar los
> 454 crates a **crates.io** es un eje *separado y opcional* (nombres, orden de
> deps, y la mayoría son binarios de app/demos que no tiene sentido subir) — no
> es requisito de la v1.

---

## Nivel 1 — Higiene del workspace (🤖, casi todo automatizable)

Barrido transversal, mecánico, alto valor por esfuerzo. **Estado al 2026-06-01:**

- **1.1 — `russh` future-incompat.** ⏸️ **Deferido.** No es cosmético: bumpear
  `0.54 → 0.61` (7 minors) es una **migración de `shared/ssh`** con riesgo de
  API-break. El warning es no-fatal. Hacerlo como tarea propia, no a ciegas.
- **1.2 — Clippy.** ✅ Medido: **173 warnings, 0 errores** — todo cosmético
  (doc-indentation, float precision en tablas generadas de cosmos, dead code,
  `needless_range_loop`). **No se aplicó `--fix` en bloque** a propósito (regla
  "no embellecer sin pedido" + tocaría decenas de dominios en un commit
  arriesgado). Queda como pase opt-in cuando lo pidas.
- **1.3 — Inventario de `todo!()`/`unimplemented!()`.** ✅ Triado: en **código de
  producción** hay **uno solo** (`tinkuy-core/integrator.rs`), y está gateado tras
  `cfg(not(feature cpu|wasm))` con mensaje útil — guard de config, no deuda. Las
  ~67 marcas iniciales eran comentarios `TODO` y código de test. Ningún camino
  `todo!()` alcanzable por un usuario en build normal. **Limpio para publicar.**
- **1.4 — Tests ejecutados (no sólo compilados).** ✅ Corridos con **nextest**
  (timeout por test, mata cuelgues). Cobertura: **cores de lógica de dominio** de
  todos los cuadrantes (no GUI/`-llimphi`, no daemons, no wawa-excluido). Total
  ~**1.900 tests**:
  - `iniy-*` (el ⚠️ del inventario): **64/64 ✅** — la lógica está sólida; el ⚠️
    era el e2e con NLI real, no los unit tests.
  - Batch shared+raíz (format, forth-emisor, foreign-fs, mirada-layout,
    pluma-notebook-core, agora, minga, khipu): **516/516 ✅**.
  - cosmos compute (13 crates): **1267/1271 ✅**, 3 skipped, **4 lentos en debug**
    (búsquedas de eclipses/tránsitos de ventana larga — uno verificado: pasa en
    149 s; no son bugs). → marcarlos release-only o subir su timeout.
  - **`nakui-core`: 133/133 ✅** (empezó en 60/133 → 73 fallos). Cerrado esta
    sesión, en orden:
    - **`treasury` recuperado** del ejemplo `tesoro/nakui`, luego **curado** al
      diseño que esperan los tests (sólo `register_cash_move` + `transfer_between_cajas`;
      se quitaron los 5 morfismos UI tesoro que contaminaban el data-flow graph).
    - **skeletons `crm`/`inventory`/`sales` autorados** (nsmc.json + schema.ncl +
      scripts .rhai) alineados al contrato de cada test.
    - **records Nickel abiertos (`, ..`)** — el validador cierra records por
      defecto y rebotaba el campo `id`.
    - **morfismos treasury** `register_cash_move` (in/out sobre saldo + Movimiento)
      y `transfer_between_cajas` (conserva Caja.saldo por currency + Transferencia);
      `saldo`/`monto` no-negativos; scripts bajo `morphisms/` (convención que los
      tests de schema_versioning hardcodean).
    - **Lección transversal:** records Nickel cerrados por defecto (`, ..` para
      abrir) y la convención `morphisms/` para scripts de módulo.
  - dominium/tinkuy/supay/tullpu/media/takiy/chasqui/sandokan cores: verdes en lo
    corrido (sin fallos), corrida no exhaustiva.
- **1.4.bis — 2 bugs reales arreglados** (`1fa3d60f`, `0c556c62`): los examples de
  `cosmos-notebook-kernel` y `dominium-notebook-kernel` importaban el nombre viejo
  del crate (`pluma_notebook_kernel_{cosmos,dominium}`) tras un rename. `cargo
  check` no los veía; `--all-targets` sí. Los kernels `pluma-notebook-kernel-{llm,
  python,wasm,media,tinkuy}` conservan ese nombre → sus imports son correctos.
- **1.4.ter — disco.** El build de tests llenó el disco dos veces (`target/` ~127 GB
  / 147 GB). Se liberaron 45 GB borrando `target/debug/{incremental,examples}`
  (regenerables). La suite completa (con GUI + 454 crates) **no entra en este
  disco**; su lugar natural es CI (disco limpio). Ver nota ambiental.
- **1.5 — Metadata de paquete** (`repository`, `keywords`, `categories`). ⬜
  Sólo relevante si se sube a crates.io (eje opcional) — diferible.

> **Nota ambiental:** `target/` ocupaba 127 GB / 147 GB. Conviene `cargo clean`
> periódico o mover `CARGO_TARGET_DIR` a un disco con más holgura; con builds
> debug de 705k LOC se llena solo.

---

## Nivel 2 — Cerrar el *core* de cada app

Esta es la columna "Falta para cerrar el core" del inventario, ordenada por
**ROI de publicación** (cuánto sube el % vs cuánto cuesta), separando lo tuyo
de lo automatizable.

> **Auditoría 2026-06-10 (reconciliación con el árbol).** Las tareas 🤖 de
> 2B estaban en su mayoría **ya hechas** desde que se escribió este plan
> (2026-06-01) — verificadas con `file:line` y marcadas ✅ abajo: **minga**
> (#5/A), **chasqui** (`consume_remote`), **sandokan** (RestartTracker +
> RunCard Virtual; Wasm-incarnator pendiente), **shuma** (mouse + flock),
> **arje** (lifecycle compartido),
> **wawa-explorer** (process-monitor extraído). **media** tiene la lógica
> de PTS lista (resta wiring a `foreign-av`). **supay**: ordering BSP ✅
> (lo diferido es occlusion culling, perf 3.3+). Lo que queda genuinamente
> abierto en 2B y NO es trivial-🤖: **puriy** (APIs Web, largo), **iniy**
> (e2e probado), y los 🧑 de UX (**nakui**, **nahual**, **wawa host**).
> *(El incarnator WASM de sandokan se cerró el 2026-06-10 — ver fila.)*
> El cuello-de-publicación real ya
> no es "cerrar cores 🤖" sino el **Nivel 4 (pulido/UX, tu juicio)** y los
> pocos abiertos grandes.

### 2A. Las que ya casi cierran (≥80%) — empujón corto

| App | Acción de cierre | Quién |
|---|---|---|
| **pineal** (92) | Una viz densa real end-to-end GPU | 🤝 Claude arma demo, vos validás visual |
| **khipu** (88) | Sync bidireccional + resolución de conflictos | 🤖 |
| **tinkuy** (88) | Escenas editables desde DSL/grafo | 🤝 |
| **ayni** (88) | NAT traversal (deuda de minga, ver 3.x) | 🤖 |
| **pluma** (85) | Cerrar kernels notebook python/wasm; foreign-docx completo | 🤖 |
| **rimay** (85) | Gating de permiso de descarga del modelo | 🤖 |
| **dominium** (85) | Exponer `SimParams`/`ZWeights` restantes como dato | 🤖 |
| **llimphi** (85) | *(deadlock ya resuelto)* — repasar que no haya regresión | 🤝 |
| **nada** (82) | Multi-ventana / split de editores | 🧑 (UX) + 🤖 (plumbing) |
| **cosmos** (82) | **Edición rica de cartas in-situ** (hoy vía JSON manual) | 🧑 — es UX, tu terreno |
| **chaka** (80) | ~~REPLACE + ficheros indexed/relative~~ ✅ (verif. 2026-06-13, ya estaban: `chaka-lexer` expande COPY+REPLACE/REPLACING/OFF, `chaka-runtime::file` tiene Organization::{Indexed,Relative}). Fix nuevo: exponenciación COBOL `**` en codegen (emitía `Decimal::zero()`); ahora `Decimal::pow` compartido por codegen y shadow-interp + corpus `28-potencia`. | 🤖 |
| **tullpu** (80) | Nodegraph visual + tiling (espera `llimphi-surface`) | 🤝 |

### 2B. Hueco de core claro (60–78%) — donde está el trabajo real

| App | Acción de cierre | Quién |
|---|---|---|
| **agora** (80) | Tabla de capacidades por bytecode hash (§14.1.3) — **code-complete**: enforcement cableado + tool + boot-anchor + ceremonia scripteada (`scripts/wawa-conceder-genesis.sh`). Resta SÓLO el paso de operador (firmar con seed slot-0 + flip a estricto). | 🧑 — ceremonia con tu seed |
| **minga** (80) | ~~`MingaPeer` genérico para escala~~ ✅ (#5/A cerrado 2026-06-10): `MingaPeer<S: NodeStore>` sobre handle sled compartido — sync P2P sin volcar 1.44M nodos a RAM, snapshot O(1), merge O(delta). | 🤖 |
| **arje** (78) | ~~Cleanup socket daemon + `RestartTracker` en `LocalEngine`~~ ✅ (verif. 2026-06-10): supervisión con backoff vía `sandokan_lifecycle::{Backoff,RestartTracker}` (compartido, sin duplicar — `arje/init/arje-zero/src/graph/lifecycle.rs`); el `LocalEngine` con tracker vive en `sandokan-local` y cuenta restarts (tests `telemetry_cuenta_restarts_*`). | 🤖 |
| **supay** (78) | ~~BSP-walking real (orden de render)~~ ✅ (verif. 2026-06-10): `walk_bsp` back-to-front (R_PointOnSide) → `bsp_rank` como clave primaria del sort unificado (`frame.rs:208`), tested (`bsp_walk_*`/`bsp_ranks_*`). La imagen es correcta. **Diferido (perf, no correctitud, 3.3+):** occlusion culling (solidsegs) para no pintar lo oculto. *Audio vía takiy: ✅.* | 🤖 (perf, opt-in) |
| **shuma** (78) | ~~Mouse en PTY + lockfile del daemon~~ ✅ (verif. 2026-06-10): mouse xterm (`sandbox/shuma-module-shell/src/mouse_xterm.rs`) + lockfile `flock(LOCK_EX\|LOCK_NB)` del daemon (`shuma-daemon/src/main.rs:1473`, test `socket_in_use + flock`). | 🤖 |
| **puriy** (78) | Cerrar APIs Web restantes + conformance | 🤖 (largo) |
| **wawa-explorer** (78) | ~~Sacar process-monitor a su crate~~ ✅ (verif. 2026-06-10): extraído a `shared/sandokan/sandokan-monitor-core`; `wawa-explorer-core` ya no lo contiene. | 🤖 |
| **wawa host** (72) | Toggles de módulos con efecto real, accent→theme global | 🧑+🤖 |
| **takiy** (72) | Pulir `takiy-midi` (núcleo ya cerrado). *Primer consumidor real: supay (`play_takiy_score`).* | 🤖 |
| **nakui** (70) | **Editor de fórmulas en UI + WAL desde UI + vista formulario.** Módulos de producción (crm/inventory/sales/treasury) **completos y verdes — nakui-core 133/133** (ver 1.4). El hueco de core que resta es el de UI/persistencia desde la app, no la lógica de dominio. | 🧑 — UX, tuya |
| **nahual** (68) | Visor PDF (falta rasterizador) + SVG + seek/scrub | 🤝 (svg-viewer untracked ya empezado: ver 0.2) |
| **media** (68) | ~~**M1: sync A/V por PTS.**~~ ✅ (verif. 2026-06-13, ya cerrado en `3bfebb60`): `FfmpegVideoSource::pts()` + `AvSync::plan` consumidos en el paint loop de `media-app` (`modelo.rs`), drop de frames tardíos contra el reloj de audio. **M4 frame stepping** (`.`/`,`) ✅ (2026-06-13): primitivo `FrameSource::step_frame`. Lo que queda es ruta GPU/hardware (M2 hwdec, M3 seek frame-accurate) — requiere pantalla. | 🤖 |
| **iniy** (65 ⚠️) | **Pipeline e2e *probado* + NLI local** (hoy piezas sueltas/mock) | 🤖 — primero *verlo correr*, recién después subir % |
| **chasqui** (62) | ~~transporte/discovery P2P + connect-and-consume remoto~~ ✅ (verif. 2026-06-10): discovery DHT (`resolve_provider`) **+ `consume_remote`** (`card-sidecar/src/discovery.rs:256`: dial+`connect_libp2p` sobre los `PeerId`, fallback entre peers). *(La "persistencia del broker" es un no-feature: efímero por diseño.)* | 🤖 |
| **sandokan** (60) | ~~Cleanup socket + `RunCard` arbitraria~~ ✅ (cerrado 2026-06-10): `LocalEngine` con `RestartTracker` + socket por sesión + telemetría; `RunCard::Virtual` ✅ y **`RunCard::Wasm` ✅** — incarnator real (arje-cas resuelve `module_sha256`→bytes, arje-wasm los corre en wasmi, `WasmHandle` reporta terminación sin PID; test e2e `wasm_card_corre_en_cas_y_termina_limpio`). | 🤖 |

### 2C. Cuello real (<60%) — NO DETALLABLE como tarea simple

- **mirada** (55) — depende de un compositor/DM **estable sobre hardware
  real** y multi-scanout. La matemática multi-DPI está ✓; cablear a hardware no
  es planificable desde aquí. 🧑 (requiere tu máquina + display). Candidato a
  **quedar fuera de la v1 pública** y marcarse "experimental".

---

## Nivel 3 — Integraciones cruzadas (🤖, pero con orden de dependencias)

El inventario las marca como el patrón que arrastra varias apps a la vez.
**Mapeo de esta sesión** (con archivo:línea verificado): varias estaban más
hechas de lo que el plan asumía.

1. **NAT traversal** → ✅ **YA HECHO** (no era deuda). `shared/card/card-net`
   cablea relay + dcutr + autonat + Kademlia DHT, *vivo y testeado* (el test
   `jalar_a_traves_de_un_relay` de khipu jala un sobre por circuito relay).
   minga/agora/ayni/chasqui/khipu **ya consumen** card-net (`MingaPeer`,
   `EnlaceMinga`, `card-handshake::network`, `KhipuNode`). El único pendiente es
   un refactor `MemStore→NodeStore` en minga, *trigger-driven* (>100k nodos), no
   bloqueante. Corregidos los comentarios/docs obsoletos que decían lo contrario.
2. **Discovery de chasqui** → ✅ **cableado** (esta sesión). Resultó estar más
   hecho de lo que el map decía: el discovery por DHT ya vivía completo y
   testeado en `card-handshake::network` (`flow_dht_key` blake3 + `announce_outputs`
   que el **Server llama solo** al registrar + `find_remote_providers`, 3/3 en
   `network_discovery.rs`). Lo que faltaba era la **capa de consumidor**:
   `card-sidecar::discovery::resolve_provider(card, net, timeout)` → `ProviderLocation
   ::{Local(socket) | Remote(Vec<PeerId>)}` (local-first, fallback al DHT), con
   test de integración verde. *Follow-up fino:* el connect-and-consume remoto
   (dial + `connect_libp2p` sobre el `PeerId`) — primitivos ya en card-handshake.
   *(La "persistencia del broker" del inventario está **mal planteada**: el broker
   indexa por `SessionId` = conexiones vivas → es efímero por diseño; lo durable
   ya es el sled de Nouser + el keypair libp2p.)*
3. **Audio supay ↔ takiy** → ✅ **HECHO** esta sesión.
   `AudioEngine::play_takiy_score(score, vol, sep)` renderiza una partitura takiy
   (OscRenderer), la colapsa a mono y la encola como voz del `DoomMixer`. +2
   tests device-free; supay-audio 20/20.
4. **AppBus out-of-process** (nahual open-with) → 🟡 **mecanismo + seam hechos**
   (esta sesión); falta la última capa de GUI. Hecho y testeado:
   - `shared/app-bus`: `expand_target` (placeholders `%f`/`%u` estilo freedesktop),
     `AppEntry::open(target)` (spawnea Exec con el archivo), `AppRegistry::open_with
     (mime, target)` (elige handler + abre out-of-process). +4 tests, 12/12.
   - `nahual viewer_registry::external_handler_for(registry, discernment)` — el
     seam que mapea mime→app externa, independiente de `pick()` (sin tocar la
     GUI). +1 test.
   - **Resta (UX-driven, en la GUI):** que el mount del shell llame
     `external_handler_for` y, si hay handler, `open_with` + spawn en vez de
     montar un widget — incluye la política "¿externo gana a builtin?" (tu call).
5. **§14.1.3 wawa** (capacidades derivadas de firma) → ✅ **código hecho +
   verificado** (esta sesión). El binding firmado `(bytecode_hash, permisos)`
   está cableado end-to-end: `format::{ConcesionCapacidad, mensaje_capacidad,
   permisos_efectivos}` + `agora-channel::{firmar,verificar}_capacidad` +
   `claves::verificar_concesion_capacidad` (kernel) + `permisos_efectivos_de`
   (intersección fresh, fail-closed). **Verificado:** cadena host 9/9 (incl. los
   4 casos del modelo de amenaza: bytecode/permisos manipulados, autor ajeno) y el
   kernel compila a `x86_64-unknown-none`. El flip escalonado→estricto se
   convirtió en un toggle nombrado (`MODO_CAPACIDAD_ESTRICTO_GLOBAL`, `false`).
   **Resta sólo trabajo de operador (no código):** la ceremonia de génesis (firma
   offline de concesiones con las seeds del `AGORA_AUTH_RING`) + poner el toggle
   en `true`. 🧑 soberano.

> Realidad tras el mapeo: el "transporte abajo" (1) **ya estaba**; lo que de
> verdad apalanca varias apps ahora es **2 (discovery por DHT)**. ayni/khipu ya
> no están NAT-bloqueadas.

---

## Nivel 4 — Refinamiento (🧑 mayormente — es tu juicio de producto)

Esto es lo que **vos** tenés que retocar a mano; Claude no debe embellecer sin
pedido (regla: MVP feo primero, no adornar solo).

- **Pulido de UX app por app**: lo que en el inventario es "robustez y pulido"
  (15% de la rúbrica). Errores claros, edge cases, estados vacíos.
- **Coherencia visual** vía el *Llimphi elegance kit* (ya existe: chequearlo
  antes de inventar UX).
- **Edición in-situ** donde aún haya patrón "readonly arriba + editor abajo"
  (cosmos, nakui son los candidatos vivos).
- **Perf percibida**: arranque de demos, latencia de input, scroll.

Pasada por app, en el orden de 2A→2B, marcando en el inventario cuándo cada una
cruza a "lista para mostrar".

---

## Nivel 5 — Documentación (🤖 borrador, 🧑 voz)

- **5.1 — READMEs faltantes** ✅ (2026-06-09): `cards`, `tullpu`,
  `03_ukupacha/sandokan`, `shared/sandokan` (en+es), `launcher-llimphi`
  (en; es ya existía), `media` (en; el es pasó a LEEME.md). Todos los
  dominios tienen ahora README.md (en) + LEEME.md (es).
- **5.2 — README raíz orientado a *visitante público*.** ✅ (2026-06-09):
  `README.md` (en) + `LEEME.md` (es) con divulgación progresiva; el LEEME
  interno viejo quedó absorbido.
- **5.3 — `CONTRIBUTING.md`** ✅ (2026-06-09): bilingüe en un archivo,
  destilado de CLAUDE.md (reglas duras, setup, tests, licencias, sin CLA).
- **5.4 — Mapa de demos ejecutables.** ✅ vive como tabla "querés ver X →
  corré esto" dentro del README/LEEME raíz (comandos verificados contra
  el árbol).
- **5.5 — Cerrar SDDs pendientes** sólo de los dominios que se publican.

---

## NO DETALLABLE AÚN (se nombra, no se planifica)

- **mirada sobre hardware real** (compositor/DM, DRM, multi-scanout). Requiere
  tu display físico.
- **wawa como daily driver** (self-hosting, compilador Rust nativo in-cage).
  Visión de fases largas — `project_wawa_selfhosting_vision`. Fuera de v1.
- **Las 17 apps de `APLICACIONES.md`** sobre agora (roadmap, no código).
- **"Grafo de la Verdad" de minga**, GossipSub/reputación/+idiomas.
- **Targets WASM/bare-metal de apps host** (chaka no_std, rimay→Wawa, puriy
  bare-metal). Dependen de wawa madurando.
- **Familia `foreign-xlsx/-pptx`** (PLAN.md §6.ter, aún no en disco).
- Cualquier cosa que dependa de **decisión de alcance del Nivel 0.5** que no
  hayas tomado.

---

## Secuencia recomendada (ruta crítica)

```
Nivel 0  (días)      → licencias, untracked, CI, decisión de alcance   [DESBLOQUEA TODO]
Nivel 1  (días)      → workspace 100% limpio: clippy, tests, warnings
Nivel 3 (1·2·3·5)    → ✅ NAT, discovery DHT, audio, §14.1.3 (código+verif). 4 hasta el borde GUI. Resta: 4-GUI (UX) + ceremonias de operador
Nivel 2A             → empujón corto a las ≥80% que ya casi cierran
Nivel 2B             → el grueso del core (agora/§14.1.3, media M1, iniy e2e…)
Nivel 4  (en paralelo, tuyo) → pulido app por app a medida que cierran
Nivel 5  (al final)  → docs públicas con las apps ya estables
```

Regla de oro del repo: **al cierre de cada bloque funcional → `git add`
específicos + commit (español) + push a `origin/main`**, gateando con
`cargo check` *sin* pipe-mask.

---

## Próximo paso concreto

Lo más barato y desbloqueante es el **Nivel 0**. Puedo, ahora mismo y sin tu
intervención: agregar los archivos de licencia (0.1), redactar el workflow de
CI (0.3), y resolver el `.gitignore`/tracking de los untracked (0.2). Lo único
que necesita decisión tuya es **0.5 (qué se publica en v1)** — eso marca el
recorte de todo lo demás.
