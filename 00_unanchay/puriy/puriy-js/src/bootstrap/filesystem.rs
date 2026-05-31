pub(crate) const FILESYSTEM_BOOTSTRAP: &str = r#"
// Fase 7.126 — File System Access API (`window.showOpenFilePicker` /
// `showSaveFilePicker` / `showDirectoryPicker`). Editores de texto/imagen y herramientas
// de respaldo la usan para leer y escribir archivos reales del usuario tras un gesto. El
// motor no tiene acceso al FS del host: los pickers son host-driven (mismo molde que la
// familia device-access 7.120-7.125). Cada picker publica una mutación y devuelve una
// Promise que el chrome resuelve con `__puriy_fs_open_resolve(id, list)` /
// `__puriy_fs_save_resolve(id, info)` / `__puriy_fs_directory_resolve(id, info)`, o cancela
// con `__puriy_fs_reject(id)` (`AbortError`, sin selección). Los handles modelan el árbol:
// `FileSystemFileHandle.getFile()` devuelve un `File` (Fase del Blob/File), `createWritable()`
// un stream cuyo `write()`/`close()` publican `fs-write`/`fs-close` y bufferizan el contenido
// para round-trip. `FileSystemDirectoryHandle.getFileHandle/getDirectoryHandle({create})`
// recorren/crean hijos en memoria.
(function() {
    if (globalThis.showOpenFilePicker != null) return;

    globalThis.__puriy_fs_pending = globalThis.__puriy_fs_pending || {};
    globalThis.__puriy_fs_next_id = globalThis.__puriy_fs_next_id || 1;

    function FileSystemHandle(kind, name) {
        this.kind = kind;
        this.name = String(name != null ? name : '');
    }
    FileSystemHandle.prototype.isSameEntry = function(other) {
        return Promise.resolve(other === this);
    };
    FileSystemHandle.prototype.queryPermission = function() { return Promise.resolve('granted'); };
    FileSystemHandle.prototype.requestPermission = function() { return Promise.resolve('granted'); };

    function FileSystemWritableFileStream(handle) {
        this._handle = handle;
        this._buf = '';
    }
    FileSystemWritableFileStream.prototype.write = function(data) {
        var chunk = data;
        if (data != null && typeof data === 'object' && data.type === 'write') chunk = data.data;
        this._buf += (chunk != null ? String(chunk) : '');
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'fs-write', value: this._handle.name
        });
        return Promise.resolve();
    };
    FileSystemWritableFileStream.prototype.seek = function(pos) { return Promise.resolve(); };
    FileSystemWritableFileStream.prototype.truncate = function(size) {
        this._buf = this._buf.slice(0, size | 0);
        return Promise.resolve();
    };
    FileSystemWritableFileStream.prototype.close = function() {
        this._handle._content = this._buf;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'fs-close', value: this._handle.name
        });
        return Promise.resolve();
    };

    function FileSystemFileHandle(info) {
        info = info || {};
        FileSystemHandle.call(this, 'file', info.name || 'archivo');
        this._content = (info.content != null) ? String(info.content) : '';
        this._type = info.type || '';
    }
    FileSystemFileHandle.prototype = Object.create(FileSystemHandle.prototype);
    FileSystemFileHandle.prototype.constructor = FileSystemFileHandle;
    FileSystemFileHandle.prototype.getFile = function() {
        var self = this;
        var file = (typeof globalThis.File === 'function')
            ? new globalThis.File([self._content], self.name, { type: self._type })
            : { name: self.name, size: self._content.length, type: self._type };
        return Promise.resolve(file);
    };
    FileSystemFileHandle.prototype.createWritable = function(options) {
        return Promise.resolve(new FileSystemWritableFileStream(this));
    };

    function FileSystemDirectoryHandle(info) {
        info = info || {};
        FileSystemHandle.call(this, 'directory', info.name || 'directorio');
        this._children = {};
    }
    FileSystemDirectoryHandle.prototype = Object.create(FileSystemHandle.prototype);
    FileSystemDirectoryHandle.prototype.constructor = FileSystemDirectoryHandle;
    FileSystemDirectoryHandle.prototype.getFileHandle = function(name, options) {
        var key = String(name);
        var existing = this._children[key];
        if (existing && existing.kind === 'file') return Promise.resolve(existing);
        if (options && options.create) {
            var h = new FileSystemFileHandle({ name: key });
            this._children[key] = h;
            return Promise.resolve(h);
        }
        return Promise.reject(new globalThis.DOMException(
            'no existe el archivo ' + key, 'NotFoundError'));
    };
    FileSystemDirectoryHandle.prototype.getDirectoryHandle = function(name, options) {
        var key = String(name);
        var existing = this._children[key];
        if (existing && existing.kind === 'directory') return Promise.resolve(existing);
        if (options && options.create) {
            var d = new FileSystemDirectoryHandle({ name: key });
            this._children[key] = d;
            return Promise.resolve(d);
        }
        return Promise.reject(new globalThis.DOMException(
            'no existe el directorio ' + key, 'NotFoundError'));
    };
    FileSystemDirectoryHandle.prototype.removeEntry = function(name, options) {
        delete this._children[String(name)];
        return Promise.resolve();
    };
    // Nota: el spec usa async iterators (entries/values/keys); acá keys() devuelve
    // un Promise<Array> de nombres como atajo (cubre el caso común de listar).
    FileSystemDirectoryHandle.prototype.keys = function() {
        return Promise.resolve(Object.keys(this._children));
    };

    globalThis.FileSystemHandle = FileSystemHandle;
    globalThis.FileSystemFileHandle = FileSystemFileHandle;
    globalThis.FileSystemDirectoryHandle = FileSystemDirectoryHandle;
    globalThis.FileSystemWritableFileStream = FileSystemWritableFileStream;

    function pedir(op) {
        return new Promise(function(resolve, reject) {
            var id = globalThis.__puriy_fs_next_id++;
            globalThis.__puriy_fs_pending[id] = { resolve: resolve, reject: reject, op: op };
            globalThis.__puriy_dirty.push({ id: '__window__', kind: op, value: String(id) });
        });
    }
    globalThis.showOpenFilePicker = function(options) { return pedir('fs-open-picker'); };
    globalThis.showSaveFilePicker = function(options) { return pedir('fs-save-picker'); };
    globalThis.showDirectoryPicker = function(options) { return pedir('fs-directory-picker'); };

    // El chrome entrega los archivos elegidos (open siempre devuelve un Array).
    globalThis.__puriy_fs_open_resolve = function(id, list) {
        var p = globalThis.__puriy_fs_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_fs_pending[id];
        var out = [];
        list = list || [];
        for (var i = 0; i < list.length; i++) out.push(new FileSystemFileHandle(list[i]));
        p.resolve(out);
        return true;
    };
    globalThis.__puriy_fs_save_resolve = function(id, info) {
        var p = globalThis.__puriy_fs_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_fs_pending[id];
        p.resolve(new FileSystemFileHandle(info || {}));
        return true;
    };
    globalThis.__puriy_fs_directory_resolve = function(id, info) {
        var p = globalThis.__puriy_fs_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_fs_pending[id];
        p.resolve(new FileSystemDirectoryHandle(info || {}));
        return true;
    };
    // Cancela el picker (el usuario cerró el diálogo sin elegir).
    globalThis.__puriy_fs_reject = function(id, name, message) {
        var p = globalThis.__puriy_fs_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_fs_pending[id];
        p.reject(new globalThis.DOMException(
            (message != null) ? String(message) : 'picker cancelado',
            (name != null) ? String(name) : 'AbortError'));
        return true;
    };
    void 0;
})();
"#;
