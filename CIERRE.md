# Cierre del monorepo → suite publicable

> Plan de trabajo por niveles para llevar gioser de "compila y se usa" a
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
| 0.2 | **Untracked sin commitear.** `crates/apps/` + `web/gioser-web/pkg/` eran artefactos; `nahual-svg-viewer-llimphi` lo está creando **otro agente** (stub). | 🤝 | ✅ `pkg/` y `/crates/` a `.gitignore`; svg-viewer se deja al otro agente |
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

Barrido transversal, mecánico, alto valor por esfuerzo.

- **1.1 — Matar el warning `russh` future-incompat.** Bumpear o pinear; que
  `cargo check` salga 100% limpio.
- **1.2 — `cargo clippy --workspace` como segundo smoke test.** Hoy sólo corre
  `check`. Clippy en `-D warnings` (al menos un pase de limpieza) sube la
  percepción de calidad de un repo público enorme.
- **1.3 — Inventario de `todo!()`/`unimplemented!()`.** Hay ~67 marcas
  TODO/FIXME/unimplemented. Triar: cuáles son deuda real de core (descuentan) y
  cuáles son notas. Las que estén en caminos que un usuario público puede
  pisar → cerrar o documentar como "no soportado".
- **1.4 — `cargo test --workspace` verde y medido.** Saber cuántos tests
  existen y cuáles fallan/ignoran. `iniy` está marcado ⚠️ justamente por
  pipeline e2e no verificado — esto lo destapa.
- **1.5 — Metadata de paquete uniforme** (`description`, `repository`,
  `keywords`, `categories`) en los crates que sí se publican. crates.io lo pide.

---

## Nivel 2 — Cerrar el *core* de cada app

Esta es la columna "Falta para cerrar el core" del inventario, ordenada por
**ROI de publicación** (cuánto sube el % vs cuánto cuesta), separando lo tuyo
de lo automatizable.

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
| **chaka** (80) | REPLACE + ficheros indexed/relative | 🤖 |
| **tullpu** (80) | Nodegraph visual + tiling (espera `llimphi-surface`) | 🤝 |

### 2B. Hueco de core claro (60–78%) — donde está el trabajo real

| App | Acción de cierre | Quién |
|---|---|---|
| **agora** (80) | Tabla de capacidades por bytecode hash (§14.1.3 — *primitivos YA existen*) | 🤖 — alto valor, es el "norte" de wawa |
| **minga** (80) | `MingaPeer` genérico para escala | 🤖 |
| **arje** (78) | Cleanup socket daemon + `RestartTracker` en `LocalEngine` | 🤖 |
| **supay** (78) | BSP-walking real (orden de render) | 🤖 |
| **shuma** (78) | Mouse en PTY + lockfile del daemon | 🤖 |
| **puriy** (78) | Cerrar APIs Web restantes + conformance | 🤖 (largo) |
| **wawa-explorer** (78) | Sacar process-monitor a su crate | 🤖 |
| **wawa host** (72) | Toggles de módulos con efecto real, accent→theme global | 🧑+🤖 |
| **takiy** (72) | Pulir `takiy-midi` (núcleo ya cerrado) | 🤖 |
| **nakui** (70) | **Editor de fórmulas en UI + WAL desde UI + vista formulario** | 🧑 — UX pesada, tuya |
| **nahual** (68) | Visor PDF (falta rasterizador) + SVG + seek/scrub | 🤝 (svg-viewer untracked ya empezado: ver 0.2) |
| **media** (68) | **M1: sync A/V por PTS completa** | 🤖 — es el cuello de media |
| **iniy** (65 ⚠️) | **Pipeline e2e *probado* + NLI local** (hoy piezas sueltas/mock) | 🤖 — primero *verlo correr*, recién después subir % |
| **chasqui** (62) | Persistencia del broker + transporte/discovery P2P | 🤖 |
| **sandokan** (60) | Cleanup socket + `RunCard` arbitraria | 🤖 |

### 2C. Cuello real (<60%) — NO DETALLABLE como tarea simple

- **mirada** (55) — depende de un compositor/DM **estable sobre hardware
  real** y multi-scanout. La matemática multi-DPI está ✓; cablear a hardware no
  es planificable desde aquí. 🧑 (requiere tu máquina + display). Candidato a
  **quedar fuera de la v1 pública** y marcarse "experimental".

---

## Nivel 3 — Integraciones cruzadas (🤖, pero con orden de dependencias)

El inventario las marca como el patrón que arrastra varias apps a la vez.
Cerrarlas sube el % de *múltiples* apps de un golpe — alto apalancamiento.

1. **NAT traversal en minga** → desbloquea `ayni`, `chasqui`, `khipu` (WAN/P2P
   real). card-net ya heredó relay/dcutr/autonat — falta cablearlo arriba.
2. **Transporte/discovery de chasqui** → bus vivo para `takiy` (audio),
   notebook, AppBus.
3. **Audio supay ↔ takiy** → cierra el camino de sonido del motor de juego.
4. **AppBus out-of-process** (nahual meta-app open-with) → despacho de visores
   entre procesos; hoy in-process.
5. **§14.1.3 wawa** (capacidades derivadas de firma) → cierra agora *y* sube la
   seguridad del kernel. Primitivos ya existen (`agora-core` + `claves.rs`).

> Regla: hacer 1 y 2 **antes** de marcar como cerradas las apps que dependen de
> ellas. No subas el % de ayni/khipu/chasqui sin el transporte abajo.

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

- **5.1 — READMEs faltantes** (PRIMERO dentro de docs): `02_ruway/cards`,
  `02_ruway/tullpu`, `03_ukupacha/sandokan`, `shared/launcher-llimphi`,
  `shared/sandokan`. Todos los demás dominios ya tienen.
- **5.2 — README raíz orientado a *visitante público*.** Hoy `LEEME.md`/`PLAN.md`
  son notas internas. Falta la puerta de entrada: qué es gioser, cómo se
  compila, qué se puede correr en 5 minutos (los `examples/*_demo.rs`).
- **5.3 — `CONTRIBUTING.md` + convenciones** (commits en español, regla un
  dominio = un crate, cuadrantes). Mucho ya vive en `CLAUDE.md` → destilar la
  parte pública.
- **5.4 — Mapa de demos ejecutables.** Una tabla "querés ver X → corré este
  comando". Es lo que convierte 454 crates en algo navegable para un extraño.
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
Nivel 3.1–3.2        → transporte minga/chasqui   [APALANCA varias apps]
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
