# SDD — `pata`, el marco del escritorio

> Estado: **Fase 8b** (layer-shell sobre wlroots: barras, Quake, clicks,
> `window_list`). Este documento es la fuente autoritativa de qué es `pata` y
> dónde termina, por encima de README.

## 0. El problema que resuelve

El escritorio de gioser tenía el concepto de "launcher" **triplicado y mal
delimitado**: `mirada-launcher-llimphi` (la barra), `shuma-shell-llimphi` (un
chasis con tabs) y `shuma-module-launcher` (un módulo lista-de-apps) competían
por el mismo rol sin una frontera clara. Correr cualquiera bajo el compositor
daba ventanas sueltas en vez de un escritorio coherente.

`pata` fija la frontera: es **una sola capa**, la del *marco* (chrome) del
escritorio, desacoplada del compositor y del shell, y portable entre Linux y
wawa.

## 1. Las tres capas (no se solapan)

| Capa | Quechua/es | Qué es | Qué **no** es |
|---|---|---|---|
| **mirada** | mirar | El **compositor**: el Cuerpo Wayland/DRM. Tesela, acopla franjas, decora, enruta input. | No dibuja barras ni widgets. |
| **shuma** | — | El **shell**: input + terminal + módulos. Se asoma como una barra inferior auto-escondible; al escribir, despliega el resto estilo Quake. | No es el chrome del escritorio; es un inquilino del marco. |
| **pata** | borde, repisa, andén | El **marco**: barras, paneles y dock declarados desde un archivo de config, con widgets colocables en cualquier slot. Hospeda el input de shuma. | No compone ventanas (eso es mirada) ni ejecuta comandos (eso es shuma). |

Regla mnemónica: **mirada** pone las ventanas, **pata** pone el marco
alrededor, **shuma** es la boca por la que le hablás al sistema.

## 2. Forma del dominio

```
pata-core      Modelo agnóstico (no_std + alloc): el esquema declarativo
               (Config → [Surface] → slots → [WidgetSpec]) + el layout
               (resolve: config+pantalla → superficies colocadas + work_area).
               No pinta, no toca el SO. Cruza a wawa por `path`, como mirada-layout.
pata-config    Loader Linux (std): lee el TOML del usuario desde las rutas XDG y
               lo materializa en el modelo. El límite std→no_std del marco. Trae
               el binario `pata` para inspeccionar la geometría resuelta. En wawa
               este rol lo cumple akasha, no este crate.
pata-llimphi   Frontend Linux: monta pata-core sobre Llimphi. Cada Surface es
               una ventana Wayland que el compositor mirada acopla; despacha
               los widgets builtin; el shuma_input despliega shuma. (Fase 3,
               hereda de mirada-launcher-llimphi.)
pata (wawa)    El kernel launcher de wawa consume el MISMO pata-core y pinta
               sobre el framebuffer. (Fase 7.)
```

UIs intercambiables sobre un `*-core` agnóstico — la regla dura del repo. El
modelo se escribe una vez; Linux y wawa son dos pinceles.

## 3. El modelo (`pata-core`)

- **`Config`** = `general` + `Vec<Surface>`. Múltiples superficies, no un único
  panel. El usuario despliega tantas barras/paneles/docks como quiera.
- **`Surface`** = `kind` (Bar | Panel | Dock) + `anchor` (Top/Bottom/Left/Right)
  + `thickness` + `autohide` + tres slots `start`/`center`/`end` (+ `cards`
  para paneles). Cada slot es una lista ordenada de widgets.
- **`WidgetSpec`** = `kind: String` (conjunto **abierto**) + `props` arbitrarias.
  El frontend despacha por string y cae a un placeholder si no conoce el kind:
  agregar un widget no toca el core.
- **`Prop`** = `Bool | Int | Num | Str`. Valor de propiedad **agnóstico del
  formato en disco** — ni `toml::Value` ni nada atado a una plataforma. El
  loader de cada SO (TOML en Linux, akasha en wawa) deserializa a esto.

El formato en disco no es parte del modelo: en Linux un loader TOML deserializa
directo a estos tipos vía `serde` (contrato fijado en `tests/toml_contract.rs`);
en wawa el config llega por akasha.

## 4. Widgets builtin previstos

`start_button` · `window_list` (ventanas abiertas, vía mirada-ctl/-link) ·
`clipboard` · `volume` · `brightness` · `tray` · `clock` · medidores
(`ram_meter`/`cpu_meter`) · **`astro`** (posición zodiacal del sol + ciclo
lunar, reusando `cosmos-ephemeris`) · `shuma_input` (el cabezal del shell).

Cada uno se coloca libremente: superficie + slot se eligen desde el config.

## 5. Integración con shuma (el Quake)

El `shuma_input` es un widget que vive típicamente en una `Surface { kind: Bar,
anchor: Bottom, autohide: true }`. Muestra el cabezal del shell; al recibir
foco/escritura, el frontend **anima el despliegue** del resto de shuma sobre el
escritorio (drawer estilo Quake) y lo repliega al soltar. El marco provee el
borde; shuma provee el contenido.

## 6. Estado y plan por fases

- **Fase 1 ✅** — `pata-core` config: esquema declarativo, `Prop` agnóstico,
  contrato TOML, `no_std` verificado (wasm32). En el workspace.
- **Fase 2 ✅** — `pata-core::layout`: resuelve config+pantalla en superficies
  colocadas + `work_area` (lo que mirada tesela). Geometría pura testeada.
- **Fase 3 ✅** — `pata-config`: loader TOML/XDG → modelo + binario `pata`
  inspector. Pipeline config→layout verificado sobre archivos reales.
- **Fase 4 ✅** — modelo de widget agnóstico (`pata-core::widget`): trait
  [`Widget`] (`tick(&WidgetCtx)` / `view() → WidgetView`), un `WidgetCtx` que el
  host muestrea (reloj, cpu, ram, volumen, brillo) y un view-model
  `Text | Meter | Placeholder | Empty` sin pincel. Builtins con lógica portada:
  `clock` (strftime reducido), `cpu_meter` / `ram_meter` / `volume` /
  `brightness` (medidor genérico). `build(spec)` despacha por string y cae a
  `Placeholder` para kinds no implementados. `no_std` verificado (wasm32); el
  inspector `pata --widgets` lo muestra de punta a punta.
- **Fase 5 ✅** — `pata-llimphi`: el frontend Linux. `sampler` muestrea el
  sistema (chrono + `/proc/stat` + `/proc/meminfo` + `/sys/class/backlight`) en
  un `WidgetCtx`; `render` traduce cada `WidgetView` a `View<Msg>` (texto,
  medidor con barra, placeholder tenue) y coloca las superficies en los rects
  que el layout resolvió (posición absoluta). `PataApp` (app-id `gioser.pata`)
  carga config vía `pata-config`, `tick`ea a 1 Hz y pinta. Por ahora una sola
  ventana; mirada acopla por superficie en la Fase 8.
- **Fase 6 (parcial)** — widgets nuevos:
  - `astro` ✅ — posición zodiacal del Sol (signo + grado) + fase lunar. La
    efeméride la computa el sampler (host) y la entrega en `WidgetCtx`
    (`sun_longitude_deg` + `moon_phase`); el widget de core sólo mapea grados→
    signo y fracción→fase, con aritmética entera (no_std). El sampler usa la
    fórmula de baja precisión del *Astronomical Almanac*; `cosmos-ephemeris`
    es el upgrade drop-in cuando se quiera alta precisión.
  - `start_button` ✅ — muestra su `label` (default `⊞`). Cablear su acción
    (abrir el lanzador) espera al ruteo de clicks (Fase 7).
  - `window_list` ✅ (en layer-shell) — lista de ventanas abiertas vía el
    protocolo `wlr-foreign-toplevel-management` (el que usan waybar/eww), no por
    IPC de mirada. Ver el detalle en la Fase 8b. Bajo el compositor `mirada` (el
    path winit) sigue vacío hasta que mirada exponga sus toplevels.
  - `tray` ⏳ — StatusNotifierItem; diferido. Placeholder por ahora.
- **Fase 7 ✅** — despliegue Quake de shuma desde `shuma_input`. El frontend
  intercepta el kind `shuma_input` (es interacción, no pasa por el `build`
  agnóstico de core, igual que mirada con su shuma_bar): un cabezal clicable en
  la barra + hotkey (`keys`) despliegan un **drawer** animado (`llimphi-motion`,
  scrim que cierra al click + panel inferior que crece con el tween) que captura
  el teclado. El estado vive en `Model::shuma`, no en core. La ejecución del
  comando es, estrictamente, de `shuma`: mientras no haya puente, `shuma::
  ejecutar_stand_in` corre por `sh -c` como **sustituto temporal** (patrón de
  mirada) — se reemplaza sin tocar el mecanismo del drawer.
  - **Puente real + cards (✅)** — el drawer corre por `shuma-exec` (no `sh -c`
    pelado): historial de *cards* (`$ cmd` + etapas + salida + código), plegables,
    con scroll. **Captura por etapa (tee, paridad con el shell de shuma):** un
    pipe «simple» (sólo comandos/args/flags y `|`, sin comillas/variables/
    redirecciones/globs/`~`) corre por `Exec::Direct` con `capture_stages`; cada
    etapa **intermedia** emite su stdout en vivo (`StageStdout`) y se guarda en
    `DrawerBlock::stage_lines`. Clickear la chip de una etapa intermedia **revela
    su salida capturada inline** (sin re-ejecutar); la última etapa no se captura
    aparte (su stdout es el cuerpo de la card). Cualquier otra sintaxis cae a
    `sh -c` (sin tee). Detección en `shuma::simple_pipe_stages` (espeja
    `shuma-module-shell`), testeada.
  - **Submit a IA (✅, paridad con el quake de mirada-launcher)** — el buffer sin
    prefijo va al **LLM** (`pluma-llm::from_env`, cae a Mock sin credenciales); el
    prefijo `!`/`$` lo fuerza a shell. `shuma::classify` decide (`Empty`/`Shell`/
    `Ia`, testeada); las consultas IA abren una card `✦ <prompt>` sin chips de
    etapa que muestra `…pensando` y luego la respuesta. El resultado llega por el
    mismo `ShumaResult`/`finish_last` que un comando. Es el último gap que tenía
    `mirada-launcher-llimphi` sobre pata de cara a la Fase 10.
- **Fase 8 ✅** — `mirada-compositor` reconoce el marco `pata`:
  - Identidad: el viejo `SHELL_APP_ID = "carmen.shell"` → `is_shell_app_id`, que
    matchea `gioser.pata` (la identidad que anuncia `pata-llimphi`) o el alias
    legacy `carmen.shell`, override por `MIRADA_SHELL_APP_ID`.
  - Anclaje/grosor configurables (`MIRADA_SHELL_ANCHOR` / `MIRADA_SHELL_THICKNESS`,
    defaults bottom/40), ya no una franja fija de 40px al pie. Geometría en
    helpers puros testeados (`shell_strip` / `shell_insets`).
  - **Zonas exclusivas en los cuatro bordes**: el acople ya no encoge la salida,
    sino que reserva *insets* (top/bottom/left/right) vía el evento nuevo
    `BodyEvent::OutputReserved` → `Body::reserve_output` → el Cerebro guarda
    `Output::reserved` y tesela sobre `Output::work_rect()` (rect menos insets).
    El motor de layout ya respetaba `screen.x/y`, así que top/left desplazan el
    origen del teselado correctamente. Soporta barras en varios bordes a la vez.
    Cerrar el shell libera la reserva (insets en cero).
- **Fase 8b (en curso)** — `pata` como **layer surface** (nivel eww/waybar) en
  compositores wlroots (Hyprland, Sway, river), no como ventana cliente. Sin
  esto, en Hyprland pata abría como ventana flotante; el acople de la Fase 8 era
  sólo para el compositor `mirada`, no para terceros.
  - `llimphi-hal::RawSurface` — `Surface` sobre una `wgpu::Surface` creada desde
    handles raw, **sin `winit::Window`** (misma intermedia + blit que
    `WinitSurface`). Es la costura: el render de Llimphi ya era winit-free salvo
    la creación de la surface.
  - `pata-llimphi::layer` — backend `wlr-layer-shell` con
    `smithay-client-toolkit`: crea **una layer surface por cada superficie
    `Bar`** de la config (cada una anclada a su borde + `set_exclusive_zone`),
    saca su `wgpu::Surface` de los punteros `wl_display`/`wl_surface`, y la pinta
    reusando `mount → compute → paint → render` vía [`render::bar_view`]. Un
    `Hal` (instancia/device de wgpu) compartido; estado wgpu por panel
    (`PanelGpu`). Muestreo 1Hz compartido + flag `dirty` por panel (no
    re-rasteriza a 60fps). `main` elige layer-shell si hay `WAYLAND_DISPLAY`
    (salvo `PATA_BACKEND=winit`), con fallback a la ventana winit.
    Verificado en runtime (Hyprland): salen todas las barras ancladas, sin
    error, y el muestreo/leyenda quietos.
    - **Gotcha Vulkan WSI + smithay (mirada)** — `draw` redimensiona la surface
      en cada cuadro (no hay evento de resize como en winit), así que
      `RawSurface::resize` es **no-op cuando el tamaño no cambia**: reconfigurar
      el swapchain por cuadro reconstruye el `wl_buffer` y destruye el recién
      presentado antes de que el compositor lo componga — wlroots lo tolera,
      smithay (mirada) no, y la barra quedaba negra (`buffer=None`). `acquire`
      reconfigura+reintenta una vez ante `Outdated`/`Lost`. Fix en `b8747b90`.
  - **Input + Quake** ✅ (verificado en Hyprland): seat/keyboard/pointer
    vía sctk. Un cliente layer-shell **no recibe hotkeys globales**, así que el
    Quake se abre con **click** en la barra de shuma (foco de teclado vía
    `OnDemand` → al abrir pasa a `Exclusive`). En vez de una segunda surface, la
    propia barra de shuma **crece hacia arriba** hasta `DRAWER_H` (su exclusive
    zone queda en el grosor de la barra, así no recoloca el teselado);
    `render::shuma_open_view` pinta el cuerpo del drawer (input + salida) arriba
    y el cabezal abajo. Teclado con foco: Esc cierra, Backspace, Enter ejecuta
    (`shuma::ejecutar_stand_in`, `sh -c` bloqueante), texto → buffer.
  - **Clicks por hit-test** ✅ — cada panel guarda su árbol pintado
    (`RenderCache`: `Mounted` + `ComputedLayout`); al click, `hit_test_click`
    ubica el nodo bajo el puntero y dispara su `on_click` (vía `handle_msg`). El
    cabezal `› shuma` togglea con precisión (abre y cierra); clickear el reloj o
    un medidor no hace nada. Reemplaza al "cualquier click en la barra abre".
  - **Acciones por widget** ✅ — cualquier widget con una prop `exec` se vuelve
    clickeable (estilo waybar): `SlotWidget::Core { widget, exec }` lleva el
    comando; el render le pone `on_click(Msg::Spawn(cmd))` + `hover_fill`; ambos
    backends lo lanzan con `spawn_cmd` (`sh -c`, sin esperar). Ej. en el asset:
    `start_button` con `exec = "wofi --show drun"`.
  - **Exec asíncrono del Quake** ✅ — el `Enter` corre el comando en un hilo y el
    resultado llega por un `mpsc::Receiver` que el latido sondea (`try_recv`) cada
    frame; ya no bloquea el loop. Mientras corre, el drawer muestra `…`.
  - **Volumen real** ✅ — el sampler lee el volumen del sink por defecto vía
    PipeWire (`wpctl get-volume`) con fallback a PulseAudio (`pactl`), y rellena
    `WidgetCtx::{volume, muted}` (el brillo ya lo lee de `/sys`). El medidor deja
    de marcar 0%. Parseo en funciones puras testeadas (`parse_wpctl`/
    `parse_pactl_pct`). Bonus en el asset: `volume` con `exec = "pavucontrol"`.
  - **`window_list`** ✅ — la lista de ventanas abiertas, vía
    `wlr-foreign-toplevel-management` (`wayland-protocols-wlr`), el protocolo de
    waybar/eww. El manager se bindea opcional (si el compositor no lo expone, el
    widget queda vacío sin romper); cada toplevel acumula título/app_id/estado en
    `pata-llimphi::toplevel::Toplevel` y se confirma en `done`. El render pinta un
    chip clickeable por ventana (la activa resaltada); el click manda
    `Msg::ActivateWindow(id)` → `activate(seat)`, que la trae al frente. Como el
    `shuma_input`, es interacción + IPC: no pasa por el `build` agnóstico de core
    sino que lo intercepta el frontend (`SlotWidget::WindowList`); los datos se
    pasan al render aparte del view-model. El asset `launcher.toml` ya lo tiene en
    el centro de la barra superior.
  - **`clipboard`** ✅ — preview del texto copiado. El sampler lo lee con
    `wl-paste --no-newline --type text/plain` (subproceso ~1Hz, como el volumen
    con `wpctl`) y lo colapsa a una línea (`sampler::preview_clipboard`, testeada);
    el render pinta `📋 <preview>` recortado. Como `window_list`, es dato del host
    interceptado por el frontend (`SlotWidget::Clipboard`), no view-model de core;
    los datos del host (ventanas + portapapeles) viajan juntos en `render::BarData`.
    Una prop `exec` lo vuelve clickeable → selector de historial (en el asset:
    `cliphist list | wofi --dmenu | cliphist decode | wl-copy`). Sólo en
    layer-shell (el path winit pasa `BarData::default()`).
  - **`tray`** ✅ — la bandeja del sistema (StatusNotifierItem). pata corre como
    **watcher + host**: posee `org.kde.StatusNotifierWatcher` y atiende a las apps
    que registran su item. Como el bucle sctk es bloqueante y zbus es async, el
    tray vive en su **propio hilo** con un runtime tokio current-thread (el
    workspace fija zbus con la feature `tokio`, no la blocking — patrón de
    `mirada-portal`); comparte el snapshot de items por `Arc<Mutex>` y recibe los
    clicks por un canal tokio (como el exec del Quake). El render pinta un chip por
    item (resaltando `NeedsAttention`); el click manda `Msg::TrayActivate(key)` →
    `Activate(0,0)` por D-Bus. Interceptado por el frontend (`SlotWidget::Tray`),
    los items viajan en `render::BarData`. **Íconos** ✅: resuelve el `IconPixmap`
    (ARGB32 por D-Bus → RGBA, sin tema) y, si no, el `IconName` como PNG en los
    dirs estándar (hicolor + pixmaps, sólo PNG, sin `index.theme` ni SVG); cae a
    texto si nada resuelve. El hilo del tray decodifica a `TrayIcon{rgba}` y el
    render lo envuelve en `peniko::Image` (`View::image`, 18px). **No** emite
    señales del watcher ni hace fallback si ya hay un watcher (si el nombre está
    tomado, queda vacío y loguea). `split_service`
    normaliza el registro (ruta+remitente / nombre de bus / combinado), testeada.
    El tray sólo arranca si la config declara un widget `tray`. Ver `02_ruway/pata/
    pata-llimphi/src/tray.rs`.
  - **Fase 6 cerrada**: todos los widgets previstos (§4) existen, con íconos
    reales en el tray. **`clipboard` y `tray` cableados también en el path winit**
    (el `Model` muestrea el portapapeles cada tick y arranca el `TrayHandle` si la
    config lo pide; `render::root` arma el `BarData` desde el `Model`). El único
    pendiente es **`window_list` bajo el path winit/mirada**: necesita el cliente
    foreign-toplevel (que vive en el backend layer-shell) o el IPC de toplevels de
    mirada; hasta entonces queda vacío en ese path. Helper `config_tiene_widget`
    compartido por ambos backends para arrancar el tray sólo si hace falta.
- **Fase 8c — pulido de escritorio** (en curso):
  - **Gradiente en los medidores** ✅ — la barra de relleno de cpu/ram/volumen/
    brillo pinta un gradiente lineal (acento → acento aclarado) con `paint_with`
    (Llimphi sólo tiene fill de color sólido). Ambos backends.
  - **Task manager estilo KDE** ✅ — el `window_list` pasa de chips planos a
    botones con ícono-badge (inicial del `app_id`) + título; la activa resaltada,
    las minimizadas atenuadas. Clic izq. activa o **minimiza** si ya estaba activa;
    clic der. **cierra** (`Msg::CloseWindow` → `handle.close()`). `Toplevel`
    trackea `minimized`; el pointer del layer-shell rutea `BTN_RIGHT`.
  - **Tarjetas flotantes (conky)** ✅ — `SurfaceKind::Panel` + `FloatingCard` ya
    estaban en el modelo; ahora `render::card_view` las pinta y el layer-shell crea
    **una layer surface por tarjeta** en `Layer::Bottom` (sobre el escritorio,
    anclada a la esquina sup-izq con margen (x,y), sin reservar franja ni teclado).
    En winit se pintan en absoluto. Asset con una tarjeta `sistema`.
  - **Botón de inicio con menú nativo** ✅ — el `start_button` despliega un menú
    de apps del registro (`app-bus AppRegistry::discover`). En layer-shell la barra
    superior crece hacia abajo (truco del drawer Quake, al revés); en winit sale
    por `view_overlay`. `exec` en el spec lo deja delegando a un lanzador externo.
  - **Hover en todos los widgets** ✅ — el layer-shell pasaba `None` a `paint`, así
    que `hover_fill` estaba muerto; ahora trackea `Motion`/`Leave` →
    `hit_test_hover` → `hover_idx`. Todos los widgets dan realce al pasar el cursor.
  - **Tooltip flotante (texto)** ⏳ — pendiente: necesita una surface popup aparte
    (las barras son finas y recortan), a validar en compositor. La base (hover
    tracking) ya está.
- **Fase 9** — kernel launcher de wawa sobre `pata-core`.
- **Fase 10** — retirar `mirada-launcher-llimphi` (migrado a pata).
