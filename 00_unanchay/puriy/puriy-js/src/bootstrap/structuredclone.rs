pub(crate) const STRUCTURED_CLONE_BOOTSTRAP: &str = r#"
// Fase 7.65 — structuredClone(value). Deep clone del structured clone
// algorithm: preserva referencias compartidas y ciclos (memo `seen`), copia
// Date/RegExp/ArrayBuffer/TypedArray/DataView/Blob/File/Map/Set/Array/objeto
// plano. Funciones y Symbols tiran DataCloneError (spec). Net-adjacent:
// postMessage, history.state, almacenar respuestas sin aliasing.
//
// Divergencias documentadas: (1) `options.transfer` se ignora (no hay
// transferables reales); (2) clases custom pierden su prototype (se clonan
// como objeto plano, igual que el spec real para non-cloneable platform
// objects — acá lo aplicamos a todo); (3) getters se materializan a su valor.
globalThis.structuredClone = function(value, options) {
    var seen = new Map();
    function clone(v) {
        if (v === null || typeof v !== 'object') {
            if (typeof v === 'function') throw new Error('DataCloneError: no se puede clonar una función');
            if (typeof v === 'symbol') throw new Error('DataCloneError: no se puede clonar un Symbol');
            return v;
        }
        if (seen.has(v)) return seen.get(v);
        if (v instanceof Date) { var d = new Date(v.getTime()); seen.set(v, d); return d; }
        if (v instanceof RegExp) { var r = new RegExp(v.source, v.flags); seen.set(v, r); return r; }
        if (v instanceof ArrayBuffer) { var ab = v.slice(0); seen.set(v, ab); return ab; }
        if (ArrayBuffer.isView(v)) {
            var buf = clone(v.buffer);
            var view = (v instanceof DataView)
                ? new DataView(buf, v.byteOffset, v.byteLength)
                : new v.constructor(buf, v.byteOffset, v.length);
            seen.set(v, view);
            return view;
        }
        if (globalThis.Blob && v instanceof globalThis.Blob) {
            var bytes = v._bytes.slice();
            var nb = (globalThis.File && v instanceof globalThis.File)
                ? new globalThis.File([], v.name, { type: v.type, lastModified: v.lastModified })
                : new globalThis.Blob([], { type: v.type });
            nb._bytes = bytes; nb.size = bytes.length;
            seen.set(v, nb);
            return nb;
        }
        if (v instanceof Map) {
            var m = new Map(); seen.set(v, m);
            v.forEach(function(val, key) { m.set(clone(key), clone(val)); });
            return m;
        }
        if (v instanceof Set) {
            var s = new Set(); seen.set(v, s);
            v.forEach(function(val) { s.add(clone(val)); });
            return s;
        }
        if (Array.isArray(v)) {
            var arr = []; seen.set(v, arr);
            for (var i = 0; i < v.length; i++) arr[i] = clone(v[i]);
            return arr;
        }
        var o = {}; seen.set(v, o);
        for (var k in v) {
            if (Object.prototype.hasOwnProperty.call(v, k)) o[k] = clone(v[k]);
        }
        return o;
    }
    return clone(value);
};
"#;
