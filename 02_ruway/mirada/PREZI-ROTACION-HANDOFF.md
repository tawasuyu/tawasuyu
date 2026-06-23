# Handoff: contenido VIVO rotado en la vista espacial (Prezi) de mirada

> Para la sesión de Claude Code corriendo **en el metal del usuario** (la máquina
> con la GPU/Mesa real y el compositor DRM). La memoria de Claude no viaja entre
> máquinas; este doc sí (va con el repo). Leé esto primero.

## El objetivo

La vista espacial (Win+Tab → mosaico de escritorios) ya funciona y está
**hermosa**: navegación tipo switcher, zoom-out/zoom-in animados, mapa completo,
y **contenido vivo** (ventanas reales en miniatura) en los tiles **rectos**.

Falta lo último: que los tiles **rotados** (el usuario rota escritorios en el
editor del panel) muestren su **contenido vivo rotado**, no el esquema de
rectángulos. Esto "le da el stunning al compositor" (palabras del usuario).

## ⟶ ACTUALIZACIÓN 2026-06-23 (en el metal del usuario: Intel Iris Xe, Mesa 26.1.1)

**El blocker de abajo es FALSO en este metal.** Se midió headless con dos tests
standalone (quedaron en el repo como examples del crate `mirada-compositor`):

- `cargo run -p mirada-compositor --example offscreen_texture_diag` → un
  `import_memory` (textura 2D, = el badge del número) dibujado a un offscreen
  anidado: **16 buckets de color, 0% clear → SÍ dibuja**.
- `cargo run -p mirada-compositor --example offscreen_dmabuf_diag` → una textura
  **external-OES** (dmabuf, = una ventana cliente real) dibujada al offscreen:
  **16 buckets, 0% clear → SÍ dibuja**.

O sea: en esta GPU/Mesa el offscreen anidado dibuja **ambos** tipos de textura.
El diagnóstico a ciegas de la otra máquina (que decía que ni el número se
dibujaba) no es confiable / era otra máquina. **El Plan B NO hace falta acá.**

Conclusión: si el tile vivo-rotado todavía cae al esquema, es por la
**EXTRACCIÓN** de la textura de la ventana (`with_renderer_surface_state(&s,
|st| st.texture(ctx)).flatten()` devolviendo `None`, o `buffer_render_sano`
filtrando la superficie), **no** por el dibujo. Se agregó un **log one-shot** en
`render_tile_live_rotated` que imprime «N/M ventanas con textura» la primera vez
que se compone un tile con ventanas — correr el compositor en DRM, abrir Prezi
con un escritorio rotado, y leer ese log da la respuesta sin mirar píxeles:
- `M/M con textura` → la extracción anda → el tile vivo-rotado debería verse;
  si igual no se ve, mirar la captura (el problema sería la rotación/colocación).
- `0/M con textura` → la extracción falla → arreglar ahí (probablemente el
  texture() necesita el context_id correcto, o importar en el mismo paso de
  composición que el frame principal).

**RESULTADO EN EL METAL (confirmado corriendo el compositor DRM):**
`extracción: 1/1 ventanas con textura` → la extracción ANDA, el tile vivo
rotado se dibuja. Quedaban tres defectos visuales, los tres arreglados:

1. **«Queda de cabeza» (flip vertical).** Causa: `GlesMapping::flipped()` en
   smithay 0.7 está **hardcodeado a `true`** (`gles/texture.rs`), pensado para
   el framebuffer de una `EGLSurface` (glReadPixels bottom-up). Pero el target
   es un **offscreen `GlesTexture`**, donde el readback YA viene top-down. La
   corrección por `flipped()` en `render_offscreen_drawing` lo daba vuelta.
   Medido headless (`examples/offscreen_orient_diag`): crudo = IDENTIDAD, tras
   el swap = flip vertical. Fix: **no aplicar el swap** en
   `render_offscreen_drawing` (es universal, no depende de la GPU).

2. **Zoom-in gris.** Causa: el cap `LIVE_ROT_MAX` devolvía `None` cuando el tile
   estaba grande (durante el zoom) → esquema gris. Fix: componer SIEMPRE a una
   **resolución acotada** (≤560) y que el llamante **escale el bitmap por GPU**
   (`RescaleRenderElement`, nueva variante `Frame::ScaledText`) hasta el tamaño
   real. El giro vivo se ve durante todo el zoom; el costo CPU queda acotado.

3. **Parpadeo.** Causa: el heurístico de «variedad de color» (buckets) devolvía
   `None` por frame cuando la composición tenía poca variedad → alternaba
   vivo/esquema. Como ya está PROBADO que el offscreen dibuja texturas, se
   **eliminó** ese heurístico (y el latch). `render_offscreen_drawing` ya
   devuelve `None` sólo ante fallo REAL de GPU; las ventanas sin buffer sano ya
   caen a un rect sólido dentro de la composición.

4. **Rotación atada a la curva del zoom.** El vuelo de cámara interpolaba
   posición/tamaño/escala por `t_open` pero NO el ángulo: el tile aparecía
   rotado de golpe al abrir y se quedaba en diagonal al cerrar. Fix: `tl.rot *=
   t_open` en el loop de cámara — a `t_open=0` (activo a pantalla completa) el
   tile está derecho, a `t_open=1` (mosaico) toma su ángulo pleno; al cerrar
   des-rota de 1→0. Cuando el ángulo interpolado es ~0 cae al camino recto de
   quads (full-res), así que sólo mid-vuelo usa el bitmap rotado de baja-res.

**PRÓXIMO PASO EN EL METAL:** correr el compositor DRM con el build nuevo,
abrir Prezi con un escritorio rotado y confirmar: derecho (no de cabeza),
vivo durante el zoom (no gris), sin parpadeo, y el giro entrando/saliendo
con la curva del zoom (no de golpe).

---

## El blocker (diagnosticado a ciegas — ver actualización de arriba: FALSO en este metal)

Para rotar contenido vivo, la estrategia fue: componer el tile (fondo + ventanas
+ número) en una **textura offscreen** axis-aligned, leerla de vuelta, **rotarla
en CPU** (`text::rotate_buffer`) y dibujarla. La rotación en CPU y el readback
funcionan. **El problema:** el render offscreen anidado, en la Mesa del usuario,
**pinta colores sólidos pero NO texturas** — confirmado por diag: con 1 ventana
presente, el readback tenía **1 solo color** (el fondo), y el **número (una
imagen simple) tampoco se dibujó**. O sea no es tema de superficies de cliente:
ninguna textura se dibuja en ese offscreen.

Intentos ya hechos (todos a ciegas vía logs del usuario):
- `import_surface_tree` antes del offscreen → no cambió nada.
- Último commit `05573e4d`: **extraer la `GlesTexture` a mano**
  (`with_renderer_surface_state` + `context_id`) y dibujarla con
  `render_texture_from_to` directo (saltando la búsqueda por context_id del
  render-element, que era la hipótesis de por qué daba vacío). **Sin confirmar
  en metal todavía.**

## PRIMER MOVIMIENTO EN EL METAL (antes de tocar nada)

Escribir un **test GLES standalone** (headless, ~50-80 líneas) que:
1. Cree un `GlesRenderer` (EGL headless / gbm).
2. Importe una textura conocida (un buffer RGBA de memoria con un patrón de
   varios colores) vía `import_memory`.
3. Haga `Offscreen::<GlesTexture>::create_buffer` + `bind` + `render` + `clear` +
   `frame.render_texture_from_to(esa_textura, ...)` + `finish`.
4. `copy_framebuffer` + `map_texture` y **cuente los colores** del readback.

Resultado:
- **El readback tiene los colores de la textura** → el offscreen SÍ dibuja
  texturas. Entonces el bug está en cómo extraigo/paso la textura de la ventana
  (commit `05573e4d`); se depura ahí, rápido, con capturas reales.
- **El readback es monocromo (solo el clear)** → la Mesa NO dibuja texturas a un
  offscreen anidado. Entonces **abandonar el offscreen** e ir por el Plan B.

Esto mata la incógnita central en minutos en vez de adivinar por commits.

## Plan B (si el offscreen no dibuja texturas)

Dibujar la textura de la ventana **rotada, directo en el frame principal** (donde
las texturas SÍ se dibujan — los tiles rectos lo prueban), con
`GlesFrame::render_texture(tex, tex_matrix, matrix, ...)` — método **público**
que acepta una `Matrix3<f32>` con **rotación arbitraria**. NO hace falta forkear
smithay.

Obstáculo a resolver: el enum `Frame` (en `drm_backend/mod.rs`, vía la macro
`render_elements!`) es **genérico** (`Frame<R>`), y un elemento que llame
`render_texture` es específico de `GlesRenderer`. Hay que:
- Reemplazar el `Frame` de la macro por un **enum concreto a mano** (el compositor
  sólo usa `Frame<GlesRenderer>`), con su `impl RenderElement<GlesRenderer>`, y
  agregar una variante `RotatedTexture` que en su `draw` llame `render_texture`
  con la matriz de giro.
- La matriz: copiar la convención de `render_texture_from_to`
  (smithay-0.7 `gles/mod.rs:2488`) y agregarle `Matrix3::from_angle_z(rot)`
  alrededor del centro del tile. `render_texture` está en `gles/mod.rs:2628`.
- Sacar la `GlesTexture` de cada ventana: `with_renderer_surface_state` +
  `renderer.context_id()` (ya está hecho en `render_tile_live_rotated`).

Verificar con capturas reales (screencopy de mirada o un screenshot).

## Dónde está el código

- `02_ruway/mirada/mirada-compositor/src/drm_backend/render.rs`:
  - `render_tile_live_rotated()` — compone el tile rotado (la pieza a arreglar).
  - `emit_overview()` — la vista espacial; el camino rotado llama a la anterior y
    cae al esquema (`text::rasterize_tile_rotated`) si devuelve `None`.
  - Hay un **latch** (`AtomicU8 OFFSCREEN`) que marca el offscreen como roto si un
    tile con ventanas sale monocromo, para no reintentar por frame.
  - Cap de tamaño `LIVE_ROT_MAX=560`: el render-vivo-rotado sólo corre con el tile
    chico (mosaico asentado), no durante el zoom (sería O(área) por frame).
- `02_ruway/mirada/mirada-compositor/src/screencopy.rs`:
  - `render_offscreen_drawing()` — offscreen por-closure (bind/render/readback).
  - `render_elements_offscreen()` — versión por-elementos (la vieja).
- `02_ruway/mirada/mirada-compositor/src/text.rs`:
  - `rotate_buffer()` (pública) — rota un buffer RGBA en CPU (anda).
  - `rasterize_tile_rotated()` — el esquema rotado (fallback, anda).

## Cómo probar / verificar en el metal

- Build: `cargo build -p mirada-compositor` (o `--release`, como lo lance el user).
- Activar Prezi: en wawa-panel elegir el conjunto de animación **«prezi»** (setea
  `workspace_switch_mode=Prezi` en `~/.config/mirada/config.ron`).
- Abrir: Win+Tab (Super+Tab) y mantener hasta que el mosaico se asiente.
- Rotar un escritorio: editor del panel («Vista espacial» → arrastrar/rotar).
- Capturar la salida real (screencopy / screenshot) y mirarla — ya en el metal se
  puede, no hace falta describir.

## Estado del resto del Prezi (todo HECHO y andando)

mover/rotar en el editor (fix de coords local→absoluta), mapa completo reducido
(`RescaleRenderElement` para que las miniaturas escalen), Win+Tab switcher con
cierre robusto (sondeo de Super por tick), zoom-out/in, supresión del slide en
modo Prezi. No tocar eso.
