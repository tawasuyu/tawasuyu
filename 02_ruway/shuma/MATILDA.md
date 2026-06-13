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

### M1. Acciones por contenedor (lifecycle dirigido) — la más pedida
Hoy todo es reconciliación todo-o-nada. Falta operar **un** recurso: filas
clickeables → menú `start / stop / restart / logs / inspect / rm`. La
ejecución reusa el patrón de `matilda-apply` (`docker start <n>` etc.);
local sincrónico, remoto por `matilda-linker`. Sin esto, "administrar
contenedores" obliga a bajar a la terminal.

### M2. Logs y stats en vivo
- `docker logs --tail N -f <n>` como stream a una card (reusa la superficie
  de streaming de shuma — el `%cN` y las secciones ya existen).
- `docker stats --no-stream --format …` → CPU/mem/red por contenedor,
  parseado como `RuntimeState` extendido. El monitor pasa de "#up" a series
  reales de CPU/mem (el `MonitorSpec` ya soporta history + sparkline).

### M3. Servicios systemd (el "servicios" que falta en el modelo)
`matilda-core` modela contenedores y vhosts, **no servicios**. Agregar
`Service { name, enabled, state }` + discover por `systemctl
list-units --type=service` (el detector de tabla genérico de shuma ya parsea
esa salida) + acciones `start/stop/enable/restart`. Cierra "administrar
servicios".

### M4. Polling periódico real
El `MonitorSpec` tiene `period_secs: 5.0` pero hoy el runtime sólo se
refresca al pulsar Discover. El chasis debería `spawn_periodic` un
`discover_runtime()` (local) / `docker ps` por SSH (remoto) y dispatchar
`Msg::SetRuntime` — el monitoreo se vuelve **vivo**, no a-pedido.

### M5. Multi-host fan-out
`Inventory` ya tiene N hosts pero discover/apply apuntan al único `Source`.
Falta iterar hosts y agregar el runtime de todos (una grilla "host × estado").
Es el salto de "una caja" a "una flota".

### M6. Drift visible en la UI
El drift ya se calcula (`container_drift`) pero sólo aparece como un `Update`
en el plan. Marcar la fila del contenedor desviado con un chip "⚠ drift:
imagen" para que el operador lo vea sin leer el plan.

## Orden sugerido

1. **M1 + M4** (acciones por contenedor + polling): convierten el tab de
   "visor declarativo" en "consola de operación viva". Máxima palanca.
2. **M2** (logs/stats): el monitoreo deja de ser binario up/down.
3. **M3** (systemd): suma la dimensión "servicios" que el nombre promete.
4. **M5 + M6** (flota + drift visible): escala y pulido.
