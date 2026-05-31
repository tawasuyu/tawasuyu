pub(crate) const CSSOM_BOOTSTRAP: &str = r#"
// Fase 7.154 — CSS Object Model: el namespace `CSS` (`CSS.supports`/`CSS.escape`/
// `CSS.registerProperty` + factory numérico Typed OM `CSS.px`/`CSS.em`/...),
// `CSSStyleSheet` constructable (`new CSSStyleSheet()` + `replace`/`replaceSync`/
// `insertRule`/`deleteRule`/`cssRules`), `CSSRule`/`CSSStyleRule` mínimos y
// `document.adoptedStyleSheets`. Feature-detection (`CSS.supports('display','grid')`)
// y constructable stylesheets son patrones de UI moderna / web components.
// JS-puro: `CSS.supports` es heurístico (allowlist de propiedades conocidas, no
// hay motor CSS al que consultar); las hojas adoptadas no re-corren la cascada
// real (wiring al engine PENDIENTE).
(function() {
    if (globalThis.CSS != null) return;

    // ---------- CSS.escape (algoritmo CSSOM) ----------
    function cssEscape(value) {
        var string = String(value);
        var length = string.length;
        var index = -1;
        var codeUnit;
        var result = '';
        var firstCodeUnit = string.charCodeAt(0);
        while (++index < length) {
            codeUnit = string.charCodeAt(index);
            if (codeUnit === 0x0000) { result += '�'; continue; }
            if ((codeUnit >= 0x0001 && codeUnit <= 0x001F) || codeUnit === 0x007F ||
                (index === 0 && codeUnit >= 0x0030 && codeUnit <= 0x0039) ||
                (index === 1 && codeUnit >= 0x0030 && codeUnit <= 0x0039 && firstCodeUnit === 0x002D)) {
                result += '\\' + codeUnit.toString(16) + ' ';
                continue;
            }
            if (index === 0 && length === 1 && codeUnit === 0x002D) {
                result += '\\' + string.charAt(index);
                continue;
            }
            if (codeUnit >= 0x0080 || codeUnit === 0x002D || codeUnit === 0x005F ||
                (codeUnit >= 0x0030 && codeUnit <= 0x0039) ||
                (codeUnit >= 0x0041 && codeUnit <= 0x005A) ||
                (codeUnit >= 0x0061 && codeUnit <= 0x007A)) {
                result += string.charAt(index);
                continue;
            }
            result += '\\' + string.charAt(index);
        }
        return result;
    }

    // ---------- CSS.supports (heurístico) ----------
    // Propiedades que reportamos soportadas. No es la lista real del motor, pero
    // cubre el grueso de los feature-detects de SPAs (grid/flex/gap/sticky/...).
    var KNOWN_PROPS = {};
    ('display position float clear width height min-width min-height max-width max-height ' +
     'margin margin-top margin-right margin-bottom margin-left ' +
     'padding padding-top padding-right padding-bottom padding-left ' +
     'border border-width border-style border-color border-radius outline ' +
     'color background background-color background-image background-size background-position ' +
     'font font-family font-size font-weight font-style line-height letter-spacing text-align ' +
     'text-decoration text-transform white-space word-break overflow overflow-x overflow-y ' +
     'opacity visibility z-index cursor box-shadow box-sizing transition transform transform-origin ' +
     'animation flex flex-direction flex-wrap flex-grow flex-shrink flex-basis justify-content ' +
     'align-items align-content align-self order gap row-gap column-gap ' +
     'grid grid-template grid-template-columns grid-template-rows grid-column grid-row grid-area ' +
     'grid-gap inset top right bottom left content list-style list-style-type ' +
     'filter backdrop-filter clip-path mask aspect-ratio object-fit gap accent-color ' +
     'scroll-behavior overscroll-behavior pointer-events user-select resize').split(' ')
        .forEach(function(p) { KNOWN_PROPS[p] = true; });

    function trimLower(s) { return String(s == null ? '' : s).trim().toLowerCase(); }

    function supportsDecl(prop, value) {
        var p = trimLower(prop);
        if (!p || value == null || String(value).trim() === '') return false;
        // `var(--x)` siempre es válido sintácticamente.
        if (/^var\(/.test(String(value).trim())) return true;
        return KNOWN_PROPS[p] === true || p.charAt(0) === '-' /* vendor/custom */ ;
    }
    function supports(a, b) {
        if (arguments.length >= 2) return supportsDecl(a, b);
        // Forma de condición: "(prop: value)" o "(a) and (b)".
        var cond = String(a == null ? '' : a).trim();
        var m = cond.match(/^\(?\s*([-a-zA-Z]+)\s*:\s*([^)]+?)\s*\)?$/);
        if (m) return supportsDecl(m[1], m[2]);
        // selector(...) / combinaciones complejas: optimista.
        if (/^selector\(/i.test(cond)) return true;
        return false;
    }

    // ---------- CSS.registerProperty + propiedades registradas ----------
    var registered = {};
    function registerProperty(def) {
        def = def || {};
        if (typeof def.name !== 'string' || def.name.slice(0, 2) !== '--') {
            throw new SyntaxError('registerProperty: name debe empezar con --');
        }
        if (registered[def.name]) {
            throw new Error('registerProperty: "' + def.name + '" ya registrada');
        }
        registered[def.name] = {
            name: def.name,
            syntax: def.syntax != null ? String(def.syntax) : '*',
            inherits: !!def.inherits,
            initialValue: def.initialValue
        };
    }

    // ---------- Typed OM numérico (CSSUnitValue) ----------
    function CSSUnitValue(value, unit) { this.value = Number(value); this.unit = unit; }
    CSSUnitValue.prototype.toString = function() {
        if (this.unit === 'number') return String(this.value);
        if (this.unit === 'percent') return this.value + '%';
        return this.value + this.unit;
    };
    function CSSKeywordValue(value) { this.value = String(value); }
    CSSKeywordValue.prototype.toString = function() { return this.value; };

    var CSS = {
        escape: cssEscape,
        supports: supports,
        registerProperty: registerProperty,
        number: function(v) { return new CSSUnitValue(v, 'number'); },
        percent: function(v) { return new CSSUnitValue(v, 'percent'); }
    };
    // Unidades comunes como factories (CSS.px(10) → CSSUnitValue).
    ['px', 'em', 'rem', 'ex', 'ch', 'vw', 'vh', 'vmin', 'vmax', 'cm', 'mm', 'in', 'pt', 'pc', 'q',
     'deg', 'rad', 'grad', 'turn', 's', 'ms', 'fr', 'dpi', 'dpcm', 'dppx', 'Hz', 'kHz']
        .forEach(function(u) { CSS[u] = function(v) { return new CSSUnitValue(v, u); }; });

    globalThis.CSS = CSS;
    globalThis.CSSUnitValue = CSSUnitValue;
    globalThis.CSSKeywordValue = CSSKeywordValue;

    // ---------- CSSStyleDeclaration mínima (para reglas) ----------
    function parseDecls(body) {
        var decls = {};
        String(body || '').split(';').forEach(function(part) {
            var i = part.indexOf(':');
            if (i < 0) return;
            var prop = part.slice(0, i).trim().toLowerCase();
            var val = part.slice(i + 1).trim();
            if (prop) decls[prop] = val;
        });
        return decls;
    }
    function CSSStyleDeclaration(body) { this._decls = parseDecls(body); }
    CSSStyleDeclaration.prototype.getPropertyValue = function(p) {
        var v = this._decls[trimLower(p)]; return v == null ? '' : v;
    };
    CSSStyleDeclaration.prototype.setProperty = function(p, v) { this._decls[trimLower(p)] = String(v); };
    CSSStyleDeclaration.prototype.removeProperty = function(p) {
        p = trimLower(p); var old = this._decls[p] || ''; delete this._decls[p]; return old;
    };
    Object.defineProperty(CSSStyleDeclaration.prototype, 'length', {
        get: function() { return Object.keys(this._decls).length; }, configurable: true });
    Object.defineProperty(CSSStyleDeclaration.prototype, 'cssText', {
        get: function() {
            var self = this;
            return Object.keys(this._decls).map(function(k) { return k + ': ' + self._decls[k] + ';'; }).join(' ');
        },
        set: function(v) { this._decls = parseDecls(v); },
        configurable: true });

    // ---------- CSSRule / CSSStyleRule ----------
    function CSSRule() {}
    CSSRule.STYLE_RULE = 1; CSSRule.CHARSET_RULE = 2; CSSRule.IMPORT_RULE = 3;
    CSSRule.MEDIA_RULE = 4; CSSRule.FONT_FACE_RULE = 5; CSSRule.KEYFRAMES_RULE = 7;
    globalThis.CSSRule = CSSRule;

    function CSSStyleRule(cssText, sheet) {
        this.type = CSSRule.STYLE_RULE;
        this.parentStyleSheet = sheet || null;
        var open = cssText.indexOf('{');
        var close = cssText.lastIndexOf('}');
        this.selectorText = (open >= 0 ? cssText.slice(0, open) : cssText).trim();
        var body = (open >= 0 && close > open) ? cssText.slice(open + 1, close) : '';
        this.style = new CSSStyleDeclaration(body);
    }
    Object.defineProperty(CSSStyleRule.prototype, 'cssText', {
        get: function() { return this.selectorText + ' { ' + this.style.cssText + ' }'; },
        configurable: true });
    CSSStyleRule.prototype.constructor = CSSStyleRule;
    Object.setPrototypeOf(CSSStyleRule.prototype, CSSRule.prototype);
    globalThis.CSSStyleRule = CSSStyleRule;
    globalThis.CSSStyleDeclaration = globalThis.CSSStyleDeclaration || CSSStyleDeclaration;

    // ---------- CSSStyleSheet constructable ----------
    // Parte el texto CSS en reglas top-level por balance de llaves.
    function splitRules(text) {
        var rules = [];
        var depth = 0, start = 0;
        for (var i = 0; i < text.length; i++) {
            var ch = text[i];
            if (ch === '{') depth++;
            else if (ch === '}') {
                depth--;
                if (depth === 0) { rules.push(text.slice(start, i + 1).trim()); start = i + 1; }
            }
        }
        return rules.filter(function(r) { return r.length > 0; });
    }

    function CSSStyleSheet(options) {
        options = options || {};
        this.type = 'text/css';
        this.disabled = !!options.disabled;
        this.media = { mediaText: options.media != null ? String(options.media) : '' };
        this.title = options.title != null ? String(options.title) : null;
        this.ownerNode = null;
        this.ownerRule = null;
        this.parentStyleSheet = null;
        this.href = options.baseURL != null ? String(options.baseURL) : null;
        this._rules = [];
    }
    Object.defineProperty(CSSStyleSheet.prototype, 'cssRules', {
        get: function() { return this._rules.slice(); }, configurable: true });
    Object.defineProperty(CSSStyleSheet.prototype, 'rules', {
        get: function() { return this._rules.slice(); }, configurable: true });
    CSSStyleSheet.prototype.insertRule = function(rule, index) {
        if (index == null) index = 0;
        if (index < 0 || index > this._rules.length) throw new Error('IndexSizeError');
        this._rules.splice(index, 0, new CSSStyleRule(String(rule), this));
        return index;
    };
    CSSStyleSheet.prototype.deleteRule = function(index) {
        if (index < 0 || index >= this._rules.length) throw new Error('IndexSizeError');
        this._rules.splice(index, 1);
    };
    CSSStyleSheet.prototype.addRule = function(selector, style, index) {
        return this.insertRule(selector + ' { ' + (style || '') + ' }',
                               index == null ? this._rules.length : index);
    };
    CSSStyleSheet.prototype.replaceSync = function(text) {
        var self = this;
        this._rules = splitRules(String(text)).map(function(r) { return new CSSStyleRule(r, self); });
    };
    CSSStyleSheet.prototype.replace = function(text) {
        var self = this;
        return new Promise(function(resolve) { self.replaceSync(text); resolve(self); });
    };
    globalThis.CSSStyleSheet = CSSStyleSheet;

    // ---------- document.adoptedStyleSheets ----------
    var doc = globalThis.document = globalThis.document || {};
    if (!('adoptedStyleSheets' in doc)) {
        var adopted = [];
        Object.defineProperty(doc, 'adoptedStyleSheets', {
            get: function() { return adopted; },
            set: function(v) { adopted = Array.prototype.slice.call(v || []); },
            configurable: true
        });
    }
    void 0;
})();
"#;
