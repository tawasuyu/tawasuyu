pub(crate) const CANVAS2D_BOOTSTRAP: &str = r#"
// Fase 7.150 — Canvas 2D (`OffscreenCanvas` + `CanvasRenderingContext2D` + `Path2D`
// + `ImageData` + `ImageBitmap`). El contexto es JS-puro funcional: registra una
// lista de comandos de dibujo en `ctx._cmds` (y se publica bajo
// `__puriy_canvas2d_contexts[id]`) que el chrome drena para pintar con vello — el
// raster real es del chrome (PENDIENTE). El estado del contexto (fillStyle, transform,
// path actual, save/restore) es 100% funcional y observable desde JS.
//   · `new OffscreenCanvas(w, h).getContext('2d')` es el punto de entrada constructible
//     sin DOM; el chrome cablea `HTMLCanvasElement.getContext` con el mismo molde.
//   · `getImageData` devuelve un `ImageData` de ceros (el host lo rellena con el
//     framebuffer real); `measureText` da métricas sintéticas proporcionales al font.
//   · `convertToBlob()` / `transferToImageBitmap()` / `createImageBitmap()` resuelven
//     con objetos sintéticos; el host reemplaza los bytes reales.
(function() {
    if (globalThis.CanvasRenderingContext2D != null) return;
    var ctxRegistry = globalThis.__puriy_canvas2d_contexts = globalThis.__puriy_canvas2d_contexts || {};
    var nextId = 1;

    function clamp255(v) { v = v | 0; return v < 0 ? 0 : (v > 255 ? 255 : v); }

    // ---- ImageData ----
    function ImageData(a, b, c) {
        var w, h, data;
        if (a != null && (a instanceof Uint8ClampedArray || (a.buffer instanceof ArrayBuffer))) {
            data = a instanceof Uint8ClampedArray ? a : new Uint8ClampedArray(a.buffer);
            w = b | 0;
            h = c != null ? (c | 0) : (w > 0 ? (data.length / 4 / w) | 0 : 0);
        } else {
            w = a | 0; h = b | 0;
            data = new Uint8ClampedArray(Math.max(0, w * h * 4));
        }
        if (w <= 0 || h <= 0) throw new globalThis.DOMException('dimensiones de ImageData inválidas', 'IndexSizeError');
        this.width = w; this.height = h; this.data = data;
        this.colorSpace = 'srgb';
    }
    globalThis.ImageData = ImageData;

    // ---- ImageBitmap ----
    function ImageBitmap(w, h) {
        this.width = w | 0; this.height = h | 0; this._closed = false;
    }
    ImageBitmap.prototype.close = function() { this._closed = true; this.width = 0; this.height = 0; };
    globalThis.ImageBitmap = ImageBitmap;

    function bitmapDims(source) {
        if (source == null) return [0, 0];
        var w = source.width || source.videoWidth || source.naturalWidth || source.codedWidth || 0;
        var h = source.height || source.videoHeight || source.naturalHeight || source.codedHeight || 0;
        return [w | 0, h | 0];
    }
    globalThis.createImageBitmap = function(source) {
        var d = bitmapDims(source);
        return Promise.resolve(new ImageBitmap(d[0], d[1]));
    };

    // ---- Image (HTMLImageElement constructible, detached) ----
    // Fase 7.197b — `new Image(); img.src = url; img.onload = …`. El chrome
    // decodifica la URL al pintar el canvas; disparamos `load` de forma
    // asíncrona (optimista) para que el patrón `onload → drawImage` corra.
    if (globalThis.Image == null) {
        function Image(w, h) {
            this._attrs = {};
            this.width = w | 0; this.height = h | 0;
            this.naturalWidth = 0; this.naturalHeight = 0;
            this.complete = false;
            this.onload = null; this.onerror = null;
            this._listeners = {};
        }
        Object.defineProperty(Image.prototype, 'src', {
            get: function() { return this._attrs.src || ''; },
            set: function(v) {
                this._attrs.src = String(v);
                this.complete = true;
                var self = this;
                var fire = function() {
                    var ev = { type: 'load', target: self };
                    if (typeof self.onload === 'function') self.onload(ev);
                    var ls = (self._listeners['load'] || []).slice();
                    for (var i = 0; i < ls.length; i++) ls[i].call(self, ev);
                };
                if (typeof queueMicrotask === 'function') queueMicrotask(fire);
                else if (typeof setTimeout === 'function') setTimeout(fire, 0);
                else fire();
            },
            enumerable: true, configurable: true
        });
        Object.defineProperty(Image.prototype, 'currentSrc', {
            get: function() { return this._attrs.src || ''; },
            enumerable: true, configurable: true
        });
        Image.prototype.addEventListener = function(type, cb) {
            (this._listeners[type] = this._listeners[type] || []).push(cb);
        };
        Image.prototype.removeEventListener = function(type, cb) {
            var ls = this._listeners[type]; if (!ls) return;
            var i = ls.indexOf(cb); if (i >= 0) ls.splice(i, 1);
        };
        Image.prototype.setAttribute = function(k, v) { if (k === 'src') this.src = v; };
        Image.prototype.getAttribute = function(k) { return k === 'src' ? this.src : null; };
        globalThis.Image = Image;
        if (globalThis.HTMLImageElement == null) globalThis.HTMLImageElement = Image;
    }

    // ---- Path2D ----
    function Path2D(arg) {
        this._cmds = [];
        if (arg instanceof Path2D) { this._cmds = arg._cmds.slice(); }
        else if (typeof arg === 'string') { this._cmds.push(['svg', arg]); }
    }
    var pathOps = ['moveTo', 'lineTo', 'bezierCurveTo', 'quadraticCurveTo', 'arc', 'arcTo',
                   'ellipse', 'rect', 'roundRect', 'closePath'];
    pathOps.forEach(function(op) {
        Path2D.prototype[op] = function() {
            this._cmds.push([op].concat(Array.prototype.slice.call(arguments)));
        };
    });
    Path2D.prototype.addPath = function(path) {
        if (path && path._cmds) this._cmds = this._cmds.concat(path._cmds);
    };
    globalThis.Path2D = Path2D;

    // ---- CanvasGradient / CanvasPattern ----
    function CanvasGradient(kind, coords) { this._kind = kind; this._coords = coords; this._stops = []; }
    CanvasGradient.prototype.addColorStop = function(offset, color) {
        this._stops.push([+offset, String(color)]);
    };
    globalThis.CanvasGradient = CanvasGradient;

    function CanvasPattern(image, repetition) { this._image = image; this._repetition = repetition || 'repeat'; this._transform = null; }
    CanvasPattern.prototype.setTransform = function(m) { this._transform = m || null; };
    globalThis.CanvasPattern = CanvasPattern;

    // Fase 7.198 — normaliza un valor de fillStyle/strokeStyle para el snapshot:
    // un CanvasPattern (objeto con _image/_repetition) se serializa a un
    // descriptor liviano {_pattern, src, rep} que el chrome decodifica vía el
    // mismo pipeline de imágenes que `drawImage`; un string (color) o un
    // CanvasGradient ({_kind/_coords/_stops}) pasan tal cual (ya serializables).
    function serStyle(v) {
        if (v && typeof v === 'object' && v._image !== undefined && v._repetition !== undefined) {
            var im = v._image;
            var src = (im && (im.src || im.currentSrc || im._src)) || '';
            return { _pattern: true, src: String(src), rep: v._repetition };
        }
        return v;
    }

    function TextMetrics(text, fontPx) {
        var w = String(text).length * fontPx * 0.5;
        this.width = w;
        this.actualBoundingBoxLeft = 0;
        this.actualBoundingBoxRight = w;
        this.actualBoundingBoxAscent = fontPx * 0.8;
        this.actualBoundingBoxDescent = fontPx * 0.2;
        this.fontBoundingBoxAscent = fontPx * 0.9;
        this.fontBoundingBoxDescent = fontPx * 0.25;
        this.emHeightAscent = fontPx * 0.8;
        this.emHeightDescent = fontPx * 0.2;
        this.hangingBaseline = fontPx * 0.8;
        this.alphabeticBaseline = 0;
        this.ideographicBaseline = fontPx * -0.2;
    }

    function parseFontPx(font) {
        var m = /(\d+(?:\.\d+)?)px/.exec(String(font));
        return m ? +m[1] : 10;
    }

    // ---- CanvasRenderingContext2D ----
    function defaultState() {
        return {
            fillStyle: '#000000', strokeStyle: '#000000', lineWidth: 1,
            lineCap: 'butt', lineJoin: 'miter', miterLimit: 10,
            lineDashOffset: 0, lineDash: [],
            font: '10px sans-serif', textAlign: 'start', textBaseline: 'alphabetic',
            direction: 'inherit', letterSpacing: '0px', wordSpacing: '0px', fontKerning: 'auto',
            globalAlpha: 1, globalCompositeOperation: 'source-over',
            shadowColor: 'rgba(0, 0, 0, 0)', shadowBlur: 0, shadowOffsetX: 0, shadowOffsetY: 0,
            imageSmoothingEnabled: true, imageSmoothingQuality: 'low', filter: 'none',
            transform: [1, 0, 0, 1, 0, 0]
        };
    }

    function CanvasRenderingContext2D(canvas) {
        this.canvas = canvas || null;
        this._id = nextId++;
        ctxRegistry[this._id] = this;
        this._cmds = [];
        this._state = defaultState();
        this._stack = [];
        this._path = new Path2D();
    }
    var C = CanvasRenderingContext2D.prototype;

    // Propiedades de estado como accessors que delegan al _state activo.
    var stateProps = ['fillStyle', 'strokeStyle', 'lineWidth', 'lineCap', 'lineJoin',
        'miterLimit', 'lineDashOffset', 'font', 'textAlign', 'textBaseline', 'direction',
        'letterSpacing', 'wordSpacing', 'fontKerning', 'globalAlpha', 'globalCompositeOperation',
        'shadowColor', 'shadowBlur', 'shadowOffsetX', 'shadowOffsetY', 'imageSmoothingEnabled',
        'imageSmoothingQuality', 'filter'];
    stateProps.forEach(function(p) {
        Object.defineProperty(C, p, {
            get: function() { return this._state[p]; },
            set: function(v) { this._state[p] = v; },
            enumerable: true, configurable: true
        });
    });

    C._rec = function() { this._cmds.push(Array.prototype.slice.call(arguments)); };

    // Fase 7.196 — snapshot del estilo activo, apendido a los comandos que
    // pintan (fill/stroke/text/rect) para que el chrome sepa con qué color/
    // ancho/alpha pintar sin re-derivar el estado (los setters de fillStyle
    // etc. NO registran comandos). fillStyle/strokeStyle pueden ser un string
    // (color CSS) o un objeto CanvasGradient (con _kind/_coords/_stops).
    C._snapshot = function() {
        var st = this._state;
        var snap = {
            f: serStyle(st.fillStyle), s: serStyle(st.strokeStyle), lw: st.lineWidth, ga: st.globalAlpha,
            fnt: st.font, lc: st.lineCap, lj: st.lineJoin, ta: st.textAlign, tb: st.textBaseline,
            ld: st.lineDash, ldo: st.lineDashOffset
        };
        // Fase 7.199 — sombra: sólo se apende si está activa (blur o offset),
        // para no engordar cada snapshot con el default transparente. El chrome
        // valida además que el color de sombra no sea totalmente transparente.
        var sb = st.shadowBlur || 0, sox = st.shadowOffsetX || 0, soy = st.shadowOffsetY || 0;
        if (st.shadowColor && (sb > 0 || sox !== 0 || soy !== 0)) {
            snap.sc = st.shadowColor; snap.sb = sb; snap.sox = sox; snap.soy = soy;
        }
        return snap;
    };

    C.save = function() {
        var s = {}; for (var k in this._state) s[k] = this._state[k];
        s.transform = this._state.transform.slice();
        s.lineDash = this._state.lineDash.slice();
        this._stack.push(s);
        this._rec('save');
    };
    C.restore = function() {
        if (this._stack.length > 0) { this._state = this._stack.pop(); this._rec('restore'); }
    };
    C.reset = function() {
        this._cmds = []; this._state = defaultState(); this._stack = []; this._path = new Path2D();
        this._rec('reset');
    };

    // Transformaciones.
    function mul(m, n) {
        return [
            m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1],
            m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3],
            m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5]
        ];
    }
    C.scale = function(x, y) { this._state.transform = mul(this._state.transform, [x, 0, 0, y, 0, 0]); this._rec('scale', x, y); };
    C.rotate = function(a) {
        var c = Math.cos(a), s = Math.sin(a);
        this._state.transform = mul(this._state.transform, [c, s, -s, c, 0, 0]);
        this._rec('rotate', a);
    };
    C.translate = function(x, y) { this._state.transform = mul(this._state.transform, [1, 0, 0, 1, x, y]); this._rec('translate', x, y); };
    C.transform = function(a, b, c, d, e, f) { this._state.transform = mul(this._state.transform, [a, b, c, d, e, f]); this._rec('transform', a, b, c, d, e, f); };
    C.setTransform = function(a, b, c, d, e, f) {
        if (a != null && typeof a === 'object') { var m = a; this._state.transform = [m.a, m.b, m.c, m.d, m.e, m.f]; }
        else { this._state.transform = [a || 0, b || 0, c || 0, d || 0, e || 0, f || 0]; }
        // Fase 7.196 — registramos la matriz RESUELTA (no los args crudos, que
        // pueden venir como DOMMatrix) para que el chrome la reaplique tal cual.
        var t = this._state.transform;
        this._rec('setTransform', t[0], t[1], t[2], t[3], t[4], t[5]);
    };
    C.resetTransform = function() { this._state.transform = [1, 0, 0, 1, 0, 0]; this._rec('resetTransform'); };
    C.getTransform = function() {
        var t = this._state.transform;
        if (globalThis.DOMMatrix) return new globalThis.DOMMatrix([t[0], t[1], t[2], t[3], t[4], t[5]]);
        return { a: t[0], b: t[1], c: t[2], d: t[3], e: t[4], f: t[5],
                 m11: t[0], m12: t[1], m21: t[2], m22: t[3], m41: t[4], m42: t[5],
                 is2D: true, isIdentity: t[0] === 1 && t[1] === 0 && t[2] === 0 && t[3] === 1 && t[4] === 0 && t[5] === 0 };
    };

    // Path: delega al path actual y registra.
    var pathDelegates = ['moveTo', 'lineTo', 'bezierCurveTo', 'quadraticCurveTo', 'arc', 'arcTo', 'ellipse', 'roundRect'];
    pathDelegates.forEach(function(op) {
        C[op] = function() { var args = Array.prototype.slice.call(arguments); this._path[op].apply(this._path, args); this._rec.apply(this, [op].concat(args)); };
    });
    C.beginPath = function() { this._path = new Path2D(); this._rec('beginPath'); };
    C.closePath = function() { this._path.closePath(); this._rec('closePath'); };
    C.rect = function(x, y, w, h) { this._path.rect(x, y, w, h); this._rec('rect', x, y, w, h); };

    function pathArg(self, a) { return (a instanceof Path2D) ? a : self._path; }
    C.fill = function(a) { this._rec('fill', this._snapshot()); };
    C.stroke = function(a) { this._rec('stroke', this._snapshot()); };
    C.clip = function(a) { this._rec('clip'); };
    C.isPointInPath = function() { return false; };
    C.isPointInStroke = function() { return false; };

    // Rectángulos.
    // Fase 7.196 — el chrome reproduce el log de comandos COMPLETO cada frame
    // (la spec dice que el bitmap es persistente). Un clear/fill de canvas
    // entero con transform identidad oculta todo lo anterior, así que ahí
    // TRUNCAMOS el log: acota memoria en animaciones (rAF que limpian cada
    // frame) y hace que el replay muestre sólo el frame actual.
    C._covers_canvas = function(x, y, w, h) {
        var t = this._state.transform;
        var identity = t[0] === 1 && t[1] === 0 && t[2] === 0 && t[3] === 1 && t[4] === 0 && t[5] === 0;
        if (!identity) return false;
        var cw = (this.canvas && this.canvas.width) || 300;
        var ch = (this.canvas && this.canvas.height) || 150;
        return x <= 0 && y <= 0 && (x + w) >= cw && (y + h) >= ch;
    };
    C.clearRect = function(x, y, w, h) {
        if (this._covers_canvas(x, y, w, h)) { this._cmds = []; }
        else { this._rec('clearRect', x, y, w, h); }
    };
    C.fillRect = function(x, y, w, h) {
        // Fondo opaco de canvas entero → también trunca (patrón común de
        // limpiar pintando un rect de fondo en vez de clearRect).
        if (this._covers_canvas(x, y, w, h) && this._state.globalAlpha >= 1 &&
            typeof this._state.fillStyle === 'string' &&
            this._state.fillStyle.indexOf('rgba') < 0 && this._state.fillStyle.indexOf('hsla') < 0 &&
            this._state.fillStyle !== 'transparent') {
            this._cmds = [];
        }
        this._rec('fillRect', x, y, w, h, serStyle(this._state.fillStyle), this._snapshot());
    };
    C.strokeRect = function(x, y, w, h) { this._rec('strokeRect', x, y, w, h, serStyle(this._state.strokeStyle), this._snapshot()); };

    // Texto.
    C.fillText = function(text, x, y, maxWidth) { this._rec('fillText', String(text), x, y, maxWidth, this._snapshot()); };
    C.strokeText = function(text, x, y, maxWidth) { this._rec('strokeText', String(text), x, y, maxWidth, this._snapshot()); };
    C.measureText = function(text) { return new TextMetrics(text, parseFontPx(this._state.font)); };

    // Imágenes. Fase 7.197b — resolvemos la fuente a su `src` (string) para
    // que el chrome la decodifique y la pinte. Una fuente sin `src`
    // (HTMLCanvasElement / ImageBitmap) registra src vacío → el painter la
    // ignora (no-op). Las coordenadas (2 / 4 / 8 números) van tal cual.
    C.drawImage = function(image) {
        var args = Array.prototype.slice.call(arguments);
        var src = '';
        if (image) src = image.src || image.currentSrc || image._src || '';
        this._rec.apply(this, ['drawImage', String(src)].concat(args.slice(1)));
    };

    // Gradientes / patrones.
    C.createLinearGradient = function(x0, y0, x1, y1) { return new CanvasGradient('linear', [x0, y0, x1, y1]); };
    C.createRadialGradient = function(x0, y0, r0, x1, y1, r1) { return new CanvasGradient('radial', [x0, y0, r0, x1, y1, r1]); };
    C.createConicGradient = function(a, x, y) { return new CanvasGradient('conic', [a, x, y]); };
    C.createPattern = function(image, repetition) { return new CanvasPattern(image, repetition); };

    // ImageData.
    C.createImageData = function(a, b) {
        if (a instanceof ImageData) return new ImageData(a.width, a.height);
        return new ImageData(Math.abs(a | 0), Math.abs(b | 0));
    };
    C.getImageData = function(sx, sy, sw, sh) { return new ImageData(Math.abs(sw | 0), Math.abs(sh | 0)); };
    C.putImageData = function(imageData, dx, dy) { this._rec('putImageData', this._id, dx, dy); };

    // Line dash.
    C.setLineDash = function(segments) { this._state.lineDash = (segments || []).slice(); };
    C.getLineDash = function() { return this._state.lineDash.slice(); };

    // Focus helpers (no-op funcional).
    C.drawFocusIfNeeded = function() {};
    C.scrollPathIntoView = function() {};
    C.getContextAttributes = function() { return { alpha: true, colorSpace: 'srgb', desynchronized: false, willReadFrequently: false }; };

    globalThis.CanvasRenderingContext2D = CanvasRenderingContext2D;
    globalThis.OffscreenCanvasRenderingContext2D = CanvasRenderingContext2D;

    // ---- OffscreenCanvas ----
    function OffscreenCanvas(width, height) {
        globalThis.EventTarget.call(this);
        this.width = width | 0;
        this.height = height | 0;
        this._ctx = null;
        this.oncontextlost = null; this.oncontextrestored = null;
    }
    OffscreenCanvas.prototype = Object.create(globalThis.EventTarget.prototype);
    OffscreenCanvas.prototype.constructor = OffscreenCanvas;
    OffscreenCanvas.prototype.getContext = function(type, attrs) {
        if (type === '2d') {
            if (this._ctx && this._ctx instanceof CanvasRenderingContext2D) return this._ctx;
            this._ctx = new CanvasRenderingContext2D(this);
            return this._ctx;
        }
        if ((type === 'webgl' || type === 'webgl2' || type === 'experimental-webgl') &&
            typeof globalThis.__puriy_webgl_context === 'function') {
            if (this._ctx && this._ctx._glType === type) return this._ctx;
            this._ctx = globalThis.__puriy_webgl_context(this, type, attrs);
            return this._ctx;
        }
        return null;
    };
    OffscreenCanvas.prototype.convertToBlob = function(opts) {
        opts = opts || {};
        var type = opts.type || 'image/png';
        return Promise.resolve(new globalThis.Blob([], { type: type }));
    };
    OffscreenCanvas.prototype.transferToImageBitmap = function() {
        return new ImageBitmap(this.width, this.height);
    };
    globalThis.OffscreenCanvas = OffscreenCanvas;

    // ---- Recolector para el chrome (Fase 7.196) ----
    // Devuelve un frame por cada `<canvas>` DOM que pidió un contexto 2D:
    // `{ id, width, height, cmds }`. El chrome lo serializa con
    // `JSON.stringify` y lo interpreta en Rust para pintar con vello. Los
    // contextos de `OffscreenCanvas` (sin domId) no se incluyen — no tienen
    // box en la página. Si un canvas pidió contexto varias veces, queda el
    // último (getContext cachea, así que en la práctica es el mismo).
    globalThis.__puriy_dom_canvas_ctxs = globalThis.__puriy_dom_canvas_ctxs || [];
    globalThis.__puriy_collect_canvas = function() {
        var reg = globalThis.__puriy_dom_canvas_ctxs || [];
        var byId = {};
        for (var i = 0; i < reg.length; i++) {
            var e = reg[i];
            if (!e || !e.ctx || e.domId == null) continue;
            var cv = e.ctx.canvas || {};
            byId[e.domId] = {
                id: String(e.domId),
                width: cv.width || 300,
                height: cv.height || 150,
                cmds: e.ctx._cmds || []
            };
        }
        var out = [];
        for (var k in byId) { if (Object.prototype.hasOwnProperty.call(byId, k)) out.push(byId[k]); }
        return out;
    };

    void 0;
})();
"#;
