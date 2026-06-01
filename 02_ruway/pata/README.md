# pata

> The desktop frame: declarative bars, panels and a dock — widgets you place
> anywhere, from one config file. The same model on Linux and on Wawa.

`pata` (Quechua: *edge, ledge, terrace*) is the chrome layer of the gioser
desktop. It is **not** the compositor (`mirada`) nor the shell (`shuma`): it is
the configurable frame that surrounds the windows. From a config file you deploy
**bars**, **panels** and a **dock**, and inside them arrange widgets — start
button, open-window list, clipboard / volume / brightness, tray, clock, an
**astro** widget (the Sun's zodiac position + lunar cycle), and the shell input
that unfolds `shuma` Quake-style.

The model lives in `pata-core`, agnostic and `no_std`, so the very same frame
runs as a Llimphi frontend on Linux (over the `mirada` compositor) and from the
Wawa kernel launcher.

See [`SDD.md`](SDD.md) for the canonical definition and the phase plan.

## Crates

| Crate | Role |
|---|---|
| [`pata-core`](pata-core/) | Agnostic model: `Config → [Surface] → slots → [WidgetSpec]`. `no_std + alloc`. |

(`pata-llimphi` and the Wawa launcher land in later phases.)
