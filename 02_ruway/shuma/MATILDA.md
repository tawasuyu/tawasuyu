# MATILDA.md — el bloque de matilda como superficie de administración

> Análisis 2026-06-13. matilda = administración **declarativa** de
> servidores (contenedores Docker + vhosts de proxy reverso), montada como
> tab del chasis de shuma (`sandbox/shuma-module-matilda`). Este documento
> separa lo que ya hace de lo que falta para "administrar
> servidores/servicios/contenedores/monitoreo efectivamente desde shuma".

## Qué es hoy (verificado contra el código)

matilda es un reconciliador deseado-vs-actual, tipo NixOS/Ansible mínimo:

| Capa | Crate | Hace |
|---|---|---|
| Modelo declarativo | `matilda-core` | `Inventory { hosts, containers, vhosts }`; `Container { image, ports, env, volumes, restart }` |
| Observación | `matilda-discover` | lee `docker ps` + `/etc/nginx/sites-enabled`; **drift** real por `docker inspect` (imagen/puerto/env/volumen/restart) |
| Diff | `matilda-plan` | `actual → deseado` → `Vec<Action>` (Create/Update/Remove) ordenado por dependencia |
| Ejecución | `matilda-apply` / `matilda-ghost` | aplica / dry-run; cada paso loguea |
| Transporte | `matilda-linker` | SSH (discover + apply remotos) |
| Carga | `matilda-config` | `matilda.toml` + includes |
| UI | `shuma-module-matilda` | tab inventario\|plan+log, shortcuts Discover/Plan/Dry-run/Apply/Reload, monitores |

El flujo declarativo (discover→plan→dry-run→apply, local y por SSH) **está
completo y es sólido**. El drift detection ya existe (no era obvio desde los
LEEME). Lo que faltaba no es *reconciliación* sino *operación en vivo*.

## El eje nuevo: monitoreo runtime (arrancado 2026-06-13)

El monitor del bloque sólo contaba "pasos de plan pendientes" — útil para
saber si el servidor está al día, inútil para saber si **algo se cayó**.
Primer ladrillo entregado:

- `matilda-discover`: `RunState` (running/exited/paused/…), `ContainerStatus
  { name, image, state, status, ports }`, `RuntimeState` (con `up_count`/
  `down_count`/`container(name)`), `parse_docker_ps` (formato rico tab-
  separado `DOCKER_PS_FORMAT`) y `discover_runtime()` local. Puro + testeado.
- `shuma-module-matilda`: `State.runtime`, `Msg::SetRuntime`, el `Discover`
  local captura runtime además del inventario, el panel pinta cada contenedor
  con semáforo (`●` vivo / `○` parado, coloreado) + el `status` de Docker,
  lista **huérfanos** (corren fuera del inventario), y un segundo monitor
  `matilda · up` samplea `(up, down)`. Verificado headless
  (`examples/runtime_monitor.rs`).

## Lo que falta — roadmap para "administrar efectivamente"

Ordenado por palanca. Todo determinista; nada exige LLM.

### M1. Acciones por contenedor (lifecycle dirigido) ✅ (2026-06-13)
`matilda-apply::lifecycle::ContainerAction` (Start/Stop/Restart/Logs/Stats/
Remove) con `command()`/`is_mutating()` puros. El bloque hace las filas
clickeables → barra de acciones; ejecución local (`sh -c`, captura al log) +
`container_action_remote_blocking` (SSH) para el chasis; tras acción mutante
re-observa el runtime.

### M2. Logs y stats en vivo ✅ (2026-06-13, on-demand)
Acciones `Logs` (`docker logs --tail 200`) y `Stats` (`docker stats
--no-stream`) vuelcan al log del bloque. **Pendiente:** stream continuo
(`-f`) a una card y series de CPU/mem en el `MonitorSpec` (history+sparkline)
— hoy es a-pedido, no continuo.

### M3. Servicios systemd ✅ (2026-06-13, runtime + acciones + declarativos)
**Runtime:** `matilda-discover` `ServiceState`/`ServiceStatus` +
`parse_systemctl_units` + `discover_services()` (running,failed);
`RuntimeState.services`. El bloque muestra la sección SERVICES (semáforo
●/✖/○ + sub + descripción) con barra de acciones (`ServiceAction`:
start/stop/restart/enable/disable/status).
**Declarativos:** `matilda_core::Service { unit, enabled, active }` +
`Inventory.services`; `matilda-plan` `Resource::Service` (diff
Create/Update/Remove); `matilda-apply` genera los `systemctl
enable --now / disable / start / stop` (combina `--now` cuando enable+active
coinciden); `matilda-discover` consulta `is-enabled`/`is-active` por unidad
declarada para el drift (sólo administra las declaradas, no las cientos del
sistema). El panel lista "SERVICES declarados" con sus flags y si corren. El
loop declarar→plan→apply→runtime queda cerrado. **Pendiente:** discovery de
estado de servicios por SSH (remoto v1 los ve como Create, idempotente).

### M4. Polling periódico real ✅ (2026-06-13, local)
El chasis poll-ea `poll_runtime()` cada 5 s en un thread para las instancias
matilda Local (topbar/bottombar/main) → `Msg::SetRuntimeQuiet`. El semáforo
queda vivo sin pulsar Discover. **Pendiente:** polling remoto (SSH por tick).

### M5. Multi-host fan-out ✅ (2026-06-13, monitoreo de flota)
`matilda_core::Host` gana `user`/`port` SSH (default root/22). El bloque tiene
`fleet: BTreeMap<nombre, FleetEntry{Pending|Ready(RuntimeState)|Failed}>` +
`selected_host`; el shortcut **Fleet** hace que el chasis spawnee un thread
por host declarado (`host_runtime_remote_blocking`: SSH + `docker ps` +
`systemctl` + `ls sites-enabled`, reusando los parsers) y reenvíe
`SetHostRuntime`/`SetHostError`. La sección FLEET pinta cada host con
semáforo (●/◐/✖/◌) + resumen up/down/svc o el error, y al seleccionarlo
expande sus contenedores/servicios (grilla "host × estado", read-only).
**Pendiente:** acciones sobre recursos de un host de la flota (hoy operar va
por el `Source` montado) y polling de la flota (hoy es a-pedido por «Fleet»).

### M6. Drift visible en la UI ✅ (2026-06-13)
El contenedor que el discover marcó `(desviado)` lleva un chip `⚠ drift` en
su fila — el operador lo ve sin leer el plan.

## Estado

M1–M6 entregados 2026-06-13 (varios con el alcance acotado anotado arriba). El
tab pasó de "visor declarativo" a **consola de operación viva de una flota**:
ves qué corre y qué se cayó en cada host, operás el host montado sin bajar a la
terminal, y reconciliás contenedores/vhosts/servicios declarativamente. Lo que
queda son los "pendientes" acotados de cada M: stream de logs `-f` continuo +
series CPU/mem (M2), servicios remotos por SSH (M3), polling remoto (M4),
acciones sobre recursos de la flota + polling de flota (M5).
