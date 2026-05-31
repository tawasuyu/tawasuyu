# Handoff — cuelgue de apps Llimphi (deadlock tras click/scroll)

> **✅ RESUELTO (2026-05-31).** NO era un deadlock ni infra Llimphi ni Wayland.
> Era un **loop infinito en el solver de Kepler** de `cosmos-ephemeris`
> (`src/planets/mod.rs`, `elements_to_cartesian`): un `loop {}` sin cota cuyo
> corte `dl.abs() < 1e-15` está pegado al epsilon de f64. Para ciertos inputs
> (Venus en el instante de una carta de Lima) la iteración entra en ciclo límite
> y nunca converge → el hilo de UI queda atascado en `compute_astro` → "Not
> Responding", no cierra. Sólo en **debug**: release fusiona/reordena los flops
> (FMA) y converge, por eso `cargo run` colgaba y el binario `--release` no.
> **Fix:** acotar la iteración (`for _ in 0..50`). compute_astro pasa de no
> terminar nunca a completar (~9 s en debug, instantáneo en release) y la app
> queda idle estable. Tests: cosmos-ephemeris 115/115, cosmos-rise-set 13/13.
> Lo de abajo es el rastro de la investigación (hipótesis descartadas).

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

## Hallazgos en la laptop (2026-05-31, sesión de diagnóstico)

Reproducción en **esta** laptop: `Wayland` nativo, compositor = **`kwin_wayland`**
(KDE Plasma), NO `mirada-compositor`. `ptrace_scope=1` ⇒ gdb/strace no pueden
attachar a procesos no-hijos; se diagnosticó vía `/proc/$pid/{wchan,syscall}`.

- **`counter` bajo XWayland + clicks reales (xdotool): NO se cuelga.** Incrementa
  bien; sólo termina al cerrar la ventana. Hilo principal ocioso en `do_epoll_wait`.
- **Sonda `spawn_periodic` (~10 Hz) en Wayland nativo: NO se cuelga.** Corrió
  **1593 updates / 1629 redraws** seguidos; cada `tick → update → request_redraw →
  acquire/present` completa y el hilo principal vuelve a `do_epoll_wait`.
  ⇒ **La hipótesis del present FIFO bloqueante queda DESCARTADA en este setup.**
  El mecanismo de entrega de `request_redraw` y el `present` funcionan en Wayland.
- Como `request_redraw()` es idéntico se dispare desde `user_event` (probado) o
  desde `window_event` (click/scroll), el camino de redraw del runtime en HEAD
  **no cuelga genéricamente**. Los splits del 05-31 (`ce830aaf` compositor/runtime,
  `a1c5d8c7`, `82480a0f`, `35a6579f`) son posteriores y pueden haber tocado el
  camino donde vivía el bug original.
- **Gap no cubierto:** no se pudo inyectar un click REAL en Wayland nativo
  (`/dev/uinput` es root-only, sin `ydotool`/`wtype`; KWin no acepta xdotool).
  Para cerrarlo: reproducir el cuelgue a mano y correr `scripts/diag-cuelgue.sh
  <app>` — imprime el `wchan` congelado, que delata la causa (futex=deadlock de
  locks, dma_fence/drm=GPU, poll sobre fd wl=frame callback).

### Sospechoso concreto reordenado: clipboard por-acción (no es "casi todas")

El antipatrón `arboard::Clipboard::new()` **por acción** (cada `new()` levanta un
hilo servidor de selección y el `drop` hace un handoff que BLOQUEA con CPU baja)
existe sólo en estos sitios — explica cuelgues al copiar/pegar en ESAS apps, no en
"casi todas":
- `02_ruway/mirada/mirada-greeter/src/main.rs:480`  (`mem::replace(... new())`)
- `01_yachay/nakui/nakui-ui-llimphi/src/main.rs:895` (`mem::replace(... new())`)
- `01_yachay/nakui/nakui-sheet-llimphi/src/main.rs:514,534` + `logic.rs:120`
- `02_ruway/shuma/sandbox/shuma-module-shell/src/update.rs:464,470`

El resto de los `SystemClipboard::new()` son de **init** (una vez, en la
construcción del Model) → inofensivos. El widget `text-editor` usa `MemClipboard`
(en memoria) → **no** es el culpable universal que haría caer "casi todas".

## Hipótesis principal (ORIGINAL — ver Hallazgos arriba: parcialmente refutada)

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
