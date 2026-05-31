//! `window.visualViewport` (Visual Viewport API). Fase 7.95.
//!
//! Apps móviles/responsive consultan `window.visualViewport` para reaccionar
//! al teclado virtual, pinch-zoom y la barra de URL que aparece/desaparece:
//! `visualViewport.height`, `.scale`, `.offsetTop`, y eventos `resize`/`scroll`.
//! Sin el objeto, `visualViewport.addEventListener` tira `is not a function`.
//!
//! Derivamos `width`/`height` del viewport conocido (`__puriy_inner_width`/
//! `__puriy_inner_height`, Fase 7.28) — igual molde que `screen` (Fase 7.72).
//! Sin zoom ni teclado virtual headless: `scale` = 1, `offsetLeft`/`offsetTop`
//! = 0; `pageLeft`/`pageTop` siguen el scroll de la página
//! (`scrollX`/`scrollY` si existen).
//!
//! **Limitaciones explícitas**:
//! 1. **No reactivo** — los eventos `resize`/`scroll` nunca disparan (el
//!    chrome no notifica cambios de viewport); cuando llegue `Msg::Resize`,
//!    recorrer los listeners vivos (igual pendiente que `matchMedia`, 7.61).
//! 2. **`scale` fijo en 1** — sin pinch-zoom; apps que ajustan UI por zoom
//!    ven siempre 1×.
//! 3. **`visualViewport` = layout viewport** — sin distinción visual/layout
//!    (no hay teclado virtual que recorte el área visible).

pub(crate) const VISUAL_VIEWPORT_BOOTSTRAP: &str = r#"
(function(){
  if (globalThis.visualViewport) return;

  function vw() { return (typeof globalThis.__puriy_inner_width === 'number') ? globalThis.__puriy_inner_width : 1024; }
  function vh() { return (typeof globalThis.__puriy_inner_height === 'number') ? globalThis.__puriy_inner_height : 768; }

  var vv = {
    get width() { return vw(); },
    get height() { return vh(); },
    get scale() { return 1; },
    get offsetLeft() { return 0; },
    get offsetTop() { return 0; },
    get pageLeft() { return (typeof globalThis.scrollX === 'number') ? globalThis.scrollX : 0; },
    get pageTop() { return (typeof globalThis.scrollY === 'number') ? globalThis.scrollY : 0; },
    onresize: null,
    onscroll: null,
    addEventListener: function() {},
    removeEventListener: function() {},
    dispatchEvent: function() { return true; }
  };

  globalThis.visualViewport = vv;
  // window === globalThis tras set_document; cubrimos el caso explícito.
  if (globalThis.window && !globalThis.window.visualViewport) {
    globalThis.window.visualViewport = vv;
  }
})();
"#;
