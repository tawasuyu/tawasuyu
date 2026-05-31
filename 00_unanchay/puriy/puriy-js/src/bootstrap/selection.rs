//! `getSelection` + `Selection` + `Range` (stubs). Fase 7.73.
//!
//! La API de selección de texto. Apps la consultan para features de copy
//! (`window.getSelection().toString()`), para limpiar selección
//! (`getSelection().removeAllRanges()`), o para construir rangos
//! (`document.createRange()`). Sin las funciones, ese código tira
//! `is not a function` y la rama se cae.
//!
//! Puriy headless no tiene una selección visual del usuario, así que:
//! - `getSelection()` devuelve un `Selection` **vacío** singleton
//!   (`rangeCount: 0`, `isCollapsed: true`, `type: 'None'`, `toString()` →
//!   `''`). Los métodos mutadores (`removeAllRanges`, `addRange`,
//!   `collapse`, …) son no-ops que no crashean.
//! - `document.createRange()` devuelve un `Range` mínimo con los métodos
//!   estructurales como no-ops y `toString()` → `''`.
//!
//! **Limitaciones explícitas**:
//! 1. **Selección siempre vacía** — `getSelection().toString()` nunca
//!    devuelve texto seleccionado (no hay UI de selección headless). Apps
//!    de "copiar lo seleccionado" no obtienen contenido.
//! 2. **`Range` no opera sobre el DOM** — `setStart`/`setEnd`/`selectNode`
//!    /`deleteContents` son no-ops; `getBoundingClientRect()` → rect cero.
//! 3. **`createRange` se cuelga de `document`** sólo si `document` ya
//!    existe al correr el bootstrap; como `document` se crea en
//!    `set_document`, también lo instalamos en `__puriy_install_selection`
//!    que set_document podría invocar. Por ahora lo agregamos perezoso:
//!    `getSelection` global siempre está; `document.createRange` se intenta
//!    si `document` existe.

pub(crate) const SELECTION_BOOTSTRAP: &str = r#"
(function(){
  if (typeof globalThis.getSelection === 'function') return;

  function makeRange() {
    return {
      collapsed: true,
      startContainer: null, endContainer: null,
      startOffset: 0, endOffset: 0,
      commonAncestorContainer: null,
      setStart: function() {}, setEnd: function() {},
      setStartBefore: function() {}, setStartAfter: function() {},
      setEndBefore: function() {}, setEndAfter: function() {},
      selectNode: function() {}, selectNodeContents: function() {},
      collapse: function() {}, cloneRange: function() { return makeRange(); },
      deleteContents: function() {}, extractContents: function() { return null; },
      insertNode: function() {}, surroundContents: function() {},
      getBoundingClientRect: function() {
        return { top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, x: 0, y: 0 };
      },
      getClientRects: function() { return []; },
      toString: function() { return ''; },
      detach: function() {}
    };
  }

  var selection = {
    anchorNode: null, anchorOffset: 0,
    focusNode: null, focusOffset: 0,
    isCollapsed: true,
    rangeCount: 0,
    type: 'None',
    removeAllRanges: function() {},
    empty: function() {},
    addRange: function() {},
    removeRange: function() {},
    getRangeAt: function() { return makeRange(); },
    collapse: function() {}, collapseToStart: function() {}, collapseToEnd: function() {},
    extend: function() {},
    selectAllChildren: function() {},
    deleteFromDocument: function() {},
    containsNode: function() { return false; },
    toString: function() { return ''; }
  };

  globalThis.getSelection = function() { return selection; };
  // Instalador para que set_document cuelgue createRange/getSelection del
  // document recién creado (idempotente).
  globalThis.__puriy_install_selection = function(doc) {
    if (!doc) return;
    if (typeof doc.getSelection !== 'function') doc.getSelection = globalThis.getSelection;
    if (typeof doc.createRange !== 'function') doc.createRange = makeRange;
  };
  // Si document ya existe (raro en el bootstrap inicial), instalalo ya.
  if (globalThis.document) { globalThis.__puriy_install_selection(globalThis.document); }
})();
"#;
