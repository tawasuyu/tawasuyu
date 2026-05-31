# Cómputo pesado fuera del hilo de UI — regla dura de Llimphi

> **PRIORIDAD URGENTE.** Patrón a aplicar a **todas** las apps Llimphi.
> Origen: el "Not Responding" de cosmos (2026-05-31). Implementación de
> referencia: `01_yachay/cosmos/cosmos-app-llimphi` (commits `added8b3`,
> `9f221983`).

## La regla

Ningún `App::update`, `App::init` ni handler (`on_key`/`on_wheel`/…) debe
ejecutar trabajo pesado **síncrono**. Bloquea el hilo de UI → la ventana no
repinta, no responde, no cierra → "Not Responding". Es el antipatrón win32 de
trabajo pesado en el message loop.

Crítico: en winit, **`App::init()` corre dentro de `resumed`, DESPUÉS de crear
la ventana**. Un cómputo pesado en init congela la ventana ya visible.

Se nota brutal en **debug** (sin optimizar, 10–50× más lento; además debug
*panica* en overflow donde release *wrappea*). Pero la mala arquitectura está
igual en release: una carta pesada, una máquina lenta o un dataset grande la
exponen.

"Pesado" = efemérides/simulación, layout de árboles grandes, IO de disco/red,
parse, embeddings, compresión… cualquier cosa que pueda pasar de ~unos ms.

## El patrón (mover a un worker)

```rust
// 1) Mensaje de resultado: u64 = generación; Arc<T> porque Msg: Clone.
enum Msg { /* … */ XComputed(u64, std::sync::Arc<Resultado>) }

// 2) En el Model: el resultado es Option (None = "calculando…"),
//    más un flag dirty y un contador de generación.
struct Model { x: Option<Resultado>, x_dirty: bool, x_gen: u64, /* … */ }

// 3) recompute_x sólo marca dirty (los helpers no tienen el Handle).
fn recompute_x(m: &mut Model) { m.x_dirty = true; }

// 4) Al FINAL de update() (que SÍ tiene el Handle): si está sucio, bumpear
//    generación, clonar los inputs y despachar a un worker.
if m.x_dirty {
    m.x_dirty = false;
    m.x_gen = m.x_gen.wrapping_add(1);
    let gen = m.x_gen;
    let input = m.input.clone();              // sólo lo que el worker necesita
    handle.spawn(move || Msg::XComputed(gen, std::sync::Arc::new(compute(&input))));
}

// 5) Arm del resultado: aplicar SÓLO si la generación sigue vigente
//    (un recálculo posterior ya dejó viejo a este). try_unwrap evita copiar
//    (el Arc llega con refcount 1 porque el Msg no se clona en el camino).
Msg::XComputed(gen, x) => {
    if gen == m.x_gen {
        m.x = Some(std::sync::Arc::try_unwrap(x).unwrap_or_else(|a| (*a).clone()));
    }
}

// 6) En init: arrancar con None y despachar el primer cómputo a un worker
//    (init tiene el Handle). La vista pinta "calculando…" mientras tanto.

// 7) En la vista: match &model.x { Some(v) => panel(v), None => calculando() }
```

Notas:
- El campo `Option<T>` exige `T: Clone` (para el fallback de `try_unwrap`).
- La **generación** evita que un resultado tardío pise a uno más nuevo
  (drags, toggles rápidos). Imprescindible si el recálculo puede dispararse
  seguido.
- Inputs al worker deben ser `Send` (clonar `Chart`, `Vec`, etc.).
- No hace falta async-ear lo barato: en cosmos el render de la carta quedó
  síncrono (con el solver acotado son ms); sólo el astro (144 muestras × 10
  cuerpos) fue a worker.

## Soluciones colaterales de la misma cacería (ya aplicadas, no revertir)

- **Preferir Vulkan en `llimphi-hal`** (`Hal::new`, commit `9f221983`): pedir
  adapter con `Backends::PRIMARY` y caer a `all()` (incluye GL) sólo si no hay
  PRIMARY. El backend **GL de Mesa sobre Wayland segfaultea en el teardown**
  (`eglTerminate → wl_proxy_marshal` sobre conexión muerta, exit 139 sin
  panic). Es infra compartida → ya beneficia a todas las apps. No volver a
  `InstanceDescriptor::default()`.
- **Acotar solvers iterativos** (`cosmos-ephemeris`, Kepler, commit `added8b3`):
  un `loop {}` con corte `dl.abs() < 1e-15` (pegado al epsilon de f64) entra en
  ciclo límite y NO converge para ciertos inputs → loop infinito. Release
  fusiona flops (FMA) y converge; debug no. **Todo solver Newton/bisección
  lleva cota dura** (`for _ in 0..N`), no `loop {}`.

## Cómo diagnosticar (sin ptrace; `ptrace_scope=1` bloquea gdb a no-hijos)

- `/proc/$PID/wchan` del hilo principal: `do_epoll_wait` = ocioso sano;
  `__futex_wait` = deadlock de lock; estado `R` sostenido = spin o cómputo en
  el hilo de UI; `dma_fence`/`drm` = GPU; `poll` sobre fd `wayland-0` = frame
  callback.
- gdb **como PADRE** sí puede (lanzar la app *bajo* gdb): backtrace del spin/
  segfault. La pila de wgpu revela el backend (`wgpu_hal::gles` vs vulkan).
- Trazar con un `eprintln` ENTER/DONE para distinguir "una llamada que no
  termina" (loop infinito) de "se llama repetidas veces" (storm de dispatch).
- En debug arranca como `cargo run` (binario `target/debug`); el release puede
  ocultar el bug (float/overflow distintos).

## Checklist — auditar y aplicar a cada app

Buscar trabajo pesado en `init`/`update`/handlers y moverlo a worker:

- [x] `01_yachay/cosmos/cosmos-app-llimphi` (referencia)
- [ ] `00_unanchay/pluma/pluma-app`
- [ ] `00_unanchay/pluma/pluma-editor-llimphi`
- [ ] `00_unanchay/pluma/pluma-notebook-llimphi`
- [ ] `00_unanchay/puriy/puriy-llimphi`  (motor JS/render — alto riesgo)
- [ ] `00_unanchay/khipu/khipu-app`
- [ ] `00_unanchay/chaka/chaka-app-llimphi`
- [ ] `01_yachay/dominium/dominium-app-llimphi`
- [ ] `01_yachay/nakui/nakui-ui-llimphi`, `nakui-sheet-llimphi`, `nakui-explorer-llimphi`
- [ ] `01_yachay/iniy/iniy-explorer-llimphi`
- [ ] `01_yachay/tinkuy/tinkuy-llimphi`  (simulación — alto riesgo)
- [ ] `02_ruway/ayni/ayni-llimphi`
- [ ] `02_ruway/chasqui/chasqui-explorer-llimphi`, `chasqui-broker-explorer-llimphi`
- [ ] `02_ruway/nada`, `02_ruway/mirada/*-llimphi`
- [ ] `pineal-*` (charting — revisar si el cómputo de series corre en update)

(Lista de partida: `grep -rl 'llimphi-ui' --include=Cargo.toml`. Los widgets/
modules/demos rara vez hacen cómputo pesado; foco en las apps de dominio.)
