pub(crate) const FILE_BOOTSTRAP: &str = r#"
// Fase 7.60 — File (subclase de Blob). El spec dice que FormData, al recibir
// un Blob con filename, lo envuelve en un File; y muchos sitios chequean
// `value instanceof File` tras leer un <input type=file>. Acá File hereda
// toda la maquinaria de bytes de Blob (text/arrayBuffer/slice/stream) y sólo
// agrega `name` + `lastModified` + `webkitRelativePath`.
//
// Divergencia documentada: `lastModified` por defecto es 0 (el spec usa
// Date.now()) — preferimos un default determinístico para testabilidad; el
// user puede pasarlo explícito en options. `webkitRelativePath` siempre ''.
globalThis.File = function(bits, name, options) {
    // Reusa el constructor de Blob para poblar _bytes / size / type.
    globalThis.Blob.call(this, bits || [], options);
    this.name = String(name);
    options = options || {};
    this.lastModified = (options.lastModified != null) ? Number(options.lastModified) : 0;
    this.webkitRelativePath = '';
};
globalThis.File.prototype = Object.create(globalThis.Blob.prototype);
globalThis.File.prototype.constructor = globalThis.File;
// Helper compartido por FormData: normaliza un value de entry. Un Blob/File
// con filename (o un Blob suelto, que el spec nombra 'blob') se envuelve en
// File; un File sin filename nuevo se conserva tal cual; cualquier otra cosa
// cae a String(value). Devuelve `{ value, filename }` — filename queda
// poblado sólo para entries de File (para la región multipart de body.rs).
globalThis.__puriy_fd_normalize = function(value, filename) {
    if (globalThis.Blob && value instanceof globalThis.Blob) {
        var esFile = globalThis.File && value instanceof globalThis.File;
        if (esFile && filename == null) {
            return { value: value, filename: value.name };
        }
        var fname = (filename != null) ? String(filename) : (esFile ? value.name : 'blob');
        var f = new globalThis.File([value], fname, { type: value.type });
        return { value: f, filename: fname };
    }
    return { value: String(value), filename: (filename != null ? String(filename) : undefined) };
};
"#;
