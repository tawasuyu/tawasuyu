# SDD — `pata`, el marco del escritorio

> Estado: **Fase 5** (frontend Llimphi). Este documento es la fuente autoritativa
> de qué es `pata` y dónde termina, por encima de cualquier README.

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
- **Fase 7** — despliegue Quake de shuma desde `shuma_input`.
- **Fase 8** — mirada-compositor reconoce superficies pata (varios anchors, no
  una sola franja de 40px al pie); `SHELL_APP_ID` → identidad pata + override
  por env.
- **Fase 9** — kernel launcher de wawa sobre `pata-core`.
- **Fase 10** — retirar `mirada-launcher-llimphi` (migrado a pata).
