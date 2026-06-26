# Cola de embellecimiento de mirada — para seguir en metal

Checklist vivo de la capa de embellecimiento (motion + efectos + glassmorphism).
Detalle y decisiones por rebanada: `PLAN.md` §«Capa de embellecimiento» y
§«Wallpaper dinámico/video» (notas `✅ HECHO`). Marcá acá lo verificado en metal.

## ✅ Hecho (en `origin/main`) — falta tu verificación en metal
- [x] **fade-in** de apertura de ventana (+ enum `Easing`, `reduce_motion`)
- [x] **pop** de apertura (escala 0.9→1)
- [x] **glow de foco** (crossfade del marco/barra)
- [x] **fade al cerrar** (motor captura-a-textura CPU)
- [x] **atenuar ventanas sin foco** (velo animado con el foco)
- [x] **wallpaper en video** (foreign-av): por-salida · loop sin costura · escalado
      GPU · pausa fullscreen/VT/DPMS · panel + clip de ejemplo
- [x] **wallpaper de marca animado por defecto** (chakana + plano cartesiano +
      fluido a las flechas + iluminación que respira) — verificado headless 1 vez

## ⏳ En curso / siguiente — shaders GLES (sólo certificable en metal)
- [x] **Esquinas redondeadas** (`corner_radius`, default 0, sección «Efectos» del
      panel). **Vía CPU** (la GPU quedó bloqueada, ver abajo): se rinde el
      contenido del cliente a un offscreen, se **lee a CPU**
      (`render_elements_offscreen`), se aplica la máscara SDF en las 4 esquinas
      (`round_mask_bgra`, pura + testeada) y se sube como `MemoryRenderBuffer`.
      Caro (lectura de GPU por ventana/frame) → opt-in. Limitación: el readback es
      `Xrgb8888` (opaco), así que una ventana **translúcida** redondeada pierde su
      translucidez interior (raro). **Falta verificar en metal.**
- [ ] **🚧 BLOQUEO para efectos por-shader GPU (rounded GPU + glass).** El enum
      `Frame<R>` es **genérico** y su `RenderElement<R>` exige que cada variante lo
      sea; `TextureShaderElement`/`TextureRenderElement<GlesTexture>` son
      **sólo `GlesRenderer`** → no entran. Además `override_default_tex_program`
      vive en `GlesFrame`, que `DrmCompositor::render_frame` crea adentro (sin
      hook). **Para hacer rounded/glass en GPU hace falta primero** una de: (a) un
      enum de elementos **específico de `GlesRenderer`** para el path DRM (el
      winit usa otro), o (b) un **pase de render manual** del layer de ventanas
      donde controlemos `GlesFrame` (y ahí `override_default_tex_program`). Es un
      refactor del `Frame`/render_output, no una rebanada más.
- [ ] **Glassmorphism** (parte C, el «wow» caro) — **depende del bloqueo de
      arriba**. Multi-pase: capturar el backdrop detrás de la superficie →
      downsample → N blur separables → upsample → componer con tinte + filo. Reusa
      `Offscreen<GlesTexture>`. Opt-in con control de calidad (off / 1 / N).
      Alternativa sin desbloquear: **blur en CPU** del readback (muy caro, sólo
      para prototipar). Pausar en apps de video/juego (idle-inhibitor).
- [ ] **`WindowEffects` ampliado por-`app_id`**: `blur`, `corner_radius`,
      `border_tint`/`border_alpha`, mover el `dim_unfocused` global a regla
      por-app (`Rules`).
- [ ] **Preset «glassmorphism»** en wawa-panel: enciende de una translucidez +
      blur + rounded + sombra suave + filo, encima del `theme`.

## Otras ideas diferidas (PLAN)
- [ ] Transiciones fullscreen: **apagado CRT** y **hero lock→thumbnail** (mismo
      motor captura-a-textura que el fade-close).
- [ ] Wallpaper: **slideshow con crossfade**, fuente **animada procedural** de
      escritorio (mover `BgAnim` del greeter a crate compartido).

## Reglas que respeta esta capa
- Todo **apagable y byte-idéntico en off**; lo caro nace **opt-in (default 0)**.
- El compositor compone en **GLES** (no Llimphi); los efectos son del path DRM.
- Certificar con **stats/tests** las piezas puras; el render se mira sólo en metal
  (regla 8 de `CLAUDE.md`).
