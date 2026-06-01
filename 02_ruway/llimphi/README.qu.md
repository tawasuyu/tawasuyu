<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# llimphi

> Natural UI framework: HAL · raster · layout · text · theme · ui — widgetkuna + modules.

`llimphi` monorepupa llapan apps tukuyniqlla grafico motor. Retained-mode declarativo pipeline (`vello` + `wgpu` + `taffy`), `fontdue`/`harfbuzz` shaping, `Dark/Light/Aurora/Sunset` themes, multi-superficie HAL (Wayland · X11 · Win32 · Android · Wawa). Detalle [SDD.md](SDD.md)-pi.

**Imayna llamk'ana qillqa (manual):** [MANUAL.md](MANUAL.md) — hunt'asqa referencia (Elm muyuy, `View<Msg>` DSL, ~44 widgetkuna, 10 modulekuna, GPU ñan). Runakunapaq IA-paqpas.

Yuyaynin: **widget mana mockuppi munakun; vello + taffy atisqankuwan ruwasqa.**

## Churay

```sh
[dependencies]
llimphi-ui = { workspace = true }
llimphi-theme = { workspace = true }
```

## Tinkuy

- **Linux/Wayland** — ñawpaq backend.
- **Linux/X11** — XWayland-rayku.
- **macOS / Windows** — `winit` + `wgpu`.
- **Android** — `android` cratekuna HAL.
- **Wawa bare-metal** — sapan framebuffer HAL.

Crateskunaq listako [README.md](README.md)-pi.

## Yuyaykunaq

- **Sapan API: declarativo `View<Msg>`.** Mana imperativo, mana hawanka vDOM.
- **Kikin escena Wayland Wawapipas**: HAL superficie huñun.
- Widgets **ch'uya rikuq**; módulos estado + ruway huñun.
