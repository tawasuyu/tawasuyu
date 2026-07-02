# PLAN — Cruces de arquitectura de la suite + capas de frontera

> Estado: propuesta (2026-07-02). Cataloga los **cruces** donde un patrón ya
> resuelto en un dominio conviene reusar en otro (como `nakui` se volvió sustrato
> de multilienzo/nahual), y las **capas de frontera** (como se intentó con `hifas`)
> que benefician transversalmente. Regla rectora, heredada del SDD de sandokan:
> **sin duplicados** — reusar lo que ya existe antes de fabricar un paralelo.

## 0. Hallazgo base: `nakui-core` ya es sustrato compartido

El motor de event-sourcing de `nakui-core` (`event_log` con `seq` monótono + `Snapshot`
+ `delta::{FieldPath,FieldOp}` + `Executor` Rhai + `trait Store` con backends
intercambiables) **es genérico, no contable**. Ya lo consumen fuera de nakui:
`nahual/libs/meta-schema` (*"equivalente a `nakui_core::event_log::seed_and_log`"*),
`pluma-reactor`, `tullpu-icon-core`, `nahual-table-viewer`, `foreign-xlsx`, `llimphi
nodegraph`. El trabajo no es inventar: es **nombrar el patrón y explotarlo a propósito**.

---

## A. Cruces (reuso de patrón entre dominios)

### A1 — nakui → sandokan · journal durable del plano de control ← **EN CURSO**
`sandokan-core` ya tiene el vocabulario `Intent`/`Engine`/`LifecycleEvent`/`TelemetryFrame`,
pero **no tiene journal durable** (nada de log/replay/snapshot en `shared/sandokan/*/src`):
el estado del plano de control es efímero, en memoria. Un `sandokan-journal` que aplique el
**patrón** de `nakui_core::event_log` (append-only, `seq` monótono, snapshot + compactación,
replay-para-reconstruir, recuperación ante crash) le da a sandokan un **registro replayable,
auditable y recuperable** de quién arrancó/paró/reinició qué.

**Decisión de diseño (importante):** se reusa el **patrón**, no el crate `nakui-core` directo.
El `LogEntry` de nakui está acoplado a su modelo documental (`Seed`/`Morphism` + `FieldOp` +
Rhai); sandokan tiene eventos **tipados** (`LifecycleEvent`). Forzar el plano de control a un
modelo JSON/morfismo sería peor cruce que ninguno. Además hay **precedente probado al lado**:
`arje-brain-audit` (cadena de audit anclada al CAS, `verify_chain_from_cas`) ya materializa
esta idea en el dominio init. sandokan-journal es su hermano para el plano de control.
Fases en §A1.1.

### A2 — consolidar el recompute reactivo (hoy N implementaciones)
El patrón "nodo sucio → recomputar dependientes en orden topológico" está resuelto **cuatro
veces** sin base común: `pluma-notebook-exec::run_from` (cono de celdas), `pluma-graph`+
`pluma-transform`+`pluma-cuerpo` (derivación multilienzo con staleness), **nahual** MonadGraph,
**tullpu** `tullpu-ops` (grafo de operaciones de imagen). Oportunidad: extraer un crate
`shared/` de *incremental dataflow* (estilo Salsa/differential) que todos consuman, cableado al
`Handle::dispatch` de Llimphi. Candidato a extraer y generalizar: `pluma-reactor`.
**Riesgo:** es el más ambicioso; hacerlo mal acopla cuatro dominios a un motor inmaduro. Sólo
tras A1 (probar el reuso barato primero).

### A3 — nakui-sync `transport` → multilienzo colaborativo
`nakui-sync/src/transport.rs` ya difunde commits multi-cliente sobre el escritor único (orden
total gratis). multilienzo hoy es local (`pluma-store` sled). Cablear ese transporte daría
**edición colaborativa multi-dispositivo del haz de cuerpos** sin reinventar sync — el mismo
salto que nakui hizo de mono-usuario a multi-cliente. Depende de F-A (la capa CRDT) para
resolver conflictos de edición concurrente que el escritor único no cubre por sí solo.

---

## B. Capas de frontera (transversales, como `hifas`)

### F-A — capa CRDT/retículo compartida (`shared/crdt`), generalizando hifas
El lever más grande. La suite tiene DAGs direccionados por contenido (minga, akasha), gossip
firmado (agora) y multi-cliente (nakui-sync), pero **ningún tipo CRDT/join-semilattice
compartido**. `hifas` ya trae el núcleo (`lattice.rs`, `tarski.rs`, escrow monótono). Extraerlo
a `shared/` da **offline-first / convergencia sin coordinación** reutilizable por: multilienzo
colaborativo (A3), nakui-sync multi-cliente, convergencia minga/agora. Precondición honesta
(heredada de hifas): exige durabilidad-por-dispositivo o rehúsa (`M4.md` de hifas).

### F-B — el asesor de coordinación de hifas (U2) como linter de diseño
`cohomology`+`tarski`+`verdict`+`oracle` responden "corre libre / coordina aquí (entre X e Y)"
sobre una topología. Aplicado a las mallas reales de minga/agora, es una **herramienta de
diseño** que dice dónde la coordinación es inevitable *antes* de codearla. Se cosecha ya, sin
cablear escrow. **Frontera honesta:** no supera a CALM (`STRONG_RESULT.md` retira ese reclamo);
el aporte es el ruteo/localización por el grafo real.

### F-C — unificar procedencia/atestación (agora + arje) como primitiva
Ya hay dos implementaciones del mismo concepto —grant firmado sobre `(hash_contenido, permisos)`—
en `agora-channel` (userspace) y `wawa-kernel/claves.rs` (bare-metal, espejo). Es una primitiva
de frontera **emergente** ("raíz de confianza ejecutable direccionada por contenido") que,
nombrada como capa, serviría a minga (verificar blobs), sandokan (autorizar unidades), pluma
(firmar cuerpos). Está a medio unificar, no a medio inventar.

---

## C. Orden y criterio

Prioridad por ROI/riesgo (consolidar-lo-existente > superficie-nueva):

1. **A1 (nakui→sandokan journal)** — bajo riesgo, alto valor, consolida. ← arrancamos aquí.
2. **F-A (CRDT de hifas a `shared/`)** — alto valor; habilita A3 y colaboración offline-first.
3. **F-B (linter de coordinación)** — barato, se cosecha sin dependencias.
4. **A3 (multilienzo colaborativo)** — tras F-A.
5. **A2 (dataflow compartido)** — el más ambicioso; último, tras probar el reuso barato.
6. **F-C (procedencia unificada)** — cuando toque endurecer confianza cross-dominio.

**Criterio de cada bloque:** un `cargo test` verde que lo certifique (regla del repo: evidencia
como texto, no aserción). `cargo check --workspace` debe seguir pasando en `main`.

### A1.1 — Fases del journal de sandokan

- **A1.0 — crate `shared/sandokan/sandokan-journal`** ✅ (este bloque): tipos (`JournalEntry`,
  `JournalRecord`, `ControlPlaneState`, `UnitState`), `trait JournalBackend` + `MemoryBackend`
  + `FileBackend` (jsonl), `Journal<B>` (open→replay, record, snapshot, compact). Tests:
  replay reconstruye estado, recuperación tras "crash" (reopen), derivación de restarts,
  compactación idempotente.
- **A1.1 — wiring en `sandokan-local`**: que `LocalEngine` registre cada `LifecycleEvent` que
  emite en el journal (append tras la transición, como `arje-zero::on_death` difunde). El estado
  vivo (`list`/`status`) puede servirse del `ControlPlaneState` replayado.
- **A1.2 — recuperación en `sandokan-daemon`**: al arrancar, `Journal::open` reconstruye qué
  unidades creía vivas → reconciliar contra la realidad (procesos vivos) como hace
  `arje-zero` en restore. Cierra el lazo crash→recover del plano de control.
- **A1.3 — panel**: exponer el journal en `sandokan-monitor-llimphi` (línea de tiempo de
  lifecycle por unidad), reusando el formateo de audit que ya tiene `arje-card-llimphi`.

**Frontera:** el journal registra el *stream de eventos* del plano de control, no es un motor de
replicación ni de consenso. La convergencia multi-nodo (varios sandokan) es F-A, no esto.
