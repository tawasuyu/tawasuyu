# SDD — `pata`, el marco del escritorio

> Estado: **Fase 8** (acople en mirada — zonas exclusivas). Este documento es la
> fuente autoritativa de qué es `pata` y dónde termina, por encima de README.

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
  - `window_list` ⏳ — necesita que mirada exponga los toplevels por IPC
    (`mirada-ctl`/`-link` aún no lo hacen); queda como placeholder.
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
  - Falta: los widgets placeholder (`window_list`/`tray`/`clipboard`, Fase 6) y
    leer el volumen real en el sampler (hoy 0%).
- **Fase 9** — kernel launcher de wawa sobre `pata-core`.
- **Fase 10** — retirar `mirada-launcher-llimphi` (migrado a pata).
