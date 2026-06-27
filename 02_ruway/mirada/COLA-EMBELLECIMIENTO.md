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
      contenido del cliente, se enmascara con el **shader SDF en GPU**
      (`Frame::Rounded`/`TextureShaderElement`) y se dibuja — sin readback. Si el
      shader no compila, **fallback CPU** (`render_elements_offscreen` +
      `round_mask_bgra`, pura + testeada). Opt-in (cada ventana redondeada se
      rinde a un offscreen). Limitación del fallback CPU: el readback es
      `Xrgb8888` (opaco). **Falta verificar en metal.**
- [x] **✅ DESBLOQUEADO — `Frame` concreto a `GlesRenderer`.** Era genérico
      (`Frame<R>`) y no admitía variantes sólo-GLES. Ahora es
      `render_elements!{ Frame<=GlesRenderer>; … }` (el path DRM sólo usa ese; el
      winit no usa este enum). Habilita `Frame::Rounded` (`TextureShaderElement`)
      y, a futuro, cualquier elemento por-shader GPU **incluido el glass**.
- [x] **✅ Glassmorphism — 1ª rebanada: menú raíz *frosted* (`glass_blur`).** El
      fondo (wallpaper) se pasa por un **blur de caja** (`box_blur_bgra`, pura +
      testeada) **una vez** al rearmarse, cacheado en `OutputCtx.wallpaper_blur`;
      el menú raíz dibuja esa rebanada desenfocada (con `src` recortado, escalando
      si el blur está acotado) + un tinte translúcido. Opt-in (`glass_blur`,
      sección «Efectos»). **Limitación:** el backdrop barato es **sólo el
      wallpaper** (no captura ventanas detrás), así que el glass es correcto en
      elementos **sobre el wallpaper** (el menú lo está). El video no lleva glass.
      Falta verificar en metal.
- [x] **✅ Glass — barra de título *frosted* en ventanas flotantes.** Reusa el
      mismo `OutputCtx.wallpaper_blur`: la barra de una ventana **flotante** deja
      ver el wallpaper desenfocado (rebanada `src` bajo la barra) + un tinte del
      color base (con el glow de foco). Sólo flotantes (sobre el escritorio, donde
      el wallpaper ES el backdrop correcto); las teseladas siguen sólidas. Mismo
      límite: muestra el wallpaper, no ventanas detrás.
- [x] **✅ Backdrop REAL — 1ª rebanada: el menú raíz ve las ventanas detrás.**
      `OutputCtx.backdrop_blur` + `DrmState::rebuild_menu_backdrop`: la escena de
      **debajo** del menú (`out[menu_z..]` en `render_output` — wallpaper + layers
      + ventanas, ya en coords de salida) se re-rinde a un offscreen
      (`render_elements_offscreen`), se pasa por `box_blur_bgra` y el menú la
      muestrea en vez de `wallpaper_blur`. Reusa el propio element-list del frame
      (cero replicación de posiciones). Opt-in (`glass_blur>0`), sólo mientras el
      menú está abierto; si el render falla cae a `wallpaper_blur` → sólido.
      Byte-idéntico en off (el menú vuelve a su z exacto vía `splice`). **Coste:**
      una pasada offscreen + readback + blur por frame con el menú abierto.
      **Falta verificar en metal** (readback GPU de escena con ventanas).
- [ ] **Backdrop REAL — 2ª rebanada: barras de título flotantes + paneles, por
      profundidad** (calidad N). Lo del menú es calidad «1» (un backdrop único de
      toda la escena, correcto porque el menú está arriba de todo). Para una barra
      flotante el backdrop correcto es lo que hay **debajo de ESA ventana**, no la
      escena entera → armar el backdrop por-superficie (componer capas `0..N`),
      multi-pase GPU (downsample → N blur separables → upsample → tinte + filo),
      opt-in con calidad (off / 1 / N). Hoy las barras flotantes siguen con
      `wallpaper_blur` (sólo wallpaper).
- [ ] **`WindowEffects` ampliado por-`app_id`**: `blur`, `corner_radius`,
      `border_tint`/`border_alpha`, mover el `dim_unfocused` global a regla
      por-app (`Rules`).
- [ ] **Preset «glassmorphism»** en wawa-panel: enciende de una translucidez +
      blur + rounded + sombra suave + filo, encima del `theme`.

## Multisesión (FUS) — verificar en metal
- [ ] **Miniaturas de sesiones en el lock** (`origin/main`, crate
      `mirada-compositor/src/thumbs.rs` + tira en `mirada-greeter`). La lógica
      de selección per-sesión quedó **verificada por lectura** (filtra por
      `w.visible`, no por `session_visible`, así toma las sesiones de fondo); lo
      que **falta certificar en metal** es el **readback GPU de una escena
      multisesión real**. Receta:
      1. Arrancar mirada en metal, abrir sesión A con alguna ventana con
         contenido distintivo.
      2. FUS «cambiar usuario» → sesión B con otra ventana distinta.
      3. Enganchar el candado y mirar la tira: **cada** sesión (la activa **y**
         la de fondo) debe mostrar **su preview real**, no la tarjeta genérica
         (el «monitor» tenue). La activa va resaltada/más grande.
      4. Clic en la tarjeta de la otra sesión → salta a ella (`SwitchTo`).
      5. Probar privacidad: `MIRADA_LOCK_PREVIEW=hidden` → todas genéricas.

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
