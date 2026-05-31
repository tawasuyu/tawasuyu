//! `document.createTreeWalker` + `document.createNodeIterator` + `NodeFilter`.
//! Fase 7.101.
//!
//! Las dos APIs estándar de recorrido del DOM. Apps las usan para barrer
//! un subárbol aplicando un filtro (resaltadores de texto, extractores de
//! contenido, sanitizadores, libs que "caminan" el árbol sin recursión
//! manual): `var w = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT,
//! { acceptNode: function(n){ return n.tagName === 'A' ? 1 : 3; } });
//! while (w.nextNode()) { ... }`. Sin las funciones, ese código tira
//! `is not a function` y la rama se cae.
//!
//! Implementación **fiel a los algoritmos WHATWG DOM** (traverseChildren /
//! traverseSiblings / nextNode / previousNode para el TreeWalker; following/
//! preceding en orden de documento para el NodeIterator), operando sobre la
//! relación `parentElement` / `children` que ya expone el registro de
//! elementos (`__puriy_elements`, Fase 7.13). `NodeFilter` se cuelga de
//! `globalThis` con las constantes `SHOW_*` / `FILTER_*`.
//!
//! **Limitaciones explícitas**:
//! 1. **Sólo elementos** — el modelo JS no expone text nodes ni comments
//!    (Fase 7.5b), así que `whatToShow` sólo distingue "muestra elementos"
//!    (`SHOW_ELEMENT` / `SHOW_ALL`) de "no muestra nada". Un walker con
//!    `SHOW_TEXT` recorre en vacío (nuestros nodos no son text nodes).
//! 2. **No es vivo ante mutaciones durante el recorrido** — el NodeIterator
//!    real ajusta su `referenceNode` cuando se borra el nodo apuntado
//!    (pre-removing steps); acá no, porque no hay un punto de intercepción
//!    para todas las rutas de borrado. Iterar y mutar a la vez es UB.
//! 3. **`filter` acepta función u objeto `{acceptNode}`** (ambas formas del
//!    spec); el valor de retorno se castea a `1`/`2`/`3` (ACCEPT/REJECT/SKIP).

pub(crate) const TREE_WALKER_BOOTSTRAP: &str = r#"
(function(){
  if (globalThis.NodeFilter) return;

  globalThis.NodeFilter = {
    SHOW_ALL: 0xFFFFFFFF,
    SHOW_ELEMENT: 0x1,
    SHOW_ATTRIBUTE: 0x2,
    SHOW_TEXT: 0x4,
    SHOW_COMMENT: 0x80,
    SHOW_DOCUMENT: 0x100,
    FILTER_ACCEPT: 1,
    FILTER_REJECT: 2,
    FILTER_SKIP: 3
  };

  // ---- accesores sobre el registro de elementos ----
  function firstChildOf(n) { var c = n.children; return (c && c.length) ? c[0] : null; }
  function lastChildOf(n) { var c = n.children; return (c && c.length) ? c[c.length - 1] : null; }
  function parentOf(n) { return n.parentElement || null; }
  function siblingOf(n, dir) {
    var p = parentOf(n); if (!p) return null;
    var c = p.children;
    for (var i = 0; i < c.length; i++) {
      if (c[i] === n) {
        var j = i + dir;
        return (j >= 0 && j < c.length) ? c[j] : null;
      }
    }
    return null;
  }
  function nextSiblingOf(n) { return siblingOf(n, 1); }
  function prevSiblingOf(n) { return siblingOf(n, -1); }

  function makeFilterFn(whatToShow, filter) {
    var show = whatToShow >>> 0;
    return function(node) {
      // nuestros nodos son todos elementos (nodeType 1, bit SHOW_ELEMENT=0x1).
      if (show !== 0xFFFFFFFF && !(show & 0x1)) return 3; // SKIP: no se muestran
      if (filter == null) return 1;
      var r;
      if (typeof filter === 'function') r = filter(node);
      else if (typeof filter.acceptNode === 'function') r = filter.acceptNode(node);
      else return 1;
      r = r >>> 0;
      return (r === 2 || r === 3) ? r : 1;
    };
  }

  // ---- TreeWalker (WHATWG DOM §6.2) ----
  function makeTreeWalker(root, whatToShow, filter) {
    var current = root;
    var filterNode = makeFilterFn(whatToShow, filter);

    function traverseChildren(type) {
      var node = (type === 'first') ? firstChildOf(current) : lastChildOf(current);
      while (node != null) {
        var result = filterNode(node);
        if (result === 1) { current = node; return node; }
        if (result === 3) {
          var child = (type === 'first') ? firstChildOf(node) : lastChildOf(node);
          if (child != null) { node = child; continue; }
        }
        // result === 2 (REJECT) o SKIP sin hijos: a sibling, luego subir.
        while (node != null) {
          var sibling = (type === 'first') ? nextSiblingOf(node) : prevSiblingOf(node);
          if (sibling != null) { node = sibling; break; }
          var parent = parentOf(node);
          if (parent == null || parent === root || parent === current) return null;
          node = parent;
        }
      }
      return null;
    }

    function traverseSiblings(type) {
      var node = current;
      if (node === root) return null;
      while (true) {
        var sibling = (type === 'next') ? nextSiblingOf(node) : prevSiblingOf(node);
        while (sibling != null) {
          node = sibling;
          var result = filterNode(node);
          if (result === 1) { current = node; return node; }
          sibling = (type === 'next') ? firstChildOf(node) : lastChildOf(node);
          if (result === 2) sibling = (type === 'next') ? nextSiblingOf(node) : prevSiblingOf(node);
        }
        node = parentOf(node);
        if (node == null || node === root) return null;
        if (filterNode(node) === 1) return null;
      }
    }

    var walker = {
      root: root,
      whatToShow: whatToShow >>> 0,
      filter: filter || null,
      parentNode: function() {
        var node = current;
        while (node != null && node !== root) {
          node = parentOf(node);
          if (node != null && filterNode(node) === 1) { current = node; return node; }
        }
        return null;
      },
      firstChild: function() { return traverseChildren('first'); },
      lastChild: function() { return traverseChildren('last'); },
      nextSibling: function() { return traverseSiblings('next'); },
      previousSibling: function() { return traverseSiblings('previous'); },
      nextNode: function() {
        var node = current;
        var result = 1;
        while (true) {
          while (result !== 2 && firstChildOf(node) != null) {
            node = firstChildOf(node);
            result = filterNode(node);
            if (result === 1) { current = node; return node; }
          }
          var sibling = null, temporary = node;
          while (temporary != null) {
            if (temporary === root) return null;
            sibling = nextSiblingOf(temporary);
            if (sibling != null) { node = sibling; break; }
            temporary = parentOf(temporary);
          }
          if (sibling == null) return null;
          result = filterNode(node);
          if (result === 1) { current = node; return node; }
        }
      },
      previousNode: function() {
        var node = current;
        while (node !== root) {
          var sibling = prevSiblingOf(node);
          while (sibling != null) {
            node = sibling;
            var result = filterNode(node);
            while (result !== 2 && lastChildOf(node) != null) {
              node = lastChildOf(node);
              result = filterNode(node);
            }
            if (result === 1) { current = node; return node; }
            sibling = prevSiblingOf(node);
          }
          if (node === root) return null;
          var parent = parentOf(node);
          if (parent == null) return null;
          node = parent;
          if (filterNode(node) === 1) { current = node; return node; }
        }
        return null;
      }
    };
    Object.defineProperty(walker, 'currentNode', {
      get: function() { return current; },
      set: function(v) { current = v; },
      enumerable: true
    });
    return walker;
  }

  // ---- NodeIterator (WHATWG DOM §6.1) ----
  function followingOf(node, root) {
    var fc = firstChildOf(node);
    if (fc != null) return fc;
    var n = node;
    while (n != null) {
      if (n === root) return null;
      var s = nextSiblingOf(n);
      if (s != null) return s;
      n = parentOf(n);
    }
    return null;
  }
  function precedingOf(node, root) {
    if (node === root) return null;
    var s = prevSiblingOf(node);
    if (s != null) {
      var n = s;
      var lc = lastChildOf(n);
      while (lc != null) { n = lc; lc = lastChildOf(n); }
      return n;
    }
    return parentOf(node);
  }

  function makeNodeIterator(root, whatToShow, filter) {
    var reference = root;
    var pointerBefore = true;
    var filterNode = makeFilterFn(whatToShow, filter);

    function traverse(direction) {
      var node = reference;
      var before = pointerBefore;
      while (true) {
        if (direction === 'next') {
          if (!before) {
            node = followingOf(node, root);
            if (node == null) return null;
          } else { before = false; }
        } else {
          if (before) {
            node = precedingOf(node, root);
            if (node == null) return null;
          } else { before = true; }
        }
        if (filterNode(node) === 1) break;
      }
      reference = node; pointerBefore = before;
      return node;
    }

    var it = {
      root: root,
      whatToShow: whatToShow >>> 0,
      filter: filter || null,
      nextNode: function() { return traverse('next'); },
      previousNode: function() { return traverse('previous'); },
      detach: function() {}
    };
    Object.defineProperty(it, 'referenceNode', { get: function() { return reference; }, enumerable: true });
    Object.defineProperty(it, 'pointerBeforeReferenceNode', { get: function() { return pointerBefore; }, enumerable: true });
    return it;
  }

  // ---- instalador colgado de document (set_document lo invoca) ----
  globalThis.__puriy_install_tree_walker = function(doc) {
    if (!doc) return;
    if (typeof doc.createTreeWalker !== 'function') {
      doc.createTreeWalker = function(root, whatToShow, filter) {
        if (root == null) throw new TypeError("createTreeWalker: root requerido");
        return makeTreeWalker(root, (whatToShow == null) ? 0xFFFFFFFF : whatToShow, filter);
      };
    }
    if (typeof doc.createNodeIterator !== 'function') {
      doc.createNodeIterator = function(root, whatToShow, filter) {
        if (root == null) throw new TypeError("createNodeIterator: root requerido");
        return makeNodeIterator(root, (whatToShow == null) ? 0xFFFFFFFF : whatToShow, filter);
      };
    }
  };
  if (globalThis.document) globalThis.__puriy_install_tree_walker(globalThis.document);
})();
"#;
