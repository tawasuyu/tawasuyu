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

## Fase 1 — Persistencia de sesión  ← EN CURSO

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

### Fase 1.bis — Restaurar ventanas por `app_id` (después)
Persistir, por escritorio, los `app_id` que vivían ahí y, al arrancar,
respawnearlos (o re-ubicar los que reaparezcan) en su sitio. Solapa con
`Rules`; se diseña reusando ese mecanismo.

---

## Fase 2 — `LayoutNode` recursivo (árbol fractal)

**Qué.** Un nodo del layout puede ser una app **o** un sub-escritorio con sus
propias reglas de teselado. Es el primitivo del que cuelgan el zoom-Z y las
sub-pantallas con sus propias drop-zones.

**Cómo (boceto).**
- En `mirada-layout`, junto a `Workspace`, un
  `enum LayoutNode { Leaf(WindowId), Space(Box<Workspace>) }` (nombre por
  decidir), de modo que `Workspace::windows` pase a contener nodos. La
  resolución (`layout`/`tile`) aplana el árbol a `Vec<(WindowId, Rect)>` en
  píxeles absolutos — el `mirada-protocol` no cambia (el Cuerpo sigue recibiendo
  una lista plana de `WindowPlacement`).
- Cuidar la compatibilidad serde (variantes nuevas al final) y los 7 modos de
  teselado dentro de cada nivel.
- Refactor contenido pero transversal: toca `Workspace`, `tile`, `placements`,
  y los accesores del `Desktop`. Se hace con tests de geometría que prueben que
  el aplanado de un árbol de 1 nivel coincide con el layout plano actual.

**Encima de Fase 2** (orden de ROI):

| Idea | Estado | Nota |
|---|---|---|
| **Capabilities por ventana** | BUILD | Gatear screencopy/export-dmabuf por `app_id` en el Cuerpo. El sandboxing *real* y honesto: somos quien otorga el protocolo. |
| **Throttle de frames** | BUILD | Espaciar los `wl_surface.frame` callbacks de apps de fondo / abusivas. Reemplaza el fantasioso "CRIU pre-emptivo". |
| **Clipboard por zona** | BUILD | Somos el broker del clipboard: lo que se copia en "código" no lo lee el browser de "comunicación". Historial en `pata`. |
| **Zoom semántico en Z** | BUILD | *Feature insignia.* Entrar/salir del árbol de procesos (no escritorios laterales). Capas profundas inactivas → el Cuerpo suspende sus frames. |
| **Alt-Tab por grafo de actividad** | BUILD | Terminal lanzada desde el editor = "hija" (conocemos el linaje vía `Spawn`). Saltar entre constelaciones, no ventanas. |
| **Workspaces por rama de Git** | BUILD | `inotify` sobre `.git/HEAD` → swap de sesión guardada. SIGSTOP, **no** CRIU. Caso especial de Fase 1. |
| **Remote vía waypipe** | BUILD | `Spawn` que envuelve `waypipe ssh host app`. Para el compositor es un cliente más. **No** inventar protocolo. |

---

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
