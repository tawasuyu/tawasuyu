# SDD — Arranque sin parpadeo (splash nativo + crossfade a mirada)

Estado: **diseño + Fase 0/1 en curso** (2026-06-23). Decidido con el maker:
camino completo (splash nativo animado con crossfade), no sólo flicker-free
mínimo. Este documento es la fuente autoritativa; antes la decisión se había
perdido por no quedar escrita.

## Problema

Arrancando Linux, arje hoy pasa por una cadena que **parpadea**:

```
firmware (GOP) → kernel modo texto (efifb + logs de consola)
              → arje-zero PID 1 (texto: bus, atestación, génesis)
              → mirada-compositor --drm (reclama DRM, hace MODESET)  ← salto texto→gráfico
```

Cada eslabón puede blanquear/borrar la pantalla, y entremedio se ven los logs
de consola. El usuario ve: firmware → negro → texto con logs → negro → GUI.

## Meta

Que **el primer píxel sea gráfico** (lo pinta arje desde UEFI) y que de ahí
hasta el primer frame de mirada **no haya ni una caída a texto ni un modeset
visible** — un arranque continuo, con un splash animado nuestro que hace
crossfade al greeter. El equivalente de Plymouth, pero nativo, en Rust/Llimphi,
y propiedad nuestra de punta a punta.

## Principio clave: un solo modo, un solo framebuffer, sin re-modeset

El parpadeo nace de **cambiar de modo** (resolución/timing del CRTC) o de
**borrar** el framebuffer entre dueños. La estrategia es elegir el modo nativo
del panel UNA vez (en el loader, vía GOP) y que todos los dueños siguientes
—efifb/simpledrm, arje-splash, mirada— **reusen ese mismo modo** sin volver a
hacer modeset. El traspaso de dueño del DRM se hace sin tocar el modo, así el
scanout no se interrumpe.

## Arquitectura por fases

### Fase 0 — Base flicker-free (sin splash todavía)

1. **arje-loader (UEFI):** abrir el GOP, fijar el modo nativo del panel, y
   pintar el primer frame (fondo + logo). Así el primer píxel ya es gráfico y
   queda el modo elegido para que el kernel lo herede.
2. **cmdline del kernel:** `quiet loglevel=0 vt.global_cursor_default=0
   rd.systemd.show_status=false` + flags de takeover sin parpadeo del driver
   KMS (`i915.fastboot=1`, etc. según GPU). El objetivo: que el kernel NO
   escriba texto sobre el framebuffer y que el driver KMS herede el modo del
   GOP sin re-modeset (efifb → simpledrm handover).
   Lo emite `arje-installer::canonical_cmdline`.

Resultado de Fase 0: del encendido se ve el logo del loader y el kernel arranca
en silencio sin flash de texto. (La GUI real aparece cuando mira incarna.)

### Fase 1 — Splash nativo (`arje-splash`)

Un binario Rust, **Ente génesis de prioridad alta**, que arranca apenas
arje-zero monta el bus (antes que mirada). Abre el nodo DRM (simpledrm o el
KMS real), toma DRM master **reusando el modo vigente** (sin modeset → sin
parpadeo desde el logo del loader), y pinta un splash **animado** (logo +
progreso/respiración), idealmente con Llimphi sobre un dumb buffer DRM.

- Render: empezar simple (blit de framebuffer con una animación pura, estilo
  los fondos del greeter) y, si rinde, subir a Llimphi/vello sobre el buffer.
- Mantiene el splash hasta que mirada avisa «primer frame listo».

### Fase 2 — Crossfade / handoff a mirada

Dos clientes DRM no pueden ser master a la vez, así que un crossfade literal
píxel-a-píxel entre procesos no existe. El crossfade **percibido** se logra así:

1. mirada arranca y se coordina con arje-splash por un socket del bus
   (`arje-splash.sock` o señal por el bus de arje).
2. mirada inicializa todo y deja su **primer frame compuesto** (el greeter
   sobre su fondo) listo, pero aún no presenta.
3. arje-splash hace **fade-out de su contenido hacia el color/fondo que mirada
   va a mostrar** (no a negro), y al terminar suelta el DRM master.
4. mirada toma master **con el mismo modo** y presenta su frame ya compuesto.
   El greeter usa su animación de entrada de la tarjeta (ya existe). Efecto
   neto: splash → fondo común → tarjeta apareciendo = crossfade continuo, sin
   modeset ni negro.

El contrato de coordinación (borrador):
- `arje-splash` escucha en un socket Unix conocido.
- `mirada-compositor --drm`, en modo greeter, al estar listo manda `READY`;
  espera `RELEASED` antes de tomar DRM master; si no hay splash (timeout corto)
  sigue solo (degradación elegante).

## Panel de logs de arranque (automático)

Estilo «details» de Plymouth pero **automático**: el panel aparece sólo si el
arranque tarda de más (`ARJE_SPLASH_LOG_AFTER_MS`, def 6 s) o si el kernel
reporta un error (prioridad de syslog ≤ 3 en `/dev/kmsg`). Sin GL: `arje-splash`
lee `/dev/kmsg`, y dibuja el texto con la fuente bitmap 8×8 de dominio público
(`font8x8`) sobre el mismo dumb buffer (`logs.rs`). Se controla con `logs =
auto | off` en la config (wawa-panel). Best-effort: sin `/dev/kmsg`, no hay
panel. Verificado en QEMU (dispara por umbral; render unit-testeado).

## Relación con wawa

`wawa-kernel` **ya** es dueño del GOP y compone desde el frame cero (no tiene
este problema). Esto es sólo para el path **Linux** de arje. El loader puede
bootear wawa o Linux; el splash es del camino Linux.

## Verificación

Render del splash y cmdline con tests unitarios. La cadena DRM se certifica en
QEMU+OVMF con **evidencia de texto** (Regla 8): `scripts/test-arje-splash-qemu.sh`
compila estáticos (musl), empaqueta el initramfs y bootea headless capturando el
serial. Veredicto Fase 1 (2026-06-23, kernel host + OVMF, modo 1280x800):

```
[drm] Initialized simpledrm 1.0.0 for simple-framebuffer.0 on minor 0
INFO  arje_zero::graph::lifecycle: Ente encarnado label=arje-splash pid=Some(Pid(75))
[arje-splash] device=/dev/dri/card0 max_ms=8000 fps=30
[arje-splash] conector ... crtc ... modo 1280x800 — reusando modo vigente
[arje-splash] tope de 8000 ms alcanzado — soltando la pantalla
INFO  arje_zero::graph::lifecycle: Ente disuelto label=arje-splash status=Exit(0)
```

Sin warning de fallback → `page_flip` funcionó en `simpledrm` (camino vblank-sync,
sin re-modeset). Lo único no certificable en texto son los píxeles de la
animación; eso queda cubierto por los tests de `render.rs`. La observación visual
(cero parpadeo percibido, crossfade) se hace abriendo la ventana de QEMU
(`DISPLAY_ARGS= ./scripts/test-arje-splash-qemu.sh`).

**Handoff Fase 2** verificado con `arje-splash --poke` (mirada falsa) como Ente
extra del génesis. Secuencia observada (serial):

```
[arje-splash] conector ... modo 1280x800 — reusando modo vigente
[arje-splash] handoff escuchando en /run/arje-splash.sock
[arje-splash --poke] conectado; mando READY
[arje-splash] READY de mirada — fade-out + handoff
[arje-splash] RELEASED enviado — mirada toma la pantalla
[arje-splash --poke] respuesta: RELEASED
Ente disuelto label=arje-splash status=Exit(0)   (~0.44 s, no a los 8 s del tope)
```

El handoff corta el tope de tiempo: el splash suelta apenas mirada avisa. El
protocolo es idéntico al que implementa `mirada-compositor::handoff`, así que
queda certificado; la integración real del greeter sobre hardware es la
observación visual pendiente.

### Demo visual del crossfade (sin GPU): greeter simulado

El greeter real de mirada usa EGL/GLES y necesita una GPU en el guest, que QEMU
sin virgl no da. Para **ver el crossfade completo** igual, `arje-splash` trae un
greeter SIMULADO sobre DRM dumb-buffer (`--greeter-sim`, render
`paint_greeter`): hace de cliente del handoff, y al recibir `RELEASED` toma el
DRM reusando el mismo modo y hace **aparecer** la tarjeta de login desde `BG`.
El seed `seeds/arje-demo.card.json` encadena `arje-splash` + `greeter-sim`
(delay 3.5 s) + agetty. El script `scripts/test-arje-splash-qemu.sh` (portable:
encuentra OVMF/kernel/display solos) lo corre con ventana por defecto, o
`--headless` para el veredicto por texto. Secuencia observada: splash 3.5 s →
fade 0.4 s → la tarjeta del greeter aparece sobre el mismo `BG`, sin negro ni
re-modeset. Es una **demostración** del crossfade percibido; el greeter real lo
reemplaza cuando hay rootfs con Mesa.

### Dos gotchas de integración (encontrados al verificar)

1. **Binarios estáticos.** El initramfs no trae `libc.so`; arje-zero/splash deben
   compilarse para `x86_64-unknown-linux-musl` (o `+crt-static`). Con binarios
   glibc-dinámicos el kernel panica con *«No working init found»*.
2. **`requires` es un gate duro.** arje-zero rechaza encarnar un Ente con un
   `requires` sin provider registrado. Declarar `requires Device{Drm}` bloqueaba
   el splash (*«requires no satisfecho»*). El acceso a DRM es físico al device,
   no una capacidad brokeada — el splash va con `requires` vacío, igual que el
   display-manager de mirada.

## Verificación sobre hardware real (2026-06-24, Intel Iris Xe)

El greeter REAL usa EGL/GLES y necesita Mesa, así que la cadena
`arje-splash → mirada-greeter` sólo se certifica sobre una GPU de verdad (no
QEMU sin virgl). Script: `scripts/test-drm-greeter-metal.sh` (arranca el splash
y `mirada-compositor --drm --greeter` desde un VT libre, con instrumentación de
timestamps). Hallazgos:

1. **El traspaso de DRM master daba `EACCES` determinista** en el modeset de
   `DrmDevice::new`. Causa: el seat manager (seatd/logind) hace `SET_MASTER` al
   `session.open()` de un nodo DRM si la sesión está activa; mirada abría el
   device **antes** del handoff, mientras el splash todavía tenía el master
   (tomado directo, fuera del seat manager) → el `SET_MASTER` chocaba y el fd
   quedaba sin master para siempre. **Arreglo:** abrir el device vía libseat
   **después** del `RELEASED`, con el master ya libre. Verificado: cadena
   completa, sin `EACCES`. (commit `fc4bae7b`.)
2. **No es un blank de modeset.** El traspaso master→master con el modo
   reusado no apaga el panel; el gap es contenido, no un apagón.
3. **Artefactos del gap (lo que se ve entre el splash y la tarjeta):**
   - *Líneas horizontales random* = el bo GBM del primer scanout de mirada
     **sin inicializar**, visible durante todo el arranque del proceso greeter.
   - *Escalón de color* slate → púrpura: el splash funde a `BG (18,18,24)` pero
     el greeter pinta el wallpaper por defecto (gradiente). La premisa vieja
     («BG == bg_app del greeter») era falsa.

### Incremento 1 — present inmediato del fondo común (hecho)

Apenas se crea cada `DrmCompositor`, mirada hace un present de `CLEAR_COLOR`
(= `BG` del splash) **antes** de armar Wayland y lanzar el greeter. El primer
scanout pasa a ser un frame controlado en vez del bo sin inicializar → mata las
líneas random y empalma slate→slate sin costura. `CLEAR_COLOR` se unificó al
`BG` del splash. El wallpaper y la tarjeta del greeter se componen encima en el
bucle. Limitación: el wallpaper y la tarjeta siguen apareciendo como pasos
posteriores (no en el frame 1).

### Incremento 2 — sin re-modeset: `disable_connectors=false` (hecho, GAP 1500→389 ms)

Perfilando el gap con `LLIMPHI_TIMING` se vio que el `queue_frame` del primer
frame de mirada tomaba **~734 ms** y `DrmDevice::new` otros **~329 ms** — juntos,
**~1063 ms del gap eran apagar y re-encender el panel eDP**. Causa: mirada abría
el device con `DrmDevice::new(fd, disable_connectors=true)`, que **deshabilita
los conectores** (apaga el panel) en el init; el panel quedaba en **negro
profundo** hasta que el primer commit hacía el modeset de power-on (link-training
del eDP). Es el re-modeset que este SDD pide evitar.

**Fix:** `disable_connectors=false`. El splash dejó el panel **encendido** con su
último frame (`BG`) y el modo vigente; heredarlo deja la pantalla viva y hace que
el primer commit de mirada sea un **page-flip dentro del modo vigente**, no un
modeset con power-cycle. Verificado en metal (Iris Xe): el negro profundo
desaparece, `queue_frame` cae a **0 ms**, `device-listo` a **3 ms**, y el GAP
total baja de ~1500 ms a **389 ms**. Queda un parpadeo de un cuadro (page-flip
entre el framebuffer del splash y el de mirada) apenas perceptible.

`disable_connectors=false` se aplica **sólo cuando hubo handoff** del splash
(`esperar_release_del_splash()` devuelve `true` al recibir `RELEASED`). Sin
splash —cold boot o `mirada --drm` a mano— se mantiene `true`: takeover limpio
desde cero, el comportamiento de siempre del escritorio normal. Así heredar el
panel encendido es una optimización del camino con splash, sin tocar el resto.

### Incremento 3 — splash keep-alive: sin hueco de framebuffer (hecho)

El parpadeo residual del Incremento 2 era un **hueco de framebuffer**: el splash
cerraba su fd (destruyendo su FB) *antes* de que mirada presentara, así que el
CRTC quedaba un cuadro sin imagen válida. **Fix (lado splash):** en el handoff,
soltar sólo el master con `release_master_lock()` —no cerrar el fd— y mantener el
framebuffer del slate **vivo en scanout** una ventana corta (300 ms) mientras
mirada toma master y flipea su primer frame (mismo `BG`). Recién después el
splash suelta todo. Verificado en metal: el parpadeo desaparece, el slate es
continuo splash→mirada hasta que entra la tarjeta del greeter. Con esto el
crossfade percibido es **limpio** sin necesidad del cross-node.

### Fase 2-bis — crossfade limpio: render-node / card-node (pendiente, baja prioridad)

Con los Incrementos 1–3 el gap quedó en ~389 ms de slate continuo, sin negro ni
parpadeo, así que esta fase pasó a **baja prioridad**: sólo agregaría meter la
tarjeta del greeter en el frame 1 (hoy aparece ~unos cientos de ms después, sobre
el fondo común). Se deja documentada por completitud.

El cero-artefactos del SDD pide que el **primer scanout de mirada ya sea la
tarjeta del greeter compuesta sobre su fondo**. Pero hay un huevo-y-gallina:
componer con GL necesita el device, y el master lo tiene el splash hasta el
`RELEASED`. La única salida es **separar nodos**:

El cero-artefactos del SDD pide que el **primer scanout de mirada ya sea la
tarjeta del greeter compuesta sobre su fondo**. Pero hay un huevo-y-gallina:
componer con GL necesita el device, y el master lo tiene el splash hasta el
`RELEASED`. La única salida es **separar nodos**:

- Abrir el **render-node** (`/dev/dri/renderD128`, **no** necesita DRM master) y
  montar EGL/GLES ahí. Componer el primer frame del greeter **offscreen**
  mientras el splash sigue dueño del **card-node** (`/dev/dri/card1`).
- Para tener la tarjeta en ese primer frame hay que **bombear el bucle Wayland**
  (dispatch de clientes) hasta el primer commit del greeter, o un timeout corto,
  **sin presentar todavía**.
- Recién entonces hacer el handoff (`READY`/`RELEASED`), tomar master del
  card-node e **importar el frame offscreen por dmabuf cross-node** para el
  primer page-flip.

**Riesgo central (sólo visible en metal):** el scanout cross-node por dmabuf
depende de los **modifiers** del driver — un mismatch compila bien pero da
**pantalla negra** en runtime. Por eso esta fase se verifica únicamente sobre
hardware y se ataca en incrementos chicos. Métrica de cierre: comparar la
duración del gap (`epoch_ms` del primer `queue_frame` de mirada menos el del
`RELEASED` del splash, ya instrumentados) contra el baseline del Incremento 1.

## Estado de implementación

- [x] SDD (este documento)
- [x] Fase 0 — cmdline flicker-free (`arje-installer`)
- [x] Fase 0 — logo GOP en `arje-loader` (`gop::paint_boot_splash`, marca central placeholder; falta verificar en QEMU+OVMF)
- [x] Fase 1 — crate `arje-splash` (DRM dumb buffer + animación): render puro
  testeable (`render.rs`, 5 tests) + capa DRM best-effort (`drm_present.rs`)
  que reusa el modo vigente del CRTC (sin re-modeset), double-buffer con
  page-flip y fallback a `set_crtc`. Animación: respiración del logo de marca
  (misma paleta que el loader) + barra de progreso indeterminada. Suelta la
  pantalla por SIGTERM o por tope `ARJE_SPLASH_MAX_MS` (def 8 s). **Falta
  verificar en QEMU+OVMF** (no reproducible en el sandbox).
- [x] Fase 1 — Ente génesis de `arje-splash` en el seed: `priority: high`,
  `OneShot`, `requires Device{Drm}`. En `synthesize_dev_seed` (dev) y en los
  seeds canónicos `seeds/arje-{host,qemu}.card.json` — declarado **antes** del
  display-manager (host) / primero (qemu, para verificar el splash aislado).
- [x] Fase 2 — contrato de handoff splash↔mirada + crossfade. Socket Unix
  `/run/arje-splash.sock` (env `ARJE_SPLASH_SOCK`). Protocolo: mirada manda
  `READY` → splash hace fade-out al `BG` (400 ms), suelta el DRM master, y
  responde `RELEASED`; mirada espera ese `RELEASED` (timeout 3 s) antes de
  tomar master con libseat. Lado splash en `arje-splash::handoff` (con
  `--poke`, cliente de prueba); lado mirada en `mirada-compositor::handoff`
  (`esperar_release_del_splash`). Degradación elegante en ambos lados: sin
  socket / sin respuesta, cada uno sigue solo. **Verificado end-to-end en QEMU**
  (ver abajo); falta la observación visual del crossfade con el greeter real en
  hardware.
- [x] Fase 2 — robustez: el handoff se llama **justo antes de `DrmDeviceFd::new`**
  (la toma irreversible de master vía SET_MASTER), no en el paso 0. Así el splash
  sigue vivo durante libseat + GPU primaria + abrir el device, y solo suelta la
  pantalla en el punto irreversible; si esos pasos fallan, el splash (con su panel
  de logs) sigue visible. `mirada-compositor/src/drm_backend/mod.rs`.
- [ ] **Fase 3 — greeter/renderer antes del handoff (split render-node/card-node).**
  Diseñada (ver sección "Fase 3" abajo); sin implementar. Requiere verificación
  en GPU real.

## Fase 3 — Greeter compuesto antes del handoff (split render-node/card-node)

Estado: **diseñado, sin implementar** (2026-06-24). Requiere verificación en GPU
real (ver más abajo) — no reproducible en este entorno ni en QEMU sin virgl.

### El gap que falta cerrar

El handoff Fase 2 (`esperar_release_del_splash`) hoy ocurre **antes** del init de
GPU de mirada. Secuencia real (`mirada-compositor/src/drm_backend/mod.rs::run`):

```
libseat → udev::primary_gpu → session.open(card)
  → [HANDOFF: READY → splash fade-out → RELEASED]   ← splash suelta acá
  → DrmDeviceFd::new (SET_MASTER) → DrmDevice::new(fd, true)
  → enumerar salidas → GbmDevice + EGL + GlesRenderer  ← PARTE LENTA (shaders)
  → create_surface + DrmCompositor → primer tick → render_frame → queue_frame
```

Entre que el splash suelta y el primer `queue_frame`, la pantalla queda en el
`BG` estático (último frame del splash, que persiste tras soltar master). Esa
ventana la domina `GlesRenderer::new` (init EGL + compilación de shaders,
~cientos de ms). El SDD original (§ Fase 2, puntos 2–4) pide lo contrario:
*mirada compone su primer frame y recién entonces se hace el handoff*. Fase 3
cierra ese gap.

### Por qué obliga al split (verificado contra smithay 0.7.0)

- `DrmDeviceFd::new` (smithay `backend/drm/device/fd.rs`) llama
  `acquire_master_lock()` **al construirse**. Si falla porque otro proceso (el
  splash) ya es master, deja `privileged=false` **permanente**: `DrmDevice::
  activate()` solo re-adquiere master `if is_privileged()`. ⇒ no se puede
  construir el device del **card node** mientras el splash vive.
- Los handles GEM son **por-fd**. Para construir el `GlesRenderer` (la parte
  lenta) *antes* del handoff hay que hacerlo en **otro fd sin master** → el
  **render node** (`/dev/dri/renderD128`) → y cruzar los buffers de scanout al
  card node por **dmabuf**. No hay variante de un solo nodo. Es el camino
  multi-GPU de smithay (el de `anvil`).
- A favor: `DrmCompositor::render_frame` (smithay `backend/drm/compositor/
  mod.rs:1684`) **no** toca master (solo chequea `surface.is_active()`).
  `queue_frame` (íd. 2433) es lo único que hace el page-flip/commit KMS. El
  split render(sin master)/flip(con master) es limpio.

### Reordenamiento de `run()`

- **Fase A — sin master, splash vivo:**
  1. `LibSeatSession::new`, `udev::primary_gpu` → path del card node.
  2. Derivar render node: `DrmNode::from_path(card)?.node_with_type(NodeType::
     Render)` (crate `drm` 0.14, re-exportado por smithay como
     `backend::drm::{DrmNode, NodeType}`). Abrir su fd (no necesita master ni
     libseat — `/dev/dri/renderD*` no tiene concepto de master).
  3. `GbmDevice::new(render_fd)` → `EGLDisplay` → `EGLContext` →
     **`GlesRenderer::new`** (la parte lenta, ahora solapada con el splash).
  4. `build_app(greeter)` + `announce_dmabuf(&renderer)` (estado Wayland; sin
     GPU ni master).
- **Fase B — handoff:** `esperar_release_del_splash()`.
- **Fase C — con master:**
  5. `session.open(card)` → `DrmDeviceFd::new` (ahora **sí** agarra master) →
     `DrmDevice::new(fd, true)`.
  6. Enumerar salidas (conector/CRTC/modo — solo lectura, rápido).
  7. `GbmDevice::new(card_fd)` para el exporter.
  8. Por salida: `create_surface` + `DrmCompositor::new(mode, surface, None,`
     `allocator=GbmAllocator(gbm_render, RENDERING|SCANOUT),`
     `exporter=GbmFramebufferExporter::new(gbm_card, Some(render_drm_node)),`
     `renderer_formats, …)`. El `import_node` (render node) hace que el exporter
     trate los buffers del render node como dmabuf foráneo y los importe al card
     node para scanout (`backend/drm/exporter/gbm.rs::add_framebuffer`).
  9. calloop + primer tick → `render_frame` (render node) → import dmabuf →
     `queue_frame` (card node, master) → primer frame.

Los type-aliases no cambian (`Compositor` = `DrmCompositor<GbmAllocator,
GbmFramebufferExporter>`, `GlesRenderer`); solo cambia **sobre qué device** vive
cada pieza. `DrmState.renderer` pasa a ser el del render node; hay que retener el
`gbm_card` para clonarlo en cada exporter.

### Detalle: el greeter es cliente aparte

Para cumplir el SDD a la letra ("primer frame **del greeter** compuesto antes
del handoff") habría que, en Fase A, levantar el socket Wayland, `spawn_greeter`,
y bombear el event loop hasta que el greeter mande su primer buffer y mirada lo
componga off-screen — todo sin master. Es factible (es render/IPC, no KMS) pero
suma complejidad. **Variante mínima recomendada:** en Fase A solo se adelanta el
`GlesRenderer::new` (el costo dominante); el greeter sigue componiéndose en Fase
C como hoy. Captura la mayor parte del beneficio con mucho menos riesgo. El
"greeter compuesto antes del handoff" completo queda como refinamiento posterior.

### Fallback

El camino nuevo es oportunista. Si abrir el render node o crear EGL/GLES sobre él
falla, se cae al orden actual (handoff → `GlesRenderer` sobre el card node). Peor
caso estructural = comportamiento de hoy.

### Verificación (pendiente, requiere GPU)

- `cargo check -p mirada-compositor` cubre lo estructural.
- El scanout cross-node por dmabuf depende de los **modifiers** del driver:
  compila bien pero un mismatch en runtime = pantalla negra, solo visible en
  hardware. Validar en una máquina con GPU real (Intel/AMD) observando el
  crossfade splash→greeter sin gap de `BG` estático, y comparando la duración del
  gap contra el baseline actual (evidencia de texto: timestamps de "splash
  RELEASED" vs primer `queue_frame`).

## Empaquetado (cómo llega al boot)

El `arje-packager` recorre `genesis` y, por cada Ente `Native`, exige el binario
del host vía `--bin <label>=<path>`. Para incluir el splash:

```bash
cargo build -p arje-splash --release
arje-packager --seed 03_ukupacha/arje/seeds/arje-qemu.card.json \
  --bin arje-splash=target/release/arje-splash \
  --bin agetty-ttyS0=/sbin/agetty \
  --out initramfs.cpio.gz
```

Queda en el initramfs como `/usr/lib/arje/arje-splash` (la ruta del `exec` de su
Card). Sin el `--bin`, el packager falla pidiéndolo (integridad seed↔binarios).
