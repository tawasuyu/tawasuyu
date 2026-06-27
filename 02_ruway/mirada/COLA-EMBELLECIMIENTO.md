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
- [x] **✅ Backdrop REAL — 2ª rebanada: barras de título flotantes por
      profundidad (calidad N).** `DrmState::window_backdrops` (id→buffer) +
      `rebuild_window_backdrops` (antes de `emit_windows`): por cada ventana
      flotante visible se re-rinde a un offscreen la escena **debajo de ESA
      ventana** (las inferiores en z, surfaces-only + wallpaper al fondo vía
      `wallpaper_frame`), se desenfoca y la barra glass la muestrea en vez de
      `wallpaper_blur`. Surfaces-only → sin realimentación («espejo infinito»).
      Una pasada por flotante (normalmente 1); cae a `wallpaper_blur` si falla.
      Opt-in, byte-idéntico en off. **Falta verificar en metal.**
- [x] **✅ Backdrop REAL — afinar (c): downsample del blur.** El backdrop
      *frosted* (menú raíz **y** barras flotantes) ya no se blurea a tamaño de
      salida: se **submuestrea** (`downsample_bgra`, caja s×s, pura+testeada) por
      un factor que acota el lado mayor a ≤720px (`backdrop_downsample`), se
      blurea el buffer reducido (radio /s, conserva la extensión visual) y se
      cachea chico. El muestreo (`src`/`dst` ratio-aware) lo re-escala a la salida
      en la GPU. Coste del blur (y peso del cache) ya **no escala** con la
      resolución (una 4K se blurea a ~720p). Sin cambiar el camino de muestreo.
      Opt-in, byte-idéntico en off. **Falta verificar en metal.**
- [x] **✅ Backdrop REAL — afinar (d): filo del cristal + tinte que SÍ se ve.**
      `push_glass_rim` empuja un filo de 1px sobre cada panel glass (menú raíz y
      barras flotantes): línea clara arriba (specular del canto) + oscura abajo
      (sombra interna) — el detalle que distingue un cristal real de un blur. De
      paso se corrigió un **orden latente**: el tinte translúcido se empujaba
      DEBAJO del blur opaco (`box_blur_bgra` fuerza alfa 255), así que nunca se
      veía; ahora el orden front-to-back es filo → tinte → blur, y el tinte
      tiñe/da contraste como dice su comentario. Opt-in, byte-idéntico en off.
      **Falta verificar en metal.**
- [x] **✅ Backdrop REAL — afinar (b): calidad del glass off/1/N en el panel.**
      Nuevo `glass_quality` (config, sección «Efectos», 0–2, default 2 = N para
      conservar lo previo). `0` = sólo wallpaper desenfocado (barato); `1` =
      backdrop REAL bajo el menú raíz; `2` = además por barra flotante (calidad
      N). El compositor gatea `rebuild_menu_backdrop` (≥1) y
      `rebuild_window_backdrops` (≥2); por debajo se cae a `wallpaper_blur`.
      Default por serde = 2 (`default_glass_quality`), así configs viejas no
      cambian. Tests de apply/clamp. **Falta verificar en metal.**
- [x] **✅ Glass = atributo del THEME, encendido por default en «mirada».** El
      glass dejó de ser un toggle global suelto: ahora es **del theme** (como
      borde/titlebar). Sección propia `glass` en el schema de mirada (separada de
      `efectos`); el panel la inyecta en la pestaña **Themes**, no en Vista, y
      `Theme` (wawa-panel) absorbe `glass_blur`/`glass_quality` en
      `from_config`/`apply_to`. `vista_mirada` nace con `glass_blur:16,
      quality:2`; el resto de las vistas y `Config::default()` crudo siguen en 0
      (doctrina opt-in intacta: el default sin theme = sin glass). Resultado:
      **apagado en todos los themes/perfiles salvo «mirada»** (el default de
      mirada), donde está activo; editable por-theme y persiste al theme. Tests:
      vista nativa lleva glass, las demás no; apply/clamp de la sección `glass`.
      **Falta verificar en metal** (el blur 16 es a ciegas — ajustable en el panel).
- [x] **✅ Backdrop REAL — afinar (a): paneles layer-shell *frosted*.**
      `over_layer_rects` (geometrías de los paneles Top/Overlay — un waybar, o la
      propia `pata`, que es un layer Top) + `build_layer_glass`: rinde la escena
      DEBAJO de los paneles (ventanas + wallpaper) a un offscreen, la desenfoca
      (downsample → blur, igual que el menú) e inserta una rebanada *frosted*
      recortada **detrás** de cada panel (en `over_z + n_over`, antes del menú).
      Un panel **translúcido** deja ver el blur (glassmorphism KDE/GNOME). Sin
      tinte ni filo: el panel cliente pone su color. Opt-in (`glass_blur>0`,
      calidad ≥1); vacío sin glass/paneles → sin splice → byte-idéntico.
      **Falta verificar en metal** (necesita panel translúcido — p. ej. que
      `pata` se rinda con alfa — para que el blur se vea).
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
