pub(crate) const FONTFACE_BOOTSTRAP: &str = r#"
// Fase 7.152 — CSS Font Loading API (`FontFace` + `FontFaceSet` expuesto como
// `document.fonts`). Permite a una app cargar fuentes dinámicamente y esperar a que estén
// listas antes de pintar. La máquina de estados (`unloaded`/`loading`/`loaded`/`error`),
// el set iterable y los eventos (`loading`/`loadingdone`/`loadingerror`) son JS-puros; la
// descarga/decode real de la fuente es del chrome (wiring PENDIENTE):
//   · `face.load()` publica `kind: 'fontface-load'` y resuelve por microtask con la propia
//     face (carga optimista); el host puede forzar el resultado real con
//     `__puriy_fontface_loaded(id)` / `__puriy_fontface_error(id, msg)`.
//   · `document.fonts.ready` resuelve cuando no quedan fuentes en `loading`.
(function() {
    if (globalThis.FontFace != null) return;
    var nextId = 1;
    var faces = {};

    function FontFace(family, source, descriptors) {
        descriptors = descriptors || {};
        this._id = nextId++;
        faces[this._id] = this;
        this.family = family != null ? String(family) : '';
        this._source = source;
        this.style = descriptors.style != null ? descriptors.style : 'normal';
        this.weight = descriptors.weight != null ? descriptors.weight : 'normal';
        this.stretch = descriptors.stretch != null ? descriptors.stretch : 'normal';
        this.unicodeRange = descriptors.unicodeRange != null ? descriptors.unicodeRange : 'U+0-10FFFF';
        this.featureSettings = descriptors.featureSettings != null ? descriptors.featureSettings : 'normal';
        this.variationSettings = descriptors.variationSettings != null ? descriptors.variationSettings : 'normal';
        this.variant = descriptors.variant != null ? descriptors.variant : 'normal';
        this.display = descriptors.display != null ? descriptors.display : 'auto';
        this.ascentOverride = descriptors.ascentOverride != null ? descriptors.ascentOverride : 'normal';
        this.descentOverride = descriptors.descentOverride != null ? descriptors.descentOverride : 'normal';
        this.lineGapOverride = descriptors.lineGapOverride != null ? descriptors.lineGapOverride : 'normal';
        this.status = 'unloaded';
        var self = this;
        this._resolve = null; this._reject = null;
        this.loaded = new Promise(function(res, rej) { self._resolve = res; self._reject = rej; });
        // Evita rechazo no manejado si la face nunca se carga.
        this.loaded.catch(function() {});
    }

    FontFace.prototype._settle = function(ok, msg) {
        if (this.status === 'loaded' || this.status === 'error') return;
        if (ok) {
            this.status = 'loaded';
            this._resolve(this);
            if (globalThis.document && globalThis.document.fonts) globalThis.document.fonts._onFaceLoaded(this);
        } else {
            this.status = 'error';
            var err = new globalThis.DOMException(msg != null ? String(msg) : 'fallo al cargar la fuente', 'NetworkError');
            this._reject(err);
            if (globalThis.document && globalThis.document.fonts) globalThis.document.fonts._onFaceError(this);
        }
    };

    FontFace.prototype.load = function() {
        if (this.status === 'loading' || this.status === 'loaded') return this.loaded;
        this.status = 'loading';
        var self = this;
        if (globalThis.document && globalThis.document.fonts) globalThis.document.fonts._onFaceLoading(this);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'fontface-load',
            value: String(this._id) + String.fromCharCode(0x1D) + this.family });
        // Carga optimista por microtask (el host puede ganar la carrera y forzar el real).
        Promise.resolve().then(function() { self._settle(true); });
        return this.loaded;
    };

    globalThis.FontFace = FontFace;

    // El host fuerza el resultado real de una carga.
    globalThis.__puriy_fontface_loaded = function(id) { var f = faces[id]; if (f) { f._settle(true); return true; } return false; };
    globalThis.__puriy_fontface_error = function(id, msg) { var f = faces[id]; if (f) { f._settle(false, msg); return true; } return false; };

    // ---- FontFaceSet (document.fonts) ----
    function FontFaceSet() {
        globalThis.EventTarget.call(this);
        this._faces = [];
        this._loadingCount = 0;
        this.status = 'loaded';
        this.onloading = null; this.onloadingdone = null; this.onloadingerror = null;
        var self = this;
        this._readyResolve = null;
        this.ready = Promise.resolve(this);
        this._resetReady = function() {
            self.ready = new Promise(function(res) { self._readyResolve = res; });
        };
    }
    FontFaceSet.prototype = Object.create(globalThis.EventTarget.prototype);
    FontFaceSet.prototype.constructor = FontFaceSet;

    function emit(self, type, faces) {
        var ev = { type: type, fontfaces: faces || [] };
        var h = self['on' + type];
        if (typeof h === 'function') { try { h.call(self, ev); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; } }
        self.dispatchEvent(ev);
    }

    FontFaceSet.prototype.add = function(face) {
        if (this._faces.indexOf(face) === -1) this._faces.push(face);
        return this;
    };
    FontFaceSet.prototype.delete = function(face) {
        var i = this._faces.indexOf(face);
        if (i === -1) return false;
        this._faces.splice(i, 1);
        return true;
    };
    FontFaceSet.prototype.clear = function() { this._faces = []; };
    FontFaceSet.prototype.has = function(face) { return this._faces.indexOf(face) !== -1; };
    FontFaceSet.prototype.forEach = function(cb, thisArg) {
        for (var i = 0; i < this._faces.length; i++) cb.call(thisArg, this._faces[i], this._faces[i], this);
    };
    FontFaceSet.prototype.values = function() { return this._faces.slice()[Symbol.iterator](); };
    FontFaceSet.prototype.keys = function() { return this._faces.slice()[Symbol.iterator](); };
    FontFaceSet.prototype.entries = function() {
        return this._faces.map(function(f) { return [f, f]; })[Symbol.iterator]();
    };
    FontFaceSet.prototype[Symbol.iterator] = function() { return this._faces.slice()[Symbol.iterator](); };
    Object.defineProperty(FontFaceSet.prototype, 'size', {
        get: function() { return this._faces.length; }, enumerable: true, configurable: true
    });

    FontFaceSet.prototype.check = function(font, text) {
        // Soportada si alguna face cargada coincide con la family pedida, o si es genérica.
        var fam = parseFontFamily(font);
        if (fam == null) return true;
        for (var i = 0; i < this._faces.length; i++) {
            if (this._faces[i].family === fam && this._faces[i].status === 'loaded') return true;
        }
        return /^(serif|sans-serif|monospace|cursive|fantasy|system-ui)$/i.test(fam);
    };
    FontFaceSet.prototype.load = function(font, text) {
        var fam = parseFontFamily(font);
        var matched = this._faces.filter(function(f) { return fam == null || f.family === fam; });
        return Promise.all(matched.map(function(f) { return f.load(); }));
    };

    function parseFontFamily(font) {
        if (font == null) return null;
        var m = /(?:\d+(?:px|pt|em|%)\s+)?["']?([^"',]+)["']?\s*$/.exec(String(font).trim());
        return m ? m[1].trim() : null;
    }

    FontFaceSet.prototype._onFaceLoading = function(face) {
        if (this._faces.indexOf(face) === -1) this._faces.push(face);
        if (this._loadingCount === 0) { this.status = 'loading'; this._resetReady(); }
        this._loadingCount++;
        emit(this, 'loading', [face]);
    };
    FontFaceSet.prototype._finishOne = function() {
        if (this._loadingCount > 0) this._loadingCount--;
        if (this._loadingCount === 0) {
            this.status = 'loaded';
            if (this._readyResolve) { this._readyResolve(this); this._readyResolve = null; }
        }
    };
    FontFaceSet.prototype._onFaceLoaded = function(face) {
        emit(this, 'loadingdone', [face]);
        this._finishOne();
    };
    FontFaceSet.prototype._onFaceError = function(face) {
        emit(this, 'loadingerror', [face]);
        this._finishOne();
    };

    globalThis.FontFaceSet = FontFaceSet;
    var doc = globalThis.document = globalThis.document || {};
    if (doc.fonts == null) doc.fonts = new FontFaceSet();
    void 0;
})();
"#;
