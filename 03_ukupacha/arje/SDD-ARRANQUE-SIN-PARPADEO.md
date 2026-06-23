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

### Dos gotchas de integración (encontrados al verificar)

1. **Binarios estáticos.** El initramfs no trae `libc.so`; arje-zero/splash deben
   compilarse para `x86_64-unknown-linux-musl` (o `+crt-static`). Con binarios
   glibc-dinámicos el kernel panica con *«No working init found»*.
2. **`requires` es un gate duro.** arje-zero rechaza encarnar un Ente con un
   `requires` sin provider registrado. Declarar `requires Device{Drm}` bloqueaba
   el splash (*«requires no satisfecho»*). El acceso a DRM es físico al device,
   no una capacidad brokeada — el splash va con `requires` vacío, igual que el
   display-manager de mirada.

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
  (`esperar_release_del_splash`, llamado al inicio de `drm_backend::run`).
  Degradación elegante en ambos lados: sin socket / sin respuesta, cada uno
  sigue solo. **Verificado end-to-end en QEMU** (ver abajo); falta la
  observación visual del crossfade con el greeter real en hardware.

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
