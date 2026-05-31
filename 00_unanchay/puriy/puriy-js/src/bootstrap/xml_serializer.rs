//! `DOMParser` + `XMLSerializer` (mínimos). Fase 7.77.
//!
//! Parser/serializador de markup que libs usan para manipular fragmentos
//! HTML/XML fuera del documento vivo (`new DOMParser().parseFromString(html,
//! 'text/html')`, sanitizadores, plantillas). El runtime no tiene un parser
//! HTML JS-side (eso vive en `puriy-engine` con html5ever, host-side), así
//! que damos una implementación **mínima**: el documento parseado expone el
//! string crudo y queries básicas, sin árbol DOM real.
//!
//! - `DOMParser().parseFromString(str, type)` → un objeto doc-like con
//!   `documentElement`/`body` cuyo `textContent`/`innerHTML` reflejan el
//!   input, más `querySelector`/`querySelectorAll` que devuelven `null`/`[]`
//!   (no hay árbol para consultar).
//! - `XMLSerializer().serializeToString(node)` → si el node tiene
//!   `outerHTML` lo devuelve; si tiene `innerHTML`/`textContent`, eso;
//!   sino `String(node)`.
//!
//! Sin estas clases, `new DOMParser()` tira `is not a constructor` y la
//! rama se cae.
//!
//! **Limitaciones explícitas**:
//! 1. **No construye un árbol DOM** — `parseFromString(...).querySelector`
//!    siempre da `null`. Apps que parsean HTML y lo recorren por selectores
//!    no funcionan; las que sólo leen `documentElement.textContent` o
//!    re-serializan, sí. Un parser real requeriría portar html5ever a WASM
//!    o exponer el del engine al runtime (fase futura).
//! 2. **`serializeToString`** es best-effort sobre las props del node.

pub(crate) const XML_SERIALIZER_BOOTSTRAP: &str = r#"
(function(){
  if (typeof globalThis.DOMParser === 'function') return;

  function makeParsedDoc(str, type) {
    var content = (str == null) ? '' : String(str);
    var leaf = {
      textContent: content,
      innerHTML: content,
      outerHTML: content,
      querySelector: function() { return null; },
      querySelectorAll: function() { return []; },
      getElementsByTagName: function() { return []; },
      getElementById: function() { return null; }
    };
    return {
      contentType: type || 'text/html',
      documentElement: leaf,
      body: leaf,
      head: leaf,
      textContent: content,
      querySelector: function() { return null; },
      querySelectorAll: function() { return []; },
      getElementsByTagName: function() { return []; },
      getElementById: function() { return null; },
      createElement: function(tag) {
        return { tagName: String(tag).toUpperCase(), textContent: '', innerHTML: '', children: [] };
      }
    };
  }

  function DOMParser() {}
  DOMParser.prototype.parseFromString = function(str, type) {
    return makeParsedDoc(str, type);
  };
  globalThis.DOMParser = DOMParser;

  function XMLSerializer() {}
  XMLSerializer.prototype.serializeToString = function(node) {
    if (node == null) return '';
    if (typeof node.outerHTML === 'string') return node.outerHTML;
    if (typeof node.innerHTML === 'string') return node.innerHTML;
    if (typeof node.textContent === 'string') return node.textContent;
    return String(node);
  };
  globalThis.XMLSerializer = XMLSerializer;
})();
"#;
