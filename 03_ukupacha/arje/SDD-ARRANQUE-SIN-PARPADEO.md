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

No reproducible en el sandbox (sin UEFI/DRM). Se certifica en QEMU+OVMF (el
maker lo corre): lógica de render del splash y del cmdline con tests unitarios;
lo visual (cero parpadeo, crossfade) por captura/observación en QEMU.

## Estado de implementación

- [x] SDD (este documento)
- [x] Fase 0 — cmdline flicker-free (`arje-installer`)
- [x] Fase 0 — logo GOP en `arje-loader` (`gop::paint_boot_splash`, marca central placeholder; falta verificar en QEMU+OVMF)
- [ ] Fase 1 — crate `arje-splash` (DRM dumb buffer + animación)
- [ ] Fase 1 — Ente génesis de `arje-splash` en el seed
- [ ] Fase 2 — contrato de handoff splash↔mirada + crossfade
