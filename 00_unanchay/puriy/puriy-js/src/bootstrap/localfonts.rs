pub(crate) const LOCALFONTS_BOOTSTRAP: &str = r#"
// Fase 7.163 — Local Font Access API (`window.queryLocalFonts()` + `FontData`).
// Enumera las fuentes instaladas en el sistema (editores de diseño, apps tipo
// Figma). Cuelga de `Blob` (Fase 7.x) + `DOMException`. Host-driven: el listado
// real lo pone el chrome con `__puriy_set_local_fonts([...])`; por defecto hay un
// set sintético mínimo para feature-detection. `FontData.blob()` devuelve los
// bytes del fichero de fuente (sintético/vacío hasta que el chrome lo cablee —
// PENDIENTE). Gated-by-permission (`local-fonts`): por defecto concedido, el
// chrome lo flippea con `__puriy_set_local_fonts_permission`.
(function() {
    if (typeof globalThis.queryLocalFonts === 'function') return;
    if (globalThis.__puriy_local_fonts_permission == null) {
        globalThis.__puriy_local_fonts_permission = true;
    }
    var DEFAULTS = [
        { postscriptName: 'ArialMT', fullName: 'Arial', family: 'Arial', style: 'Regular' },
        { postscriptName: 'TimesNewRomanPSMT', fullName: 'Times New Roman', family: 'Times New Roman', style: 'Regular' },
        { postscriptName: 'Courier', fullName: 'Courier', family: 'Courier', style: 'Regular' }
    ];

    function FontData(info) {
        info = info || {};
        this.postscriptName = String(info.postscriptName != null ? info.postscriptName : '');
        this.fullName = String(info.fullName != null ? info.fullName : '');
        this.family = String(info.family != null ? info.family : '');
        this.style = String(info.style != null ? info.style : 'Regular');
        this._bytes = info.bytes || null;
    }
    FontData.prototype.blob = function() {
        var bytes = this._bytes ? [this._bytes] : [];
        return Promise.resolve(new globalThis.Blob(bytes, { type: 'font/opentype' }));
    };
    globalThis.FontData = FontData;

    globalThis.queryLocalFonts = function(options) {
        if (!globalThis.__puriy_local_fonts_permission) {
            return Promise.reject(new globalThis.DOMException('Permission denied', 'SecurityError'));
        }
        var raw = globalThis.__puriy_local_fonts_list;
        var src = (Array.isArray(raw) && raw.length) ? raw : DEFAULTS;
        var fonts = src.map(function(i) { return new FontData(i); });
        // Filtro opcional por postscriptNames.
        if (options && Array.isArray(options.postscriptNames)) {
            var want = options.postscriptNames;
            fonts = fonts.filter(function(f) { return want.indexOf(f.postscriptName) >= 0; });
        }
        return Promise.resolve(fonts);
    };
    if (typeof globalThis.window === 'object' && globalThis.window) {
        globalThis.window.queryLocalFonts = globalThis.queryLocalFonts;
    }

    globalThis.__puriy_set_local_fonts = function(list) {
        globalThis.__puriy_local_fonts_list = Array.isArray(list) ? list.slice() : [];
        return true;
    };
    globalThis.__puriy_set_local_fonts_permission = function(ok) {
        globalThis.__puriy_local_fonts_permission = !!ok;
        return true;
    };
    void 0;
})();
"#;
