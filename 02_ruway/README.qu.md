<!-- Quechua (Cusco/Collao). Revisión por hablante nativo bienvenida. -->

# 02 ruway · ruway

`ruway` (runa-simi: *ruway — ruwakuy, llank'ay*). Kaymi **ruway** *cuadrante*: interfaces, compositorkuna, brokers, shellkuna. Imayna `unanchay`pi rikurqanchik, `yachay`pi modelorqanchik, kaypi runa imapas usaq, huk piezakunawan tinkuq, hinaspa binario hina compilakuq imayhanaqkitaq.

Kay cuadrantepa kamachiyninmi **k'irimi kamachin**: huk widget mana ñawpaq dibujullapi munasqachu, `vello` `taffy` atisqankuwan munasqa; huk compositor mana qhasi munasqachu, `weston`-pa kasqanwan tupachisqa.

## Aplicaciones

- **[chasqui](chasqui/README.md)** — *chasqui* (mensajero del camino-inka). Broker mensaheykuna + tipo-bus. Monorepopa nervio.
- **[llimphi](llimphi/README.md)** — UI framework natural (`hal · raster · layout · text · theme · ui`) + widgets + modules. Llapan aplikacionkuna kamachiq imaymana ñawi.
- **[mirada](mirada/README.md)** — Wayland compositor + XDG portal + login greeter. Display pacha.
- **[nada](nada/README.md)** — qillqana editor, Llimphi pataman. File tree + editor LSPwan + clipboard real + sesiones. Framework yachachiq mancha.
- **[nahual](nahual/README.md)** — sapankuna ñawi: archivo shell, qillqa-ñawi, siq'i-ñawi.
- **[shuma](shuma/README.md)** — shell interactivo (zsh/fish parity), Llimphi chasispi 4 ñawi (TopBar/Main/BottomBar/Drawer).
- **[supay](supay/README.md)** — DOOM-laya renderer Llimphi pataman (FFI `doomgeneric`-man, sprite atlas, WAD paletas).
- **[takiy](takiy/README.md)** — *takiy* (cantar). Música — capture, secuencia, audio render.
- **[wawa](wawa/README.md)** — control panel + `wawactl` Wawapaq (kernel paipa userspace pares, `03_ukupacha/wawa`-pa).

## Manifesto

> **Ruwayqa k'irimanta hap'iy.**
> Huk API manan kanchu iskay aplikacion mana usaspa; huk widget manan kanchu chiqaq pantalla 60 fps mana riqsinata kashaqtin.
>
> 1. **Mana grafiko dep-kuna `core`-pi.** Motor kamachin, UI rikuchin — manaraqcha kikin crate-pi.
> 2. **Kikin escena Wayland-pi Wawa-pipas.** Llimphi/HAL superficiekunata huñun; huk-kunaqa kaqlla.
> 3. **Runa pachatachus kamachin.** Frame mana atiqtin, ñawpaq pisillamanta qillqasun, huk kompútu mañakunmanqa.
> 4. **Herramientakuna llank'aqman yupaychaspa.** Kaqlla atajos, undo seguro, clipboard chiqaq sistemaq.
