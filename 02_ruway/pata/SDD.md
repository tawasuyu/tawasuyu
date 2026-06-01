# SDD — `pata`, el marco del escritorio

> Estado: **Fase 1** (núcleo del modelo). Este documento es la fuente autoritativa
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
pata-core      Modelo agnóstico (no_std + alloc). El esquema declarativo:
               Config → [Surface] → slots → [WidgetSpec]. No pinta, no toca
               el SO, no sabe quién lo renderiza. Cruza la frontera a wawa
               por `path`, como mirada-layout.
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

- **Fase 1 ✅** — `pata-core`: esquema declarativo, `Prop` agnóstico, contrato
  TOML, `no_std` verificado (wasm32). En el workspace.
- **Fase 2** — modelo de widget agnóstico (trait `tick`/`view` que devuelve un
  view-model, no Llimphi) + lógica de datos de los widgets portada al core.
- **Fase 3** — `pata-llimphi`: render de superficies sobre Llimphi; hereda
  `mirada-launcher-llimphi`. App-id que mirada acopla por superficie.
- **Fase 4** — widgets nuevos: `start_button`, `window_list` (IPC con mirada),
  `tray`, `astro` (cosmos).
- **Fase 5** — despliegue Quake de shuma desde `shuma_input`.
- **Fase 6** — mirada-compositor reconoce superficies pata (varios anchors, no
  una sola franja de 40px al pie); `SHELL_APP_ID` → identidad pata + override
  por env.
- **Fase 7** — kernel launcher de wawa sobre `pata-core`.
- **Fase 8** — retirar `mirada-launcher-llimphi` (migrado a pata).
