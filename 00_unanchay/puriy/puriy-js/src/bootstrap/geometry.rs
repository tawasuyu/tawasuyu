pub(crate) const GEOMETRY_BOOTSTRAP: &str = r#"
// Fase 7.153 — Geometry Interfaces (`DOMPoint`/`DOMPointReadOnly`,
// `DOMRect`/`DOMRectReadOnly`, `DOMQuad`, `DOMMatrix`/`DOMMatrixReadOnly`).
// Tipos puros de geometría 2D/3D que el Canvas 2D (Fase 7.150), CSS Typed OM y
// WebXR/animaciones usan para describir posiciones, cajas y transformaciones.
// 100% JS-puro y matemáticamente real: las matrices son 4x4 column-major (mismo
// orden que `toFloat32Array`), con multiply/inverse/transformPoint exactos.
// Cierra el pendiente del trío gráfico: `ctx.getTransform()` ahora devuelve un
// `DOMMatrix` real en vez del literal `{a..f}` (Fase 7.150).
(function() {
    if (globalThis.DOMMatrix != null) return;

    // ---------- DOMPointReadOnly / DOMPoint ----------
    function DOMPointReadOnly(x, y, z, w) {
        this._x = x != null ? Number(x) : 0;
        this._y = y != null ? Number(y) : 0;
        this._z = z != null ? Number(z) : 0;
        this._w = w != null ? Number(w) : 1;
    }
    DOMPointReadOnly.prototype.matrixTransform = function(m) {
        var mat = (m instanceof DOMMatrixReadOnly) ? m : DOMMatrixReadOnly.fromMatrix(m);
        return mat.transformPoint(this);
    };
    DOMPointReadOnly.prototype.toJSON = function() {
        return { x: this.x, y: this.y, z: this.z, w: this.w };
    };
    DOMPointReadOnly.fromPoint = function(p) {
        p = p || {};
        return new DOMPointReadOnly(p.x, p.y, p.z, p.w);
    };
    ['x', 'y', 'z', 'w'].forEach(function(k) {
        Object.defineProperty(DOMPointReadOnly.prototype, k, {
            get: function() { return this['_' + k]; }, configurable: true, enumerable: true
        });
    });

    function DOMPoint(x, y, z, w) { DOMPointReadOnly.call(this, x, y, z, w); }
    DOMPoint.prototype = Object.create(DOMPointReadOnly.prototype);
    DOMPoint.prototype.constructor = DOMPoint;
    DOMPoint.fromPoint = function(p) { p = p || {}; return new DOMPoint(p.x, p.y, p.z, p.w); };
    ['x', 'y', 'z', 'w'].forEach(function(k) {
        Object.defineProperty(DOMPoint.prototype, k, {
            get: function() { return this['_' + k]; },
            set: function(v) { this['_' + k] = Number(v); },
            configurable: true, enumerable: true
        });
    });

    // ---------- DOMRectReadOnly / DOMRect ----------
    function DOMRectReadOnly(x, y, width, height) {
        this._x = x != null ? Number(x) : 0;
        this._y = y != null ? Number(y) : 0;
        this._width = width != null ? Number(width) : 0;
        this._height = height != null ? Number(height) : 0;
    }
    Object.defineProperty(DOMRectReadOnly.prototype, 'left', {
        get: function() { return Math.min(this._x, this._x + this._width); }, configurable: true });
    Object.defineProperty(DOMRectReadOnly.prototype, 'right', {
        get: function() { return Math.max(this._x, this._x + this._width); }, configurable: true });
    Object.defineProperty(DOMRectReadOnly.prototype, 'top', {
        get: function() { return Math.min(this._y, this._y + this._height); }, configurable: true });
    Object.defineProperty(DOMRectReadOnly.prototype, 'bottom', {
        get: function() { return Math.max(this._y, this._y + this._height); }, configurable: true });
    DOMRectReadOnly.prototype.toJSON = function() {
        return { x: this.x, y: this.y, width: this.width, height: this.height,
                 top: this.top, right: this.right, bottom: this.bottom, left: this.left };
    };
    DOMRectReadOnly.fromRect = function(r) {
        r = r || {}; return new DOMRectReadOnly(r.x, r.y, r.width, r.height);
    };
    ['x', 'y', 'width', 'height'].forEach(function(k) {
        Object.defineProperty(DOMRectReadOnly.prototype, k, {
            get: function() { return this['_' + k]; }, configurable: true, enumerable: true });
    });

    function DOMRect(x, y, width, height) { DOMRectReadOnly.call(this, x, y, width, height); }
    DOMRect.prototype = Object.create(DOMRectReadOnly.prototype);
    DOMRect.prototype.constructor = DOMRect;
    DOMRect.fromRect = function(r) { r = r || {}; return new DOMRect(r.x, r.y, r.width, r.height); };
    ['x', 'y', 'width', 'height'].forEach(function(k) {
        Object.defineProperty(DOMRect.prototype, k, {
            get: function() { return this['_' + k]; },
            set: function(v) { this['_' + k] = Number(v); },
            configurable: true, enumerable: true });
    });

    // ---------- DOMQuad ----------
    function DOMQuad(p1, p2, p3, p4) {
        this.p1 = DOMPoint.fromPoint(p1);
        this.p2 = DOMPoint.fromPoint(p2);
        this.p3 = DOMPoint.fromPoint(p3);
        this.p4 = DOMPoint.fromPoint(p4);
    }
    DOMQuad.fromRect = function(r) {
        r = r || {}; var x = +r.x || 0, y = +r.y || 0, w = +r.width || 0, h = +r.height || 0;
        return new DOMQuad({x: x, y: y}, {x: x + w, y: y}, {x: x + w, y: y + h}, {x: x, y: y + h});
    };
    DOMQuad.fromQuad = function(q) {
        q = q || {}; return new DOMQuad(q.p1, q.p2, q.p3, q.p4);
    };
    DOMQuad.prototype.getBounds = function() {
        var xs = [this.p1.x, this.p2.x, this.p3.x, this.p4.x];
        var ys = [this.p1.y, this.p2.y, this.p3.y, this.p4.y];
        var minX = Math.min.apply(null, xs), maxX = Math.max.apply(null, xs);
        var minY = Math.min.apply(null, ys), maxY = Math.max.apply(null, ys);
        return new DOMRect(minX, minY, maxX - minX, maxY - minY);
    };
    DOMQuad.prototype.toJSON = function() {
        return { p1: this.p1.toJSON(), p2: this.p2.toJSON(), p3: this.p3.toJSON(), p4: this.p4.toJSON() };
    };

    // ---------- helpers de matriz 4x4 (column-major, orden toFloat32Array) ----------
    function ident() { return [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1]; }
    // C = A · B (post-multiply). Elemento (row,col): Σ_k A(row,k)·B(k,col).
    function mul(a, b) {
        var c = new Array(16);
        for (var col = 0; col < 4; col++) {
            for (var row = 0; row < 4; row++) {
                var s = 0;
                for (var k = 0; k < 4; k++) { s += a[k * 4 + row] * b[col * 4 + k]; }
                c[col * 4 + row] = s;
            }
        }
        return c;
    }
    function is2Dm(m) {
        return m[2] === 0 && m[3] === 0 && m[6] === 0 && m[7] === 0 &&
               m[8] === 0 && m[9] === 0 && m[10] === 1 && m[11] === 0 &&
               m[14] === 0 && m[15] === 1;
    }
    function rad(deg) { return (deg || 0) * Math.PI / 180; }
    function rotZ(r) { var c = Math.cos(r), s = Math.sin(r); return [c, s, 0, 0, -s, c, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]; }
    function rotX(r) { var c = Math.cos(r), s = Math.sin(r); return [1, 0, 0, 0, 0, c, s, 0, 0, -s, c, 0, 0, 0, 0, 1]; }
    function rotY(r) { var c = Math.cos(r), s = Math.sin(r); return [c, 0, -s, 0, 0, 1, 0, 0, s, 0, c, 0, 0, 0, 0, 1]; }
    function translation(x, y, z) { return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, x, y, z, 1]; }
    function scaling(x, y, z) { return [x, 0, 0, 0, 0, y, 0, 0, 0, 0, z, 0, 0, 0, 0, 1]; }
    // Inversa 4x4 por cofactores (algoritmo MESA), column-major. null si singular.
    function invert(m) {
        var inv = new Array(16);
        inv[0] = m[5]*m[10]*m[15] - m[5]*m[11]*m[14] - m[9]*m[6]*m[15] + m[9]*m[7]*m[14] + m[13]*m[6]*m[11] - m[13]*m[7]*m[10];
        inv[4] = -m[4]*m[10]*m[15] + m[4]*m[11]*m[14] + m[8]*m[6]*m[15] - m[8]*m[7]*m[14] - m[12]*m[6]*m[11] + m[12]*m[7]*m[10];
        inv[8] = m[4]*m[9]*m[15] - m[4]*m[11]*m[13] - m[8]*m[5]*m[15] + m[8]*m[7]*m[13] + m[12]*m[5]*m[11] - m[12]*m[7]*m[9];
        inv[12] = -m[4]*m[9]*m[14] + m[4]*m[10]*m[13] + m[8]*m[5]*m[14] - m[8]*m[6]*m[13] - m[12]*m[5]*m[10] + m[12]*m[6]*m[9];
        inv[1] = -m[1]*m[10]*m[15] + m[1]*m[11]*m[14] + m[9]*m[2]*m[15] - m[9]*m[3]*m[14] - m[13]*m[2]*m[11] + m[13]*m[3]*m[10];
        inv[5] = m[0]*m[10]*m[15] - m[0]*m[11]*m[14] - m[8]*m[2]*m[15] + m[8]*m[3]*m[14] + m[12]*m[2]*m[11] - m[12]*m[3]*m[10];
        inv[9] = -m[0]*m[9]*m[15] + m[0]*m[11]*m[13] + m[8]*m[1]*m[15] - m[8]*m[3]*m[13] - m[12]*m[1]*m[11] + m[12]*m[3]*m[9];
        inv[13] = m[0]*m[9]*m[14] - m[0]*m[10]*m[13] - m[8]*m[1]*m[14] + m[8]*m[2]*m[13] + m[12]*m[1]*m[10] - m[12]*m[2]*m[9];
        inv[2] = m[1]*m[6]*m[15] - m[1]*m[7]*m[14] - m[5]*m[2]*m[15] + m[5]*m[3]*m[14] + m[13]*m[2]*m[7] - m[13]*m[3]*m[6];
        inv[6] = -m[0]*m[6]*m[15] + m[0]*m[7]*m[14] + m[4]*m[2]*m[15] - m[4]*m[3]*m[14] - m[12]*m[2]*m[7] + m[12]*m[3]*m[6];
        inv[10] = m[0]*m[5]*m[15] - m[0]*m[7]*m[13] - m[4]*m[1]*m[15] + m[4]*m[3]*m[13] + m[12]*m[1]*m[7] - m[12]*m[3]*m[5];
        inv[14] = -m[0]*m[5]*m[14] + m[0]*m[6]*m[13] + m[4]*m[1]*m[14] - m[4]*m[2]*m[13] - m[12]*m[1]*m[6] + m[12]*m[2]*m[5];
        inv[3] = -m[1]*m[6]*m[11] + m[1]*m[7]*m[10] + m[5]*m[2]*m[11] - m[5]*m[3]*m[10] - m[9]*m[2]*m[7] + m[9]*m[3]*m[6];
        inv[7] = m[0]*m[6]*m[11] - m[0]*m[7]*m[10] - m[4]*m[2]*m[11] + m[4]*m[3]*m[10] + m[8]*m[2]*m[7] - m[8]*m[3]*m[6];
        inv[11] = -m[0]*m[5]*m[11] + m[0]*m[7]*m[9] + m[4]*m[1]*m[11] - m[4]*m[3]*m[9] - m[8]*m[1]*m[7] + m[8]*m[3]*m[5];
        inv[15] = m[0]*m[5]*m[10] - m[0]*m[6]*m[9] - m[4]*m[1]*m[10] + m[4]*m[2]*m[9] + m[8]*m[1]*m[6] - m[8]*m[2]*m[5];
        var det = m[0]*inv[0] + m[1]*inv[4] + m[2]*inv[8] + m[3]*inv[12];
        if (det === 0) return null;
        det = 1.0 / det;
        for (var i = 0; i < 16; i++) inv[i] *= det;
        return inv;
    }

    // ---------- DOMMatrixReadOnly / DOMMatrix ----------
    function parseInit(init) {
        if (init == null) return ident();
        if (typeof init === 'string') return ident(); // parse de cadena CSS no soportado
        if (init._m) return init._m.slice();
        if (typeof init.length === 'number' && typeof init !== 'function') {
            var a = Array.prototype.slice.call(init).map(Number);
            if (a.length === 6) return [a[0], a[1], 0, 0, a[2], a[3], 0, 0, 0, 0, 1, 0, a[4], a[5], 0, 1];
            if (a.length === 16) return a.slice();
            return ident();
        }
        return fromInitObject(init);
    }
    function fromInitObject(o) {
        var m = ident();
        var keys = ['m11','m12','m13','m14','m21','m22','m23','m24','m31','m32','m33','m34','m41','m42','m43','m44'];
        for (var i = 0; i < 16; i++) { if (o[keys[i]] != null) m[i] = Number(o[keys[i]]); }
        if (o.a != null) m[0] = Number(o.a);
        if (o.b != null) m[1] = Number(o.b);
        if (o.c != null) m[4] = Number(o.c);
        if (o.d != null) m[5] = Number(o.d);
        if (o.e != null) m[12] = Number(o.e);
        if (o.f != null) m[13] = Number(o.f);
        return m;
    }
    function toM(other) { return (other && other._m) ? other._m : parseInit(other); }

    function DOMMatrixReadOnly(init) { this._m = parseInit(init); }
    DOMMatrixReadOnly.fromMatrix = function(o) { return new DOMMatrixReadOnly(o || {}); };
    DOMMatrixReadOnly.fromFloat32Array = function(a) { return new DOMMatrixReadOnly(a); };
    DOMMatrixReadOnly.fromFloat64Array = function(a) { return new DOMMatrixReadOnly(a); };

    var IDX = { a:0, b:1, c:4, d:5, e:12, f:13,
                m11:0, m12:1, m13:2, m14:3, m21:4, m22:5, m23:6, m24:7,
                m31:8, m32:9, m33:10, m34:11, m41:12, m42:13, m43:14, m44:15 };
    function defineMatrixProps(proto, writable) {
        Object.keys(IDX).forEach(function(name) {
            var i = IDX[name];
            var desc = { get: function() { return this._m[i]; }, configurable: true, enumerable: true };
            if (writable) desc.set = function(v) { this._m[i] = Number(v); };
            Object.defineProperty(proto, name, desc);
        });
        Object.defineProperty(proto, 'is2D', {
            get: function() { return is2Dm(this._m); }, configurable: true });
        Object.defineProperty(proto, 'isIdentity', {
            get: function() { var m = this._m, I = ident();
                for (var i = 0; i < 16; i++) if (m[i] !== I[i]) return false; return true; },
            configurable: true });
    }
    defineMatrixProps(DOMMatrixReadOnly.prototype, false);

    var RO = DOMMatrixReadOnly.prototype;
    RO.multiply = function(o) { return new DOMMatrix(mul(this._m, toM(o))); };
    RO.translate = function(tx, ty, tz) {
        return new DOMMatrix(mul(this._m, translation(tx || 0, ty || 0, tz || 0))); };
    RO.scale = function(sx, sy, sz, ox, oy, oz) {
        sx = sx == null ? 1 : sx; sy = sy == null ? sx : sy; sz = sz == null ? 1 : sz;
        ox = ox || 0; oy = oy || 0; oz = oz || 0;
        var t = mul(translation(ox, oy, oz), mul(scaling(sx, sy, sz), translation(-ox, -oy, -oz)));
        return new DOMMatrix(mul(this._m, t)); };
    RO.scaleNonUniform = function(sx, sy) { return this.scale(sx == null ? 1 : sx, sy == null ? 1 : sy, 1); };
    RO.scale3d = function(s, ox, oy, oz) { return this.scale(s, s, s, ox, oy, oz); };
    RO.rotate = function(rx, ry, rz) {
        if (ry == null && rz == null) { rz = rx; rx = 0; ry = 0; }
        var m = this._m;
        if (rx) m = mul(m, rotX(rad(rx)));
        if (ry) m = mul(m, rotY(rad(ry)));
        if (rz) m = mul(m, rotZ(rad(rz)));
        return new DOMMatrix(m); };
    RO.rotateFromVector = function(x, y) {
        var ang = (x === 0 && y === 0) ? 0 : Math.atan2(y, x);
        return new DOMMatrix(mul(this._m, rotZ(ang))); };
    RO.rotateAxisAngle = function(x, y, z, angle) {
        var len = Math.sqrt(x * x + y * y + z * z);
        if (len === 0) return new DOMMatrix(this._m.slice());
        x /= len; y /= len; z /= len;
        var r = rad(angle), c = Math.cos(r), s = Math.sin(r), t = 1 - c;
        var ax = [t*x*x + c, t*x*y + s*z, t*x*z - s*y, 0,
                  t*x*y - s*z, t*y*y + c, t*y*z + s*x, 0,
                  t*x*z + s*y, t*y*z - s*x, t*z*z + c, 0,
                  0, 0, 0, 1];
        return new DOMMatrix(mul(this._m, ax)); };
    RO.skewX = function(deg) { var t = Math.tan(rad(deg));
        return new DOMMatrix(mul(this._m, [1, 0, 0, 0, t, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1])); };
    RO.skewY = function(deg) { var t = Math.tan(rad(deg));
        return new DOMMatrix(mul(this._m, [1, t, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1])); };
    RO.flipX = function() { return new DOMMatrix(mul(this._m, [-1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1])); };
    RO.flipY = function() { return new DOMMatrix(mul(this._m, [1, 0, 0, 0, 0, -1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1])); };
    RO.inverse = function() {
        var r = invert(this._m);
        if (r == null) { var nan = []; for (var i = 0; i < 16; i++) nan.push(NaN); return new DOMMatrix(nan); }
        return new DOMMatrix(r); };
    RO.transformPoint = function(p) {
        p = p || {}; var x = +p.x || 0, y = +p.y || 0, z = +p.z || 0, w = p.w == null ? 1 : +p.w;
        var m = this._m;
        return new DOMPoint(
            m[0]*x + m[4]*y + m[8]*z + m[12]*w,
            m[1]*x + m[5]*y + m[9]*z + m[13]*w,
            m[2]*x + m[6]*y + m[10]*z + m[14]*w,
            m[3]*x + m[7]*y + m[11]*z + m[15]*w); };
    RO.toFloat32Array = function() { return Float32Array.from(this._m); };
    RO.toFloat64Array = function() { return Float64Array.from(this._m); };
    RO.toJSON = function() {
        var o = {}; Object.keys(IDX).forEach(function(k) { o[k] = this._m[IDX[k]]; }, this);
        o.is2D = this.is2D; o.isIdentity = this.isIdentity; return o; };
    RO.toString = function() {
        var m = this._m;
        if (this.is2D) return 'matrix(' + [m[0], m[1], m[4], m[5], m[12], m[13]].join(', ') + ')';
        return 'matrix3d(' + m.join(', ') + ')'; };

    function DOMMatrix(init) { this._m = parseInit(init); }
    DOMMatrix.prototype = Object.create(DOMMatrixReadOnly.prototype);
    DOMMatrix.prototype.constructor = DOMMatrix;
    defineMatrixProps(DOMMatrix.prototype, true);
    DOMMatrix.fromMatrix = function(o) { return new DOMMatrix(o || {}); };
    DOMMatrix.fromFloat32Array = function(a) { return new DOMMatrix(a); };
    DOMMatrix.fromFloat64Array = function(a) { return new DOMMatrix(a); };

    var MUT = DOMMatrix.prototype;
    MUT.multiplySelf = function(o) { this._m = mul(this._m, toM(o)); return this; };
    MUT.preMultiplySelf = function(o) { this._m = mul(toM(o), this._m); return this; };
    MUT.translateSelf = function(tx, ty, tz) { this._m = this.translate(tx, ty, tz)._m; return this; };
    MUT.scaleSelf = function(sx, sy, sz, ox, oy, oz) { this._m = this.scale(sx, sy, sz, ox, oy, oz)._m; return this; };
    MUT.scale3dSelf = function(s, ox, oy, oz) { this._m = this.scale3d(s, ox, oy, oz)._m; return this; };
    MUT.rotateSelf = function(rx, ry, rz) { this._m = this.rotate(rx, ry, rz)._m; return this; };
    MUT.rotateFromVectorSelf = function(x, y) { this._m = this.rotateFromVector(x, y)._m; return this; };
    MUT.rotateAxisAngleSelf = function(x, y, z, a) { this._m = this.rotateAxisAngle(x, y, z, a)._m; return this; };
    MUT.skewXSelf = function(d) { this._m = this.skewX(d)._m; return this; };
    MUT.skewYSelf = function(d) { this._m = this.skewY(d)._m; return this; };
    MUT.invertSelf = function() { this._m = this.inverse()._m; return this; };
    MUT.setMatrixValue = function(s) { this._m = parseInit(s); return this; };

    globalThis.DOMPointReadOnly = DOMPointReadOnly;
    globalThis.DOMPoint = DOMPoint;
    globalThis.DOMRectReadOnly = DOMRectReadOnly;
    globalThis.DOMRect = DOMRect;
    globalThis.DOMQuad = DOMQuad;
    globalThis.DOMMatrixReadOnly = DOMMatrixReadOnly;
    globalThis.DOMMatrix = DOMMatrix;
    // Alias WebKit legacy.
    globalThis.WebKitCSSMatrix = DOMMatrix;
    void 0;
})();
"#;
