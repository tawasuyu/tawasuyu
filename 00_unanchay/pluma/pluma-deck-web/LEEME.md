# pluma-deck-web

> Deck en navegador para [pluma](../README.md).

Toma un `Deck` de [`pluma-deck-core`](../pluma-deck-core/README.md) y lo renderiza como SPA: slide actual + nav (←/→ teclas), aspect ratio configurable, full-screen, soporta el mismo theme system que el reader.

## API

```rust
use pluma_deck_web::Presenter;

let p = Presenter::new(container);
p.cargar(&deck);
```

## Modo espacial — Recorrido (tipo Prezi)

Espejo web del frontend Llimphi (`pluma-deck-recorrido-llimphi`): en vez del
strip 1D, un **lienzo infinito** con marcos en coordenadas de mundo y una
cámara que vuela entre ellos. La lógica (cámara/ruta/gesto) vive en
[`pluma-deck-core`](../pluma-deck-core/README.md); el binding (`recorrido`)
sólo aplica la cámara como **un único `transform` CSS** sobre `mundo`
(estilo impress.js) y delega el vuelo entre pasos a una transición CSS.

### Contrato DOM

```html
<div class="recorrido-viewport">
  <div class="recorrido-mundo">
    <div class="recorrido-marco" data-x="0"   data-y="0" data-w="640" data-h="400">…</div>
    <div class="recorrido-marco" data-x="900" data-y="0" data-w="640" data-h="400" data-rot="0.1">…</div>
  </div>
</div>
```

Cada marco lleva su rect de **mundo** en `data-{x,y,w,h}` (px) y un giro opcional
`data-rot` (radianes). El orden DOM define la ruta. El contenido HTML interno es
libre (texto, `<img>`, etc.).

### API

```rust
use pluma_deck_web::recorrido::RecorridoWeb;

let r = RecorridoWeb::mount(viewport, mundo)?;
r.on_change(|paso| { /* … */ });
// flechas/espacio/enter avanzan; rueda = zoom-a-cursor; arrastrar = paneo.
r.siguiente();  r.anterior();  r.goto(2, true);
```

## Deps

- [`pluma-deck-core`](../pluma-deck-core/README.md), [`pluma-md`](../pluma-md/README.md)
- `wasm-bindgen`, `web-sys`
