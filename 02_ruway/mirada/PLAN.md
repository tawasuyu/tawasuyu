# PLAN — evolución del compositor mirada

Plan de trabajo para mirada (el compositor Wayland de la suite). Nace de un
brainstorm grande sobre "ideas disruptivas de escritorio". El brainstorm venía
escrito sobre una arquitectura **que no es la nuestra** (asumía "Cerebro en
GPUI", `pata` = launcher, `arje-zero` + `seatd` como DM, layout fractal ya
existente, sandboxes/CRIU) — todo eso es falso aquí. Este documento es el
veredicto destilado: qué se descarta, qué se construye, y en qué orden.

## Estado real de mirada (verificado contra el código, 2026-06-08)

- **Es un compositor Wayland real** sobre **smithay 0.7** (`mirada-compositor`),
  con backend winit (anidado, para desarrollo) y DRM/KMS (bare-metal, vía
  `libseat`/GBM/EGL/`libinput`).
- **Split Cerebro/Cuerpo limpio** (`mirada-protocol`: `BrainCommand`/`BodyEvent`
  sobre postcard, prefijo `u32` LE). Éste es el activo arquitectónico: la
  política (sesión, seguridad, contexto) vive en una state-machine pura y
  testeable (`mirada-brain::Desktop`), agnóstica del hardware.
- **`pata` es el shell** (cliente wlr-layer-shell, barras de borde), **no** el
  cerebro. El cerebro es `mirada-brain`.
- **Layout plano**: `Workspace { windows: Vec<WindowId>, floating, fullscreen }`
  con 7 modos de teselado. Las "zonas" (`ZoneFrac`) sólo existen como blancos de
  arrastre. **No hay** árbol recursivo / fractal.
- **9 escritorios virtuales** fijos; cada salida muestra uno. Hay una **vista
  espacial** ("Prezi") que pinta miniaturas de todos (`workspace_layouts`).
- **No hay** sandboxing, jails, CRIU, ni persistencia de sesión.

## El cuello de botella

Cinco de las ideas valiosas (zoom-Z, sub-pantallas, contextos por rama, alt-tab
por grafo, persistencia posicional) dependen de **dos piezas que no existen**:

1. **Persistencia de sesión** — serializar/restaurar el estado del `Desktop`.
   Es el agujero #1 de Wayland y nuestro split lo hace casi gratis.
2. **`LayoutNode` recursivo** en `mirada-layout` — que un nodo pueda ser una app
   *o* un sub-escritorio. Hoy el layout es plano.

Se construyen primero; todo lo demás se cuelga de ellas.

---

## Fase 1 — Persistencia de sesión  ✅ HECHA

**Qué.** Que el escritorio recuerde su forma entre arranques: el modo/ratio/
nmaster/gap de cada escritorio virtual, qué escritorio mostraba cada salida y
cuál tenía el foco. **No** persiste las ventanas vivas: sus `WindowId` son
efímeros (los clientes Wayland se reconectan con otros ids), así que sobrevive
la *forma* del escritorio, no la geometría por-ventana. (Restaurar ventanas
concretas — respawn por `app_id` — es Fase 1.bis, ver abajo.)

**Cómo.**
- `mirada-brain/src/session.rs`: `DesktopState` (serializable a RON), con
  `to_ron`/`from_ron`, `default_path` (`~/.local/share/mirada/session.ron`),
  `save` (atómico tmp+rename), `load`, `load_if_present`. Versionado
  (`SESSION_VERSION`) para migrar sin romper.
- `Desktop::snapshot() -> DesktopState` y `Desktop::restore(&DesktopState)`.
  `restore` re-aplica los params a cada escritorio y guarda el mapa
  salida→escritorio en pendiente (al restaurar en el arranque aún no hay
  salidas conectadas); se aplica en `OutputAdded` según el orden de aparición.
  Va **después** de `set_config` (la sesión manda sobre la config sembrada).
- `mirada-app-llimphi`: carga la sesión al arrancar y la guarda
  **on-change** en el tick (sólo si cambió y sólo con Cuerpo conectado, para
  no pisar la sesión real desde el modo simulación).

**Hecho cuando:** `cargo test -p mirada-brain` cubre snapshot↔restore y el
round-trip RON; al reiniciar el Cerebro contra un Cuerpo vivo, los modos por
escritorio y el mapa salida→escritorio se recuperan.

### Fase 1.bis — Restaurar ventanas por `app_id`  ✅ HECHA
`DesktopState.window_homes` recuerda, por `app_id`, en qué escritorio vivía cada
ventana; al **reaparecer** (reabrir la app o reconectar el Cuerpo) vuelve ahí.
No respawnea: sólo enruta lo que se reabre. El hogar se consume una vez (no fija
las ventanas posteriores) y las `Rules` mandan sobre él. (Respawn automático por
`app_id` queda fuera: `app_id` ≠ ejecutable, es frágil.)

---

## Fase 2 — `LayoutNode` recursivo (árbol fractal)

**Primitivo  ✅ HECHO** (`mirada-layout/src/tree.rs`):
`enum LayoutNode { Leaf(WindowId), Space(Box<SpaceNode>) }` +
`SpaceNode { params, children }`. `SpaceNode::resolve(screen)` **aplana** el
árbol a `Vec<(WindowId, Rect)>` en píxeles absolutos — el `mirada-protocol` no
cambia (el Cuerpo sigue recibiendo una lista plana). Es additivo: no toca el
`Workspace` plano. Tests de geometría prueban que un árbol de un nivel resuelve
**idéntico** a `Workspace::layout` en los 7 modos, más el anidamiento. Compila
`no_std` (sólo `alloc`).

**Integración + zoom semántico  ✅ HECHA (primera rebanada):** el `Workspace`
gana una capa de agrupación opcional (`grouping: Option<SpaceNode>` + `view_path`,
ambos `#[serde(skip)]`), **additiva y apagada por defecto**: el camino plano es
byte-idéntico al de siempre (los 67 tests previos siguen verdes). `layout()`
reconcilia el árbol con `windows` (añade teseladas nuevas, poda las que se van) y,
con zoom activo, resuelve el sub-espacio en vista a pantalla completa. Acciones
nuevas: `GroupStack` (pliega la pila en un sub-espacio), `Ungroup`, `ZoomIn`,
`ZoomOut` (`Super+a`/`Super+Shift+a`/`Super+i`/`Super+u`). El `mirada-protocol`
no cambia: el Cuerpo sigue recibiendo una lista plana.

**Capas dormidas  ✅ HECHA (2ª rebanada):** al entrar en un sub-espacio, las
ventanas que quedan fuera de la vista ya no se ocultan *por omisión* sino que se
listan explícitamente con `WindowPlacement.suspended = true` (`Workspace::dormant`
las calcula con su rect del nivel superior). El Cuerpo (`mirada-body` →
`BodyOp::Configure.suspended` → `ManagedWindow.suspended`) les **corta los frame
callbacks** en ambos backends (winit y DRM): el cliente bloquea su bucle y deja de
pintar a ciegas, en vez de seguir consumiendo GPU detrás del zoom. 3 tests de
`dormant` en layout + 1 en protocol + 1 en body + el de integración en brain.

**Multinivel  ✅ HECHA (3ª rebanada):** el árbol ya es genuinamente fractal.
`Workspace::group` pliega **dentro del sub-espacio en vista** (no siempre en la
raíz), con `view_leaves` exponiendo las hojas sueltas del nivel actual y
`GroupStack` tomando su pila de ahí: estando dentro de un grupo se puede plegar
otra vez y entrar más profundo, a nivel arbitrario (`zoom_in`/`zoom_out` ya
navegaban cualquier profundidad por `view_path`). La app pinta un chip `⧉ N` en
la barra con la profundidad de zoom cuando hay agrupación.

**Agrupación persistente  ✅ HECHA (4ª rebanada):** la forma del árbol fractal
sobrevive al reinicio. `DesktopState.groupings` guarda, por escritorio agrupado,
la forma del árbol anclada por `app_id` (`SpaceShape`/`NodeShape`, espejo de
`SpaceNode`/`LayoutNode` con `app_id` en las hojas). Al restaurar queda pendiente
y se **rematerializa** cuando todas las apps miembro reabren en su escritorio
(los `WindowId` nuevos se mapean por `app_id`, una ventana distinta por hoja). Si
alguna hoja no tiene `app_id`, ese escritorio no se persiste (no se podría
reconstruir fielmente). El zoom (view_path) arranca en el nivel superior.

**Constelaciones  ✅ HECHA (5ª rebanada):** agrupación dirigida por el grafo de
actividad. El Cuerpo reporta el linaje de proceso de cada ventana
(`BodyEvent::WindowLineage { id, pid, ancestors }`: PID por `SO_PEERCRED` del
socket al aceptar el cliente, ancestros caminando `/proc/<pid>/stat`). El Cerebro
(`mirada-brain/src/activity.rs`, `ActivityGraph` puro y testeable) parte las
ventanas en *constelaciones* — componentes conexas por linaje, así que la terminal
y el editor que lanzó caen juntos aunque haya un shell intermedio sin ventana. La
acción `GroupConstellation` (`Super+Shift+c`) pliega la constelación de la ventana
enfocada en un sub-espacio del zoom-Z. Evento aditivo (no campo de `WindowOpened`)
para no romper la simulación; best-effort (sin PID → no se emite, la ventana es su
propia constelación).

**Zoom-Z completo.** Las cinco rebanadas están hechas: agrupar+entrar/salir,
capas dormidas, multinivel, persistencia, constelaciones. Además, encima del mismo
`ActivityGraph`, el **alt-tab por constelación** (`FocusConstellationNext/Prev`,
`Super+Tab`/`Super+Shift+Tab`): salta el foco entre familias de actividad, no entre
ventanas sueltas.

**Encima de Fase 2** (orden de ROI):

| Idea | Estado | Nota |
|---|---|---|
| **Zoom semántico en Z** | ✅ COMPLETO | Agrupar + entrar/salir + **capas dormidas** (el Cuerpo corta frames) + **multinivel** (anidar a profundidad arbitraria; chip ⧉N) + **persistencia** (la forma del árbol sobrevive al reinicio por `app_id`) + **constelaciones** (agrupar por linaje de proceso, `Super+Shift+c`). |
| **Alt-Tab por grafo de actividad** | ✅ HECHO | `FocusConstellationNext/Prev` (`Super+Tab`/`Super+Shift+Tab`) salta el foco entre constelaciones del `ActivityGraph`, no entre ventanas. |
| **Capabilities por ventana** | 🔨 EN CURSO (4ª rebanada hecha) | **Globals sensibles gateados por ejecutable.** Una capacidad denegada no se concede por una tabla eludible: el Cuerpo **no anuncia el global** al cliente (frontera física). Identidad = ejecutable real vía `SO_PEERCRED → /proc/<pid>/exe` (verdad del kernel, no el `app_id` falsificable). Política autorada en el Cerebro (`mirada-brain/src/permisos.rs`, RON en `~/.config/mirada/caps.ron`), empujada al Cuerpo por `BrainCommand::SetCapabilities` (espejo de `SetDecorations`) y aplicada en el filtro `Fn(&Client)->bool` del global. Postura: **permitir por defecto** (denylist por subcadena). **1ª rebanada:** clipboard (`zwlr_data_control`, `Permisos.clipboard_denylist`). **2ª rebanada:** inyección de pulsaciones (`zwp_virtual_keyboard`, `Permisos.virtual_input_denylist`) — global nuevo creado *gateado desde su nacimiento* con `VirtualKeyboardManagerState::new` y el mismo filtro por exe; sin él, cualquier cliente podría inyectar teclas (keylogger a la inversa). **3ª rebanada:** censo de ventanas (`ext_foreign_toplevel_list`, `Permisos.window_list_denylist`) — otro global nacido gateado (`ForeignToplevelListState::new_with_filter`); además mirada **gana el protocolo** (antes ni lo anunciaba): alta en `register_toplevel` (la ventana del shell no se censa), título/`app_id` espejados en `title_changed`/`app_id_changed`, baja con `closed` en `toplevel_destroyed` — taskbars/docks/switchers externos ya pueden listar ventanas, salvo los denegados. **4ª rebanada:** captura de pantalla (`zwlr_screencopy` v3, `Permisos.screencopy_denylist`) — la más sensible, e implementada **a mano** (`mirada-compositor/src/screencopy.rs`: smithay 0.7 sólo trae los bindings): `capture_output`/`capture_output_region` sobre `wl_shm` `Xrgb8888`, copia one-shot servida en la próxima composición — en winit se lee el backbuffer recién pintado; en DRM se re-componen los mismos elementos en un offscreen (`Offscreen<GlesTexture>` + `ExportMem`) porque el framebuffer real vive dentro del `DrmCompositor`. Orientación por flag `Y_INVERT` (el mapping GL sale bottom-up). Pendiente de verificar con `grim` en sesión gráfica. **Daño real para `copy_with_damage`** (2ª rebanada de screencopy): la captura queda retenida hasta que la salida acumule daño genuino — commits de toplevels (celda de la ventana, resolviendo subsuperficies por `get_parent`), re-teselados/foco/cierre/título desde `exec_op` y los handlers xdg, layer surfaces/menú raíz/cambio de modo como daño total — y el evento `damage` reporta el extents acumulado traducido al frame del cliente (`danio_en_frame`, pura y testeada ×5, con origen por salida para multi-monitor DRM); es lo que permite a wf-recorder grabar sin re-capturar cuadros idénticos. El cursor no acumula daño porque tampoco entra en la captura. Pendiente de verificar con wf-recorder en sesión gráfica. Próximas rebanadas screencopy: buffers dmabuf (zero-copy), `overlay_cursor` honrado en DRM, daño fino por rects del commit (hoy: celda entera). |
| **Throttle de frames** | ✅ HECHO (1ª rebanada) | Espaciar los `wl_surface.frame` callbacks de las ventanas **de fondo** (visibles pero sin foco, teseladas): pintan a 1 de cada N vblanks en vez de quemar GPU a 60 Hz detrás del foco. Config `background_frame_divisor` (default 1 = apagado, sin cambio). Misma forma que `suspended` del zoom-Z, cableada en paralelo: `WindowPlacement/Surface/Configure.frame_divisor` + contador `frame_tick` por ventana en ambos backends (winit + DRM); la política (enfocada/flotante/fullscreen = pleno ritmo) la aplica el Cerebro en `relayout`. La enfocada nunca se throttlea; las dormidas ya tienen el frame cortado del todo. Pendiente (nicho, diferido): override por `app_id` (apps abusivas concretas) y refresh-rate por ventana (vídeo a 144 Hz). |
| **Clipboard por zona** | ✅ HECHO (2026-06-26, sin verificar headless; flag `MIRADA_CLIPBOARD_POR_ZONA=1`) | Cada escritorio tiene su propio portapapeles de **texto**: lo copiado en "código" no lo lee una app de "comunicación". mirada (ya broker) **captura** la selección de un cliente al copiar (`new_selection` → `request_data_device_client_selection` por un pipe leído en un hilo, como el DnD) bajo la zona activa, y al cambiar de escritorio (`cambiar_workspace`) la **re-ofrece** como selección server-side (`set_data_device_selection`, con la zona como `SelectionUserData`); `send_selection` sirve los bytes guardados. Núcleo `zone_clipboard` (almacén zona→contenido + helpers de mime, 4 tests). Sólo texto; binario pasa sin tocar. El path con Cerebro **enlazado** (no embebido) aún no restaura en el switch remoto. **Verificar en sesión gráfica.** Historial en `pata` ya existía. |
| **Alt-Tab por grafo de actividad** | BUILD | Terminal lanzada desde el editor = "hija" (conocemos el linaje vía `Spawn`). Saltar entre constelaciones, no ventanas. Las constelaciones alimentan la agrupación del zoom-Z. |
| **Workspaces por rama de Git** | ✅ HECHO (2026-06-26, 1ª rebanada) | `MIRADA_GIT_WORKSPACE=<repo>` activa un vigía de `<repo>/.git/HEAD` (reusa `FileWatch`/inotify). Al cambiar de rama, mirada **guarda** la sesión actual bajo la rama vieja (`…/mirada/sessions/<rama>.ron`) y **restaura** la de la nueva — cada rama es un escritorio guardado (modos/ratio por workspace + `home` por `app_id`). Módulo `mirada_brain::git_branch` (`parse_git_head` + `branch_session_path` + `GitBranchWatch`, puros y testeados ×3); swap cableado en el `tick` de `mirada-app-llimphi`. Se intercambia la **forma** del escritorio; el respawn/SIGSTOP de los procesos por rama queda para Fase 1.bis (CRIU descartado). El brain embebido en `mirada-compositor` puede adoptar el mismo `swap_session_on_branch_change`. |
| **Remote vía waypipe** | ✅ HECHO (2026-06-26) | `mirada-ctl remote [user@]host <app> [args…]` arma `waypipe ssh <host> <app…>` (helper puro `waypipe_remote_cmd`, testeado) y lo manda como `DesktopAction::Spawn` — el Cuerpo lo corre con `sh -c` y la ventana remota llega como un cliente Wayland más. **No** se inventó protocolo. Requiere `waypipe` en ambos extremos; el túnel real se prueba en sesión gráfica. |
| **Sesiones waypipe en el diseño de escritorios** | ✅ HECHO (2026-06-26) | Una sesión remota se **declara** en `config.ron` como una app local más: lista `startup` de `mirada_brain::StartupApp` (`command` + `remote: "[user@]host"` opcional + `workspace`/`app_id`/`floating`/`fullscreen`). Si `remote` está puesto, el comando se envuelve con el helper compartido `waypipe_ssh_command` (el **mismo** que usa `mirada-ctl remote` — un único armador, testeado). Al arrancar, el compositor lanza cada entrada (`spawn_config_startup`, junto al archivo `autostart` clásico) y la **ubica** por la vía existente: `Config::startup_rules()` deriva una `Rule` por entrada con anclaje, que se concatena a `rules.ron` en `embedded_brain` (las del usuario ganan por «primera que case»). Así un `foot` en otra máquina aterriza fijo en el escritorio 3 igual que uno local. El túnel real se prueba en sesión gráfica. |
| **Afinado de latencia waypipe** | ✅ HECHO (2026-06-26) | `StartupApp` gana `ssh_port`/`ssh_key` (puerto e identidad ssh) y `compress`/`video`/`threads` (afinado de waypipe que baja latencia/ancho de banda: `--compress=lz4\|zstd`, `--video` H.264/VP9, `--threads`). El armador único pasa a `waypipe_command(tuning, port, key, host, command)`: banderas globales ANTES de `ssh`, `-p`/`-i` entre `ssh` y el host. `WaypipeTuning::flags()` puro; `waypipe_ssh_command` queda como atajo (lo usa `mirada-ctl remote`). +5 tests. |
| **Editor de sesiones remotas en wawa-panel** | ✅ HECHO (2026-06-26) | Diente **Inicio** → sección «Sesiones remotas (waypipe)»: lista el `startup` con aviso de disponibilidad de `waypipe`/`ssh` en el PATH; tocar una sesión (o «＋ nueva») abre una **subventana** (overlay con scrim, como el file-picker) con el formulario completo (comando, host, puerto/clave ssh, escritorio, app_id, compresión/vídeo/hilos, flotante/fullscreen) + vista previa del comando y guardar/borrar/cancelar. Al guardar se vuelca a `mirada.startup` y persiste (`flush_saves`). Lógica pura en `wawa-panel-llimphi/src/remote.rs` (RemoteEdit + Schema allichay + detección de binarios), con tests; el overlay reusa `schema_panel`. |

---

## Bloqueo de sesión (lock) — el shell de credenciales reentrante  ✅ HECHO (2026-06-26)

**Idea rectora:** el **greeter y el lock son el mismo artefacto** — un *shell de
credenciales* que se compone *encima* de cero-o-más sesiones. Boot con 0 sesiones
⇒ greeter (login). Sobre una sesión viva, por atajo (`Super+Escape`) ⇒ lock. No es
un congelamiento global: la sesión sigue **residente** debajo, el shell es un
overlay reentrante (a diferencia del flip `Greeter→Session`, de una sola vía).

**Reuso total de pantalla:** el lock es el **mismo binario `mirada-greeter`** con
`--lock` (usuario fijo = dueño de la sesión vía `MIRADA_LOCK_USER`, sin selector
de sesión, botón «Desbloquear», emite `ShellAction::Unlock` en vez de un ticket).
Misma tarjeta, mismos fondos animados, mismo **reloj grande** (agregado a la `view`,
aparece en login y lock). El compositor lo compone igual que el greeter de boot —lo
detecta por `app_id == "mirada.greeter"`— con `is_greeter` al **frente del z-order**
(tapa la sesión, incluida la pata) y le **rutea todo el input**.

**Seguridad del candado:** mientras `shell_activo()` (greeter o lock), el filtro de
teclado **no dispara ningún atajo de sesión** (switchers, overview, grabs) — sólo
quedan VT-switch y la salida de emergencia; todo lo demás va al shell. Sin esto,
`Super+q` cerraría una ventana detrás del lock.

**Arquitectura preparada para multisesión (FUS), sin construirla aún.** Se
generalizaron los tres chokepoints single-session a forma de N, arrancando con N=1:
1. `BodyMode::{Greeter,Session}` (one-way) → se le sumó `Locked` (overlay
   reentrante) + el helper `App::shell_activo()`.
2. `session_user: Option<UserInfo>` + `session_env` → `App::sessions: Vec<Session>`
   + `active_session: Option<usize>` (hoy 0..1), con accesores `active_user`/`active_env`.
3. El canal greeter→compositor pasó de un `SessionTicket` pelado a
   `auth_core::ShellAction { StartSession | Unlock | Cancel }` — el seam que deja
   crecer a «saltar entre sesiones» sin reescribir el contrato. El emisor del canal
   se crea **siempre** (no sólo en modo DM) para que el lock pida el shell en runtime.

El compositor **no hace `setuid` de sí mismo** (se queda con sus privilegios y
lanza los clientes de cada sesión rebajados a su usuario) — la precondición que
habilita hostear varias sesiones.

**Diferido a FUS (anotado, no hecho):** arrancar/multiplexar >1 sesión concurrente;
el selector «cambiar usuario» en el lock (`ShellAction` ganaría `SwitchTo(SessionId)`
y `Unlock` un destino); y si la orquestación multi-seat termina en `sandokan`.
**Auto-lock por inactividad** (`ext_idle_notify_v1`) sigue pendiente — requiere
unificar el tipo de estado del bucle DRM (TODO en `setup.rs`), un refactor aparte.
**Caveat N=1:** si una app de la sesión abriera una ventana justo mientras está
bloqueada, se la marcaría `is_greeter` por error (glitch transitorio, se resuelve al
desbloquear).

## Diferido (implementable, caro/nicho — no ahora)

- **Compositor anidado real** (mirada dentro de mirada). Smithay lo soporta;
  pesado (un compositor por nido). El "fractal" barato es Fase 2; el anidado
  real se reserva para aislamiento duro.
- **Notification firewall focus-aware** — daemon propio + DND consciente del
  foco + pulso/shake de borde para críticas (barato; sirve a a11y). El "los
  bytes se retienen en el socket" del brainstorm está mal.
- **Refresh-rate por ventana** (terminal 30Hz, vídeo 144Hz vía subsurface). El
  subconjunto tratable del "frame-perfect scheduling"; el resto (latencia
  negativa, predicción de movimiento) es un tarpit.
- **Multi-seat simétrico** (dos ratones/teclados). Smithay tiene `Seat`
  múltiple, pero es mucho ruteo de input para un caso nicho.
- **AT-SPI / input-method / virtual-keyboard** (a11y). Estándar, eventual. Para
  "control por voz" ya está `mirada-asistente-llimphi` (NL→comando).

## Descartado (basura, theater, o fuera del scope del compositor)

- **CRIU pre-emptivo de apps GUI** — frágil (contextos GPU/FDs/sockets). La
  versión real: PSI + SIGSTOP + throttle de frames.
- **Buffers `wl_shm` cifrados** — security theater: ambos extremos necesitan el
  plano para pintar; cuesta cripto por frame y no aporta nada local.
- **Snapshot Btrfs/ZFS por workspace** — eso es NixOS/snapper; va en la capa de
  sistema (`arje`), nunca en el compositor.
- **Escritorio autopoiético** (lee el repo y levanta DBs) — es un devcontainer;
  a lo sumo un "template de proyecto" disparado a mano.
- **UI líquida / ventanas no-rectangulares para apps ajenas** — no se puede
  reformar útilmente una app GTK ajena. La "burbuja de inspección de variable"
  pertenece **dentro de pluma (Llimphi)**, no en mirada.
- **Eye/blink-tracking, focus-by-proximity, binaural edges** — rabbit hole de
  hardware el primero; gimmicks desorientadores los otros.
