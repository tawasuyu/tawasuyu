# SDD — Plano de control de gioser (sandokan)

> Estado: **2026-05-31**. Documento autoritativo del plano de control de
> procesos/apps en gioser. Cuando difiera con CLAUDE.md o PLAN.md, manda este.

## 0. Propósito

Definir **un solo** plano de control —arrancar, parar, supervisar y observar
unidades ejecutables— para los dos mundos de gioser:

- **Linux (host)**: el init `arje` y todo lo que corre sobre él.
- **Wawa (bare-metal)**: el kernel SASOS y sus apps WASM.

El problema que resuelve: hoy hay **lógica de control duplicada** (ver §4).
Este SDD fija qué pieza es dueña de cada responsabilidad para que nadie la
reimplemente.

## 1. Los cuatro roles del control

Todo plano de control se descompone en cuatro responsabilidades ortogonales.
La regla es: **cada rol tiene un único dueño por mundo**.

| Rol | Qué hace | Dueño Linux | Dueño Wawa |
|---|---|---|---|
| **Materializar** | Spec → cosa corriendo | `arje-incarnate` (clone+ns+cgroup) | kernel: `encender_app` + executor |
| **Política de vida** | backoff / restart / cuotas / estado | `sandokan-lifecycle` | kernel: fuel + techo memoria (oneshot) |
| **Contrato de control** | run / stop / list / status / telemetry | `sandokan-core::Engine` | kernel: compositor + `Mando` |
| **Transporte** | llevar la orden al dueño | `arje-bus` (Unix socket, postcard) | syscalls / IRQ (in-proceso) |

## 2. Modelo canónico — Linux

```
clientes (arje-card · shuma · systemctl-compat)
        │  hablan SOLO el contrato Engine
        ▼
sandokan-core::Engine { run, stop, list, status, telemetry }
   ├─ Engine de SISTEMA  = arje-zero (PID 1)      ── boot + genesis + SIGCHLD
   └─ Engine NO-PID1     = sandokan-local          ── sesiones shuma, sandboxes, tests
        │                         (daemon / remote = transportes del MISMO contrato)
        ▼
política:  sandokan-lifecycle { Backoff, RestartPolicy, RestartTracker, LifecycleState }
        ▼
primitiva: arje-incarnate::Incarnator (Card → proceso aislado)
        ▼
transporte de las órdenes remotas: arje-bus
```

**Reglas:**

1. **Un contrato.** Toda orden de control se expresa como `sandokan-core::Engine`.
   Los clientes nunca arman spawn/kill/list a mano.
2. **Una política.** El backoff/restart/cuota/estado vive **sólo** en
   `sandokan-lifecycle`. Nadie más calcula backoff.
3. **Una primitiva.** Card→proceso es **sólo** `arje-incarnate`. (Ya se cumple:
   lo usan tanto arje-zero como sandokan-local.)
4. **Un transporte en Linux.** El wire de control es `arje-bus`. El
   `DaemonEngine`/`RemoteEngine` de sandokan viajan sobre ese mismo wire, no
   sobre un protocolo paralelo.
5. **arje-zero es un Engine, no un competidor.** Es PID 1 — *tiene* que poseer
   boot, génesis y SIGCHLD. Pero implementa la semántica del contrato `Engine`
   y reusa `sandokan-lifecycle`; no mantiene su propia matemática de restart.

### Por qué arje-zero sigue siendo especial

PID 1 hace cosas que ningún otro Engine puede: cosecha zombis del sistema
(SIGCHLD global), instancia el génesis de la Semilla al boot, y orquesta el
apagado en cascada. Eso es **implementación de sistema**, no un contrato
distinto. La fachada hacia afuera es `Engine`; lo de adentro es lo único que
justifica que arje-zero exista aparte de `sandokan-local`.

## 3. Modelo canónico — Wawa

Wawa **no comparte código de control con Linux, y está bien así.** Es `no_std`,
`x86_64-unknown-none`, sin POSIX (nada de fork/exec/signal/cgroups). Forzar
`sandokan`/`card_core::Card` dentro del kernel sería el error opuesto al que
este SDD evita.

```
manifiesto [EntradaApp]  (postcard no_std, en el DAG)
        ▼  encender_app
AplicacionWasm (wasmi)  ── guardarraíles: fuel/tick · techo memoria · capacidades gateadas en el linker
        ▼  spawn
tarea cooperativa en el executor (async_system)  ── dueño del ciclo de vida
        ▼
compositor  ── Alt+Q (Mando::Cerrar = stop) · desalojo por falla (trap/sin-fuel/sin-memoria)
```

- **Sin restart automático**: las apps de génesis son oneshot. Re-instalar es
  re-anclar un manifiesto (`sys_manifiesto_proponer`), no un loop de supervisión.
- `EntradaApp` ≠ `card_core::Card`: **ortogonales**. `EntradaApp` es mínima
  (nombre, bytecode, región, techo, fuel, permisos, concesión); `Card` es la
  spec POSIX completa (namespaces, cgroups, supervision, flow…).

### Correspondencia conceptual (rhyme, no código compartido)

| Concepto | Linux / sandokan | Wawa |
|---|---|---|
| unidad | `Card` | `EntradaApp` |
| materializar | `Incarnator::incarnate` | `encender_app` |
| sandbox | namespaces + cgroups + seccomp | wasmi + capacidades en el linker |
| cuota | rlimits (vía `soma`) | fuel/tick + techo memoria |
| supervisor | Engine de sistema (arje-zero) | executor cooperativo |
| stop | SIGTERM→grace→SIGKILL | `Mando::Cerrar` / desalojo |
| substrato común | **DAG direccionado por contenido** + `format` (no_std) | idem |

Lo único compartido por código: `shared/format` (no_std) y el DAG/Akasha. El
parecido del resto es intencional —el mismo modelo mental en dos runtimes— pero
**no se unifica en código** porque los mecanismos no tienen nada en común.

## 4. Duplicados detectados (a resolver)

Verificados en disco al 2026-05-31:

| # | Duplicado | Ubicaciones | Resolución |
|---|---|---|---|
| 1 | **backoff/restart** calculado dos veces | `arje-zero/src/graph/lifecycle.rs:18` (`backoff_delay` + `restart_state.attempts`) **vs** `sandokan-lifecycle/src/{backoff,restart}.rs` | arje-zero adopta `sandokan_lifecycle::Backoff`; borra su `backoff_delay` |
| 2 | **gestión de ciclo de vida** dos veces | supervisor de `arje-zero` (`on_death`) **vs** `sandokan-local::LocalEngine` | arje-zero expone/implementa `Engine`; `sandokan-local` queda como Engine no-PID1 |
| 3 | **protocolo IPC de control** dos veces | `arje-bus` (`BusRequest::{SpawnCardFromDisk,KillEnte,ListEntes}`) **vs** `sandokan-daemon` (postcard socket propio) | el `DaemonEngine` viaja sobre arje-bus; un solo wire |

No-duplicados (correctos hoy): `arje-incarnate` es la única primitiva de
materialización; Wawa es deliberadamente separado.

## 5. Roadmap de dedup (orden por riesgo)

### Fase 1 — backoff a `sandokan-lifecycle` *(menor riesgo)* — ✅ 2026-05-31
arje-zero depende de `sandokan-lifecycle`. `restart_state` guarda un `Backoff`
por label en vez de `attempts: u32`; `on_death` llama `backoff.next_delay()` /
`backoff.reset()` (cuando uptime ≥ max). Se borra la fn pura `backoff_delay` y
sus tests migran a verificar equivalencia vía el `Backoff` canónico. Sin cambio
de comportamiento observable.

### Fase 2 — un solo transporte de control *(medio)* — ✅ (núcleo) 2026-05-31
El subconjunto de control de `arje-bus` se vuelve el wire del `Engine`.
- **Paso A ✅**: `arje-bus` ganó `EnteStatus`/`EnteTelemetry` (+ `Liveness`/
  `ResourceSample`), respondidos por arje-zero (telemetry lee `/proc`). Era el
  vocabulario que faltaba para cubrir el contrato.
- **Paso B ✅**: `sandokan-arje-engine` implementa `sandokan-core::Engine`
  hablando arje-bus a arje-zero. El `Engine` delegado YA viaja por el bus del
  init, no por un socket paralelo.
- **Pendiente (cleanup)**: deprecar/borrar el socket propio de `sandokan-daemon`
  (hoy redundante, sin consumidores). `run` por ahora mapea a
  `SpawnCardFromDisk{name:label}` (store-based); `RunCard{card}` arbitraria
  queda como opción futura.

### Fase 3 — arje-zero detrás de `Engine` *(mayor riesgo, toca PID 1)* — ✅ (vía bridge) 2026-05-31
arje-zero queda alcanzable como `sandokan-core::Engine` a través de
`sandokan-arje-engine` (Paso B de Fase 2): los clientes hablan el contrato
`Engine`, backed by arje-zero sobre el bus. Decisión de diseño: el init queda
**bus-native** y el `impl Engine` vive en el bridge —más limpio que meter un
trait async dentro de PID 1—. `sandokan-local` queda explícitamente como el
Engine para contextos no-PID1 (shuma, sandboxes, tests). Ambos comparten
`sandokan-lifecycle` y `arje-incarnate`. Suite de arje-zero verde antes/después.

**Invariante de cierre**: tras las 3 fases, `grep -r backoff` da un solo
calculador; hay un solo trait de control consumido; hay un solo wire de control
en Linux.

## 6. Plan coordinado — Process monitor

> Es un plan propio, pero **coordinado aquí**: el monitor es el consumidor
> *de sólo-lectura* del mismo contrato `Engine`. No inventa su propia vía de
> observación; usa `Engine::{list, status, telemetry}`.

### Principio
El monitor **no controla** (eso es el Engine); **observa**. Pero observa por el
mismo contrato, no por un canal paralelo. Así "lo que ves" y "lo que controlás"
son la misma fuente de verdad.

### Relación con lo ya hecho
`arje-card-llimphi` (la card de escritorio del init) ya es un precursor: su
sección **Unidades** lee el card store directo del filesystem, y **Brain/Audit**
por el socket introspect. El process monitor **promueve** esa sección Unidades a
leer por `Engine::list`/`status`/`telemetry` — estado vivo (Running/Exited/
Failed/Killed), conteo de restarts, CPU/mem/threads— en vez de sólo el `.json`
estático del store.

### Piezas
| Pieza | Crate | Rol |
|---|---|---|
| núcleo agnóstico | `sandokan-monitor-core` (nuevo, en `shared/sandokan`) | snapshot del Engine: `MonitorSnapshot { units: Vec<UnitObservation> }` con estado+telemetría+restarts; polling sobre `Engine` (cualquier transporte) |
| frontend | `arje-card-llimphi` (extender) o `sandokan-monitor-llimphi` | pinta el snapshot; reusa los stat-cards/banners ya hechos |

`UnitObservation = { card_id, label, state: LifecycleState, telemetry: Option<TelemetryFrame>, restarts: u32 }`.

### Fases del monitor
*(Fase 1 ✅ y Fase 3 ✅ entregadas 2026-05-31; Fase 2 pendiente; Fase 4 futura.)*
1. **`sandokan-monitor-core`** ✅: `fn observe(engine: &dyn Engine) -> MonitorSnapshot`
   (async): `list()` → por cada handle `status()` + `telemetry()`. Agnóstico de
   transporte (sirve LocalEngine para tests, DaemonEngine en vivo). Tests con un
   Engine mock.
2. **Restarts visibles** ✅ 2026-05-31: `TelemetryFrame.restarts` (contrato) +
   `ResourceSample.restarts` (bus) + contador en `arje-zero` (`RestartState`,
   reset al estabilizarse) → el bridge lo mapea, `observe` lo surfacea y la card
   lo muestra (`↻N`). `LocalEngine` aún lo deja en 0 (pendiente sobre
   `RestartTracker::count`).
3. **Frontend**: la sección "Unidades" de `arje-card-llimphi` pasa a consumir
   `MonitorSnapshot` cuando hay un Engine alcanzable; cae al scan del store si no
   (degradación, igual que el brain). Estado por color, telemetría en los items.
4. **Wawa**: el monitor de Wawa NO es este crate (es otro mundo). El
   `wawa-explorer-*` ya observa el DAG/manifiesto; el equivalente "process
   monitor" de Wawa es leer el censo de tareas del executor + balizas del
   compositor — pieza futura del lado wawa, fuera de `sandokan-monitor-core`.

### Por qué coordinado y no aparte
Si el monitor leyera procesos "por su cuenta" (p.ej. `/proc` o el card store
crudo), volveríamos a tener dos fuentes de verdad —la del control y la de la
observación— que es justo el tipo de duplicado que este SDD elimina. El monitor
es la cara de lectura del Engine.

## 7. Reglas duras (resumen)

1. Un **contrato** de control: `sandokan-core::Engine`.
2. Una **política** de vida: `sandokan-lifecycle`.
3. Una **primitiva** de materialización por mundo: `arje-incarnate` (Linux),
   `encender_app` (Wawa).
4. Un **transporte** de control por mundo: `arje-bus` (Linux), syscalls (Wawa).
5. **Wawa no importa `sandokan` ni `card_core::Card`.** Comparte sólo `format`
   (no_std) + DAG.
6. El **monitor observa por el contrato**, nunca por un canal paralelo.
