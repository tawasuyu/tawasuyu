# 02 ruway · hacer

`ruway` (quechua: *hacer, obrar, fabricar*). Es el cuadrante de la **acción**: las interfaces, los compositores, los brokers, los shells. Lo que `unanchay` percibió y `yachay` modeló se vuelve aquí algo que el humano usa, que se compone con otras piezas, que se compila a un binario que arranca y responde.

La regla del cuadrante es **el material manda**: un widget no se diseña pensando en mockups, se diseña con lo que `vello` y `taffy` pueden hacer; un compositor no se diseña en abstracto, se mide contra `weston`. La materialidad limita y guía.

## Aplicaciones

- **[ayni](ayni/LEEME.md)** — chat persona-a-persona soberano, local-first, sin servidor: la conversación como grafo criptográfico reproducible (BLAKE3 + DAG), identidad `agora`, transporte `chasqui`/`minga`.
- **[cards](cards/LEEME.md)** — una sola manera de leer toda Card: proyecta los documentos Card de la suite (runtime, semánticos, de UI) sobre una única estructura canónica.
- **[chasqui](chasqui/README.md)** — broker de mensajería + bus tipado. El sistema nervioso del monorepo.
- **[llimphi](llimphi/README.md)** — framework de UI nativa (hal · raster · layout · text · theme · ui) + widgets + modules. El núcleo gráfico que comparten todas las apps.
- **[media](media/LEEME.md)** — el dominio audio/video de la suite: player, decoders, visualizadores, grabadora.
- **[mirada](mirada/README.md)** — compositor Wayland (`mirada-compositor`) + portal XDG (`mirada-portal`) + greeter de login (`mirada-greeter`). La pila de display.
- **[nada](nada/README.md)** — editor de archivos sobre Llimphi: file tree + editor con LSP + clipboard real + sesiones. Banco de pruebas del framework.
- **[nahual](nahual/README.md)** — visores cotidianos: shell de archivos, viewer de texto, viewer de imagen.
- **[paloma](paloma/LEEME.md)** — cliente de correo nativo sobre Llimphi: IMAP de entrada, SMTP de salida, sin navegador de por medio.
- **[pata](pata/LEEME.md)** — el marco del escritorio: barras, paneles y dock declarativos desde un archivo de config; mismo modelo en Linux y en Wawa.
- **[raymi](raymi/LEEME.md)** — calendario + contactos nativos (CalDAV/CardDAV), el compañero de paloma; reusa su capa de cuentas.
- **[shuma](shuma/README.md)** — shell interactivo (zsh/fish-paridad) con vistas en chasis Llimphi (TopBar/Main/BottomBar/Drawer).
- **[supay](supay/README.md)** — renderer estilo DOOM sobre Llimphi (FFI a `doomgeneric`, atlas de sprites, paletas WAD).
- **[takiy](takiy/README.md)** — música. Captura, secuenciación, render audio.
- **[tullpu](tullpu/LEEME.md)** — editor de imágenes por capas donde nada se destruye: la pila de capas es un DAG content-addressed; las capas derivadas quedan *stale* en vez de sobreescribir.
- **[uya](uya/README.md)** — videollamada soberana (`uya` = "cara" en quechua): `uya-core` agnóstico + frontends Llimphi sobre el nodo P2P de la suite (`card-net`).
- **[wawa](wawa/README.md)** — panel de control + `wawactl` para la pila Wawa (la pareja userspace del kernel de `03_ukupacha/wawa`).

## Manifiesto

> **Hacer es comprometerse con la materia.**
> Una API no existe hasta que la usa una segunda app; un widget no existe hasta que se ve renderizado a 60 fps en pantalla real.
>
> 1. **Cero deps gráficas en `core`.** El motor decide; la UI muestra — y son crates distintos.
> 2. **El mismo árbol gráfico en Wayland y en Wawa.** Llimphi/HAL abstrae la superficie; el resto del stack es idéntico.
> 3. **El usuario manda el ritmo.** Si el frame se atrasa, simplificamos antes de pedir más cómputo.
> 4. **Herramientas que respeten al artesano.** Atajos consistentes, undo confiable, clipboard que funciona con el sistema real.
