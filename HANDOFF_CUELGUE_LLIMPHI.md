# Handoff — cuelgue de apps Llimphi (deadlock tras click/scroll)

> Documento de traspaso para continuar la investigación en **la laptop** (donde
> sí hay display gráfico). En la máquina anterior era un TTY headless y no se
> pudo reproducir. Pegá este archivo (o pedile a Claude que lo lea) al arrancar.

## El problema

Casi **todas** las apps GUI de gioser (Llimphi) **se cuelgan**. NO es la laptop:
pasa en más de una máquina.

Síntoma confirmado por el usuario:
- Afecta a **casi todas** las apps Llimphi → es **infra compartida**, no una app puntual.
- Se congela **tras una acción**: gestos reportados = **click en botón/lista** y **scroll/rueda**.
- Al colgarse, la **CPU está baja** (ventilador tranquilo) → es un **deadlock / bloqueo
  esperando algo**, NO un bucle infinito (spin).
- Empezó **~2026-05-30 por la mañana/mediodía**, *antes* de los menús.

## Ya descartado (con evidencia)

- **No es bump de dependencias.** `Cargo.lock` sin cambios desde 2026-05-28
  (wgpu 24.0.5, winit 0.30.13, vello 0.5.1, parley 0.4.0, naga 24.0.0).
- **No es el renderer.** `02_ruway/llimphi/llimphi-raster` sin cambios de código desde 2026-05-29.
- **No es el clipboard ni los menús.** `llimphi-clipboard` (`a0c5dfd5`) y los lotes de
  menús (`bd70a520` + lotes 1-6) aterrizaron **HOY 2026-05-31 a las 07:03+**, o sea
  *después* de que el cuelgue ya ocurría → no pueden ser la causa raíz.
- **No es el split de `lib.rs`** (`35a6579f`, 2026-05-31 01:11) — también posterior.

## Hipótesis principal

El bloqueo está en el camino de redraw compartido:

```
input (click/scroll) → A::update → window.request_redraw()
  → WindowEvent::RedrawRequested   (02_ruway/llimphi/llimphi-ui/src/eventloop.rs:476)
    → surface.acquire()            (= surface.get_current_texture(), llimphi-hal/src/lib.rs:285)
    → render
    → surface.present()            (llimphi-hal/src/lib.rs:306)
```

`present_mode = AutoVsync` (`02_ruway/llimphi/llimphi-hal/src/lib.rs:215`). En Wayland
con FIFO, `get_current_texture()` **bloquea con CPU baja** hasta que el compositor
suelta un buffer (frame callback). Si el compositor —¿`mirada-compositor`?— deja de
enviar frame callbacks, **todas** las apps que corran bajo él se cuelgan exactamente ahí.
Esto encaja con: casi-todas-las-apps + tras-una-acción + CPU-baja.

**Candidato de runtime mejor fechado:** `d3957c17` "feat(llimphi-ui): App::on_file_drop"
(2026-05-30 00:21, modifica `eventloop.rs`) — justo antes de la mañana del 05-30.
Otro candidato más viejo: `15eb952b` (media, "controles en tiles draggables",
2026-05-29 14:42, añadió `DragState` al runtime).

## Próximos pasos (EN LA LAPTOP, con display)

1. **Reproducir.** Correr una app simple y colgarla con click/scroll:
   ```bash
   cargo run -p nada --release      # editor con file-tree, mínimo
   # o el ejemplo más chico del runtime:
   cargo run -p llimphi-ui --example counter --release
   ```
   Anotar bajo qué compositor corre (Wayland propio `mirada-compositor` vs sway/GNOME/etc.):
   ```bash
   echo "$XDG_SESSION_TYPE $WAYLAND_DISPLAY $DISPLAY"
   ```

2. **Backtrace del proceso colgado.** Con la app congelada, en otra terminal:
   ```bash
   PID=$(pgrep -f 'target/release/nada' | head -1)
   gdb -p "$PID" -batch -ex 'thread apply all bt' 2>/dev/null
   # alternativa: rust-lldb -p "$PID" --batch -o 'thread backtrace all'
   ```
   - Si el hilo principal está parado en `get_current_texture` / `present` / `vkAcquireNextImage` /
     un frame-callback de Wayland → **confirma la hipótesis del compositor/FIFO**.
   - Si está en un `Mutex`/`Condvar`/`parking_lot` → es otra cosa; seguir esa pila.

3. **Según el backtrace:**
   - Si es `get_current_texture` bloqueante: probar `present_mode` a `Mailbox` o `Immediate`
     en `llimphi-hal/src/lib.rs:215` y ver si desaparece el cuelgue (diagnóstico, no fix final).
     Luego investigar los frame callbacks de `mirada-compositor`.
   - Si no aclara: `git bisect` entre 2026-05-29 y 2026-05-30 mañana, probando una app simple
     en cada paso (buen-conocido: el commit anterior a `d3957c17`).

## Bug colateral REAL a arreglar (independiente de este cuelgue)

`std::mem::replace(&mut m.clipboard, SystemClipboard::new())` crea un `arboard::Clipboard`
**nuevo por cada acción** de portapapeles → en X11 cada `new()` arranca un hilo servidor de
selección y el `drop` del viejo hace un handoff que **bloquea**. Sitios:
- `01_yachay/nakui/nakui-ui-llimphi/src/main.rs:895`
- `02_ruway/mirada/mirada-greeter/src/main.rs:480`

Fix: usar `&mut m.clipboard` directo (reordenar borrows) o swap con un `Option`/`NullClipboard`;
nunca construir `SystemClipboard::new()` por acción.

## Archivos clave

- Bucle de eventos / redraw: `02_ruway/llimphi/llimphi-ui/src/eventloop.rs`
- Runtime / Handle / dispatch: `02_ruway/llimphi/llimphi-ui/src/lib.rs`
- Surface (acquire/present/present_mode): `02_ruway/llimphi/llimphi-hal/src/lib.rs`
