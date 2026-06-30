# SDD — Plano de control de tawasuyu (sandokan)

> Estado: **2026-05-31**. Documento autoritativo del plano de control de
> procesos/apps en tawasuyu. Cuando difiera con CLAUDE.md o PLAN.md, manda este.

## 0. Propósito

Definir **un solo** plano de control —arrancar, parar, supervisar y observar
unidades ejecutables— para los dos mundos de tawasuyu:

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

### 6.bis — Segundo consumidor de lectura: veto de suspensión (`energia`) — 2026-06-30
El `MonitorSnapshot` no sólo se pinta; también se **juzga**. El escritorio
(`pata`, el shell de la sesión) antes de **suspender por inactividad** le pregunta
al plano de control "¿hay trabajo importante en curso?" — para no cortar procesos
como los sistemas brutos (el dolor que motiva los workarounds tipo `caffeine`).

- **Pieza:** `sandokan-monitor-core::energia` (módulo, no crate nuevo) —
  `evaluar(&MonitorSnapshot, &PoliticaVeto) -> VeredictoSuspension { permite, bloqueos }`.
  **Puro**, sobre el snapshot que el monitor ya produce. Veta una unidad
  **corriendo** si quema ≥ `cpu_ocupada_pct` de CPU, o si su `label` coincide con
  una de las `etiquetas_despiertas` (keep-awake declarativo). No mira `/proc` ni
  el reloj: el consumidor combina este veredicto con sus propias señales de
  sistema.
- **Respeta el principio §6:** es otra cara de **lectura por el contrato** — se
  alimenta del mismo `Engine::{list,status,telemetry}` vía `observe()`, no de un
  canal paralelo. El veto no controla (no para nada); aconseja.
- **Límite honesto del transporte (importante):** vía **arje-bus** la telemetría
  **no trae CPU** (`sandokan-arje-engine`: `cpu_pct = 0.0`; el bus da RSS + hilos,
  no jiffies). Por eso el veto-por-CPU sólo pesa con engines que la miden
  (`LocalEngine`); con el engine que `pata` usa de verdad (arje), la coordinación
  que **sí** funciona es por **`label` keep-awake** (los labels llegan en
  `list()`). El "sistema ocupado" genérico (un `cargo build` suelto, que **no** es
  unidad gestionada) lo cubre el consumidor con `/proc/loadavg` por su cuenta —
  fuera del contrato, porque no es una unidad del plano de control.
- **Lo que NO se hizo, a propósito:** no se agregó `priority`/`inhibits_suspend`
  al wire de `EnteStatus`/`TelemetryFrame` ni a `UnitObservation`. Surfacear la
  `Card.priority` por el contrato tentaría tocar los enums de `arje-bus`, que
  `hammer` espeja por discriminantes hardcodeados (rompe en silencio). Si algún
  día se quiere "esta unidad inhibe suspensión" declarativo desde la Card, va por
  un método **aditivo** del trait `Engine` (`fn suspend_inhibit(id) -> Option<…>`,
  default `None`), no por cambiar la telemetría.

## 7. Reglas duras (resumen)

1. Un **contrato** de control: `sandokan-core::Engine`.
2. Una **política** de vida: `sandokan-lifecycle`.
3. Una **primitiva** de materialización por mundo: `arje-incarnate` (Linux),
   `encender_app` (Wawa).
4. Un **transporte** de control por mundo: `arje-bus` (Linux), syscalls (Wawa).
5. **Wawa no importa `sandokan` ni `card_core::Card`.** Comparte sólo `format`
   (no_std) + DAG.
6. El **monitor observa por el contrato**, nunca por un canal paralelo.

## 8. Repotenciación: del monitor al centro de control (2026-06-30)

> El plano de control ya tiene **contrato** (`Engine`), **política**
> (`sandokan-lifecycle`), **observación** (`sandokan-monitor-core` + la app
> `sandokan-monitor-llimphi`) y hasta un **cerebro de automatización**
> (`arje-brain-rules` evento→acción + `arje-brain-cognitive` que cristaliza
> reglas de patrones observados). Lo que falta no es inventar el orquestador
> —es **cablear y exponer** poderes que están a medio construir—. El objetivo:
> que el usuario controle el sistema **por intención**, no sólo matando PIDs.

El estado de partida tiene tres huecos verificados en disco (2026-06-30):

1. **El cerebro observa pero no actúa.** `arje_brain_rules::Action` =
   `Log/Notify/Spawn/Invoke/Inhibit`. Ninguna acción toca el contrato `Engine`
   (no hay `Stop`/`SetCpuWeight`/`Freeze`). El lazo está abierto.
2. **Verbos del contrato declarados pero no cableados.** `Engine` define
   `set_cpu_weight` y `freeze`, pero el único Engine real de Linux
   (`sandokan-arje-engine::ArjeEngine`, sobre `arje-bus`) los deja en
   `Unsupported` — no viajan por el bus, no hay `restart`, ni `io.weight`,
   `memory.high`, `cpuset`, `nice`/`ionice`.
3. **Disparadores y acciones pobres + sin capa de intención.** El cerebro sólo
   reacciona a `EnteSpawned/Died/Device/Bus`; no a umbrales de métrica, estado
   del sistema (AC/batería/red/idle) ni tiempo/agenda. Y no hay capa
   declarativa de **intención** (“presentando”, “ahorro”, “build pesado”) que
   reconcilie un conjunto de prioridades+reglas. Casa natural: `pacha`
   (contextos de usuario) + el `set_cpu_weight` cuyo doc ya cita “el slice de un
   contexto pacha”.

### Las cinco capas (fases verificables, orden por riesgo)

| Capa | Qué agrega | Invariante de cierre |
|---|---|---|
| **3 — Action→Engine** *(primera)* | el cerebro **actúa** sobre el contrato: `Action::{Stop,SetCpuWeight,Freeze}` + un `ActionSink` con teeth (`EngineSink`) que enruta a `sandokan_core::Engine` | una regla evento→acción puede detener/priorizar/congelar una unidad **vía el contrato**, probado con un Engine mock |
| **1 — verbos que faltan** | `Engine::restart`/`reload`; cablear `set_cpu_weight`/`freeze` por `arje-bus`→arje-zero (cgroup v2); `io.weight`, `memory.high/max`, `cpuset` (pin), `nice`/`ionice`; `set_ttl`/`enable` | `ArjeEngine` deja de responder `Unsupported`; el slider de prioridad del monitor escribe de verdad |
| **2 — disparadores** | `EventKind`/condición por **umbral de métrica** (CPU>X durante Ns, mem, temp, disco, batería), **estado** (AC/batería, red, idle, lock, login) y **tiempo** (cron, atardecer, boot, cada N). Forma canónica = evaluación pura sobre `MonitorSnapshot` (como `monitor-core::energia::evaluar`) | una regla “si chasqui >80% CPU 30 s → bajá su peso” se declara y dispara |
| **4 — intención (pacha)** | capa declarativa: intenciones nombradas que un **reconciliador** traduce a prioridades+reglas+freeze de estado deseado; atadas a `pacha` | elegir “presentando” congela lo no-esencial y prioriza mirada de una |
| **5 — UI centro de control** | el monitor pasa de leer+matar a **gobernar**: slider de prioridad (→`set_cpu_weight`), toggle freeze, restart, editor de reglas/automatizaciones, selector de intenciones, vista de dependencias | desde la app se ejerce cada verbo y se edita una automatización sin tocar JSON a mano |

**Reglas de la repotenciación** (heredadas del SDD): las acciones de control se
expresan **sólo** por el contrato `Engine` (capa 3 no abre un canal paralelo);
los disparadores de métrica evalúan el **mismo `MonitorSnapshot`** que el
monitor (capa 2 no inventa una segunda fuente); la intención (capa 4) **compone**
verbos+reglas existentes, no los reimplementa.

### Estado de la repotenciación
- **Capa 3** *(en curso, 2026-06-30)*: `arje_brain_rules::Action` gana
  `Stop/SetCpuWeight/Freeze` + `ActionSink` gana los métodos de control (default
  no-op para sinks de sólo-observación). Crate nuevo
  `03_ukupacha/sandokan/sandokan-brain` con `EngineSink` que implementa
  `ActionSink` enrutando los tres verbos a un `sandokan_core::Engine`
  (fire-and-forget vía `Handle::spawn`), probado con un Engine mock que registra
  las llamadas. Cierra el lazo observar→actuar sin tocar PID 1.
- **Capa 1** *(parcial, 2026-06-30)*: `set_cpu_weight`/`freeze` cableados
  **end-to-end** por Linux — antes existían en el contrato pero el único Engine
  real (`ArjeEngine` sobre arje-bus) los dejaba en `Unsupported`. Ahora
  `arje-bus` lleva `SetCpuWeight`/`Freeze` (agregados **al final** del enum: el
  wire postcard numera por posición y hammer espeja discriminantes), arje-zero
  los atiende escribiendo `cpu.weight`/`cgroup.freeze` con el escritor canónico
  `arje_incarnate::cgroup` (mismo que `LocalEngine`), gateados por identidad
  autenticada y anclados en la cadena de auditoría (`AuditAction::Cgroup`). Con
  esto los tres Engines de Linux (`LocalEngine`, `DaemonEngine`, `ArjeEngine`)
  honran prioridad/freeze, y la capa 3 ya tiene a quién pedirle de verdad.
  **`restart`** ✅ (2026-06-30): verbo del contrato (default `Unsupported`),
  implementado en `LocalEngine` (retiene el `intent` en `Entity` → stop+run del
  mismo, mismo `card_id`) y `DaemonEngine` (passthrough, `DaemonRequest::Restart`
  al final del enum). `ArjeEngine` lo deja `Unsupported` a propósito: en arje el
  reinicio de una unidad supervisada lo gobierna su `RestartPolicy`; un restart
  by-id por el bus (kill+re-encarnar del grafo) es trabajo de PID 1 pendiente.
  **Pendiente de capa 1**: restart en arje, `io.weight`, `memory.high/max`,
  `cpuset` (pin), `nice`/`ionice`, `set_ttl`/`enable`.
- **Capa 2** *(núcleo, 2026-06-30)*: motor de disparadores por **métrica** en
  `sandokan-monitor-core::reglas` (puro, como `energia`): `Condicion`
  (CpuPctMin/MemBytesMin/RestartsMin/EtiquetaContiene/Todas) + `ReglaMetrica`
  con `durante` (sostenido-por-tiempo) → `MotorMetrico::evaluar(snapshot, dt)`
  que acumula rachas entre polls, dispara al cruzar el umbral **una sola vez**
  (debounce) y resetea al caer la condición. El `Disparo` se ejecuta por el
  contrato vía `aplicar()` → `Engine::{stop,set_cpu_weight,freeze}` (los verbos
  de la capa 1). Evalúa el **mismo `MonitorSnapshot`** que el monitor — sin
  segunda fuente. **Disparadores de estado del sistema** ✅ (2026-06-30):
  `EstadoSistema` (en_bateria/bateria_pct/red/idle) + `CondicionSistema`
  (EnBateria/EnCorriente/BateriaMenorQue/SinRed/IdleMayorQue + Todas/Cualquiera)
  + `ReglaSistema` → `evaluar_sistema` (puro) + `aplicar_sistema`; el I/O de
  sensar vive en el borde (el caller pasa el estado, como `energia`). El
  Vigilante los corre en `tick_sistema(estado)` con set hot-swap
  (`armar_sistema`). **Pendiente de capa 2**: disparadores de **tiempo**
  (cron/atardecer/boot) — son un *scheduler*, no una condición de snapshot.
- **Lazo vivo** *(2026-06-30)*: `sandokan-vigilante::Vigilante` corre la capa 2 —
  cada `intervalo`: `observe(engine) → MotorMetrico::evaluar → aplicar(&engine)`,
  todo por el contrato. Reglas **hot-swappables** (`armar(reglas)`): el gancho
  donde la capa 4 enchufa (una intención arma su set al entrar). Test de lazo
  cerrado: una unidad a 95 % CPU dispara y el `stop` llega al Engine mock.
- **Capa 4** *(seam, 2026-06-30)*: la capa de intención **ya existía** —`pacha`
  (contextos): cada `Pacha` reconcilia overlay/vista/apps/`cpu_weight`/freeze/hide
  vía `Effect`s que `pacha-manager` aplica sobre el `Engine` (`LinuxSurfaces`).
  No se reinventó. El **seam con la capa 2** ya está: una intención **arma sus
  reglas de métrica** al enfocarse y las desarma al salir. Decisión de capas:
  el binding `contexto → Vec<ReglaMetrica>` vive en `pacha-manager` (no en
  `pacha-core`, que es **agnóstico de sandokan por diseño** —su propia
  descripción lo dice—). `Surfaces::armar_reglas` + `Manager::con_reglas` +
  `Manager::switch/close` arman/desarman; `LinuxSurfaces` enruta al `Vigilante`
  real (inyectado por el daemon vía `with_vigilante`). Probado con el `Recorder`
  (switch arma n=1, contexto sin reglas n=0, close desarma). El **daemon ya lo
  enciende**: `LinuxSurfaces::connect_vigilado()` construye el `Vigilante` con un
  handle propio al orquestador, spawnea su `correr()` (poll cada 2 s) y lo
  cablea; `paths::load_reglas()` lee el binding `contexto → [ReglaMetrica]` de
  `~/.config/pacha/reglas.ron` (aparte del catálogo, que es agnóstico de
  sandokan), y `pacha-cli run_daemon` hace `Manager::con_reglas(...)`. **Gaps
  conocidos**: (1) si el daemon arranca con un contexto ya activo, sus reglas no
  se arman hasta el próximo switch; (2) el `Vigilante` por métrica sólo es útil
  con un Engine que muestree CPU (`LocalEngine`/`DaemonEngine` sí; `ArjeEngine`
  da `cpu_pct = 0`, así que las reglas de CPU no disparan vía el engine de
  sistema hasta que arje-zero muestree `/proc` en `EnteTelemetry`).
- Capa 5: pendiente.

## Estado (2026-05-31)

### Hecho
- Crates del plano de control: `sandokan-core` (contrato `Engine` { run, stop, list, status, telemetry } + intent/event/error), `sandokan-lifecycle` (Backoff, RestartPolicy, RestartTracker, LifecycleState, quota, ttl), `sandokan-local` (LocalEngine no-PID1), `sandokan-daemon` (DaemonEngine + protocolo wire), `sandokan-remote` (RemoteEngine vía SSH socket-forward), `sandokan` umbrella (`Engine::auto()`), `sandokan-app` (CLI de prueba).
- Dedup #1 ✅ (backoff unificado: `arje-zero` adopta `sandokan_lifecycle::Backoff`). Dedup #3 ✅ núcleo (`arje-bus` gana `EnteStatus`/`EnteTelemetry`; el control viaja por el bus del init). Dedup #2 ✅ vía bridge (`sandokan-arje-engine` implementa `Engine` hablando arje-bus a arje-zero, que queda bus-native).
- Process monitor: `sandokan-monitor-core` ✅ (Fase 1, `observe(&dyn Engine) -> MonitorSnapshot`, cara de sólo-lectura agnóstica de transporte) + restarts visibles end-to-end ✅ (Fase 2/3, `↻N` en `arje-card-llimphi`).
- **App dedicada `sandokan-monitor-llimphi`** ✅ 2026-06-01 (frontend Fase 3, variante "app propia"): monitor de procesos sobre Llimphi, **tres pestañas**. (1) **Sistema**: todos los procesos del SO leídos de `/proc` (módulo `procfs`), tabla virtualizada con %CPU (delta de jiffies)/%MEM/RSS/estado/hilos/uid/**tiempo de vida** (uptime del proceso)/comando, orden por columna, y señales reales (`Terminar`/`Matar`/`Pausar`/`Seguir` vía `nix::sys::signal::kill`). Con **árbol padre/hijo** (toggle Lista/Árbol): jerarquía por `ppid` aplanada DFS, nodos colapsables (triángulo o ←/→), sangría por profundidad. **Filtro/búsqueda** incremental por nombre/comando/PID (`/` o Ctrl+F; con filtro activo cae a lista plana de coincidencias, estilo htop). **Gráficos en el tope**: un gráfico de %uso **por core** (delta busy/total de cada `cpuN` en `/proc/stat`, **ordenados por número**, línea **coloreada por nivel** verde→ámbar→rojo) + uno de memoria usada, área + línea vía `paint_with`, ~2 min a 1 Hz, en FlexWrap. La columna comando rellena el espacio disponible (flex), texto a la izquierda y se pica con `...` medido pixel-exacto en `paint_with`; el triángulo del árbol va dibujado (no glifo de fuente). (2) **Unidades**: las unidades del plano de control, observadas SOLO por el contrato `Engine` (`observe()` sobre `auto_default()`), tarjetas vivas con sparkline de CPU; detener/matar por `Engine::stop`. (3) **Wawa**: censo host-side de las apps WASM instaladas. Bindings de teclado reales (`on_key`/`on_wheel`: Tab cicla, ↑↓ navegan, Supr termina, Ctrl+1/2/3, Ctrl+R/F5/Ctrl+Q). `SANDOKAN_MONITOR_SEED=1` siembra unidades reales para demo. Binario: `sandokan-monitor`.
  - **Mapa (treemap fractal)** ✅ 2026-06-01: pestaña con un **treemap jerárquico** de los procesos del SO — cada proceso es un rectángulo de área proporcional a su memoria (RSS) o CPU (toggle), anidado por padre/hijo (slice-and-dice recursivo, módulo `treemap` puro y testeado), **coloreado por proceso** (paleta categórica estable por nombre, compartida con la lista; opacidad sube con el uso de CPU, baja con la profundidad; **gradiente vertical leve** por celda; cada recuadro con espacio muestra **nombre + %CPU · RAM**). **Interactivo**: click selecciona (hit-test recomputando el layout en coords locales) y la barra ofrece Terminar/Matar (o Supr); **doble-click hace zoom al subárbol** (detección por `Instant`), con breadcrumb Subir/Todo y Backspace para subir. Misma fuente `/proc` que el modo Sistema.
  - **Nota de límites (SDD §6):** el modo **Sistema** lee `/proc` directo a propósito —es el SO entero, una fuente sin dueño en el control plane—. NO viola "una sola fuente de verdad": esa regla aplica a las **unidades gestionadas**, que siguen observándose por el `Engine` (pestaña Unidades). Sistema y Unidades son fuentes distintas que no se pisan.
- **Veto de suspensión `energia`** ✅ 2026-06-30 (segundo consumidor de lectura, §6.bis): `sandokan-monitor-core::energia::evaluar` juzga el `MonitorSnapshot` para que `pata` no suspenda por inactividad si hay trabajo (unidad ocupada o keep-awake por label). 5 tests. Límite: vía arje-bus `cpu_pct=0`, así que el veto real es por label; el "sistema ocupado" no-unidad lo cubre el consumidor con `/proc/loadavg`.

### Pendiente
- **`RunCard{card}` arbitraria** *(en curso)*: el `Engine::run` transmite la `Card` entera por el bus (`BusRequest::RunCard{card: WireCard}`) en vez de pedirla del store por nombre. Modelo de confianza fijado (2026-06-05): **gate por `Capability::Spawn` del caller + caller como requester** — sólo Entes con `Spawn` pueden usarlo y la card se encarna con las caps del caller (no de la Semilla), así que es imposible escalar privilegios. `SpawnCardFromDisk` sigue existiendo para el caso store-based (timers, systemd1-compat).
- Monitor Fase 4 (lado Wawa): leer censo del executor + balizas del compositor — pieza futura, fuera de `sandokan-monitor-core`.

### Resuelto / reconciliado (2026-06-05)
- ~~`RestartTracker::count` en `LocalEngine`~~ **✅ ya estaba hecho**: `sandokan-local::telemetry` lee `tracker.count()` (incrementado en `on_failure`); test `telemetry_cuenta_restarts_en_salida_anomala`. La entrada anterior estaba stale.
- ~~Deprecar/borrar el socket de `sandokan-daemon`~~ **❌ NO procede**: la premisa "redundante, sin consumidores" era falsa. `sandokan-daemon` tiene consumidores vivos — `sandokan::auto()` lo usa como **tier 2** (embedding horizontal sobre `LocalEngine` no-PID1), `sandokan-remote` **reusa su `protocol`** (`DaemonRequest/Response`) para engines por SSH, y `sandokan-app daemon` lo sirve. No es redundante con `arje-bus`: arje-bus frontea a arje-zero (PID 1); el daemon frontea un `LocalEngine` para sesiones shuma/sandboxes/tests. Se mantiene.
