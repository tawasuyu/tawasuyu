pub(crate) const WEBXR_BOOTSTRAP: &str = r#"
// Fase 7.158 — WebXR Device API (`navigator.xr` = `XRSystem` + `XRSession`/`XRFrame`/
// `XRViewerPose`/`XRView`/`XRRigidTransform`/`XRReferenceSpace`/`XRWebGLLayer` +
// `XRInputSource`). Realidad virtual/aumentada. Host-driven (mismo molde que la
// familia device-access 7.120+): `requestSession` publica `kind: 'xr-request-session'`
// y el chrome confirma con `__puriy_xr_session_resolve(id)` o la rechaza con
// `__puriy_xr_session_reject(id, name)` (`NotSupportedError`). Cierra el pendiente del
// trío geometría 7.153: `XRRigidTransform` usa `DOMMatrix` para componer `matrix`/`inverse`.
//   · `isSessionSupported(mode)` es host-decidido: default 'inline' → true, immersive →
//     false; el chrome lo fija con `__puriy_xr_set_supported(mode, bool)`.
//   · El loop de frames (`requestAnimationFrame`) lo dispara el chrome vía
//     `__puriy_xr_frame(sessionId, time)`; sin device real, no se auto-dispara.
(function() {
    if (globalThis.navigator != null && globalThis.navigator.xr != null) return;
    var nextId = 1;
    var supported = { inline: true };  // immersive-vr/ar quedan host-decididos (default false)

    // ---------- XRRigidTransform (apoyado en DOMMatrix de Fase 7.153) ----------
    function XRRigidTransform(position, orientation) {
        position = position || {};
        orientation = orientation || {};
        this.position = { x: position.x || 0, y: position.y || 0, z: position.z || 0, w: position.w != null ? position.w : 1 };
        this.orientation = { x: orientation.x || 0, y: orientation.y || 0, z: orientation.z || 0, w: orientation.w != null ? orientation.w : 1 };
        this._matrix = null;
        this._inverse = null;
    }
    // Construye la matriz 4x4 column-major (rotación del cuaternión + traslación).
    function quatTransMatrix(q, p) {
        var x = q.x, y = q.y, z = q.z, w = q.w;
        var x2 = x + x, y2 = y + y, z2 = z + z;
        var xx = x * x2, xy = x * y2, xz = x * z2;
        var yy = y * y2, yz = y * z2, zz = z * z2;
        var wx = w * x2, wy = w * y2, wz = w * z2;
        return new Float32Array([
            1 - (yy + zz), xy + wz, xz - wy, 0,
            xy - wz, 1 - (xx + zz), yz + wx, 0,
            xz + wy, yz - wx, 1 - (xx + yy), 0,
            p.x, p.y, p.z, 1
        ]);
    }
    Object.defineProperty(XRRigidTransform.prototype, 'matrix', {
        get: function() {
            if (this._matrix == null) this._matrix = quatTransMatrix(this.orientation, this.position);
            return this._matrix;
        }, configurable: true
    });
    Object.defineProperty(XRRigidTransform.prototype, 'inverse', {
        get: function() {
            if (this._inverse == null) {
                var inv = new XRRigidTransform();
                if (typeof globalThis.DOMMatrix === 'function') {
                    var m = new globalThis.DOMMatrix(Array.prototype.slice.call(this.matrix));
                    var im = m.inverse();
                    inv._matrix = im.toFloat32Array ? im.toFloat32Array() : new Float32Array(16);
                } else {
                    inv._matrix = new Float32Array(this.matrix);
                }
                inv._inverse = this;
                this._inverse = inv;
            }
            return this._inverse;
        }, configurable: true
    });
    globalThis.XRRigidTransform = XRRigidTransform;

    // ---------- XRSpace / XRReferenceSpace ----------
    function XRSpace() {}
    globalThis.XRSpace = XRSpace;
    function XRReferenceSpace(type) {
        var et = new globalThis.EventTarget();
        for (var m in et) { if (typeof et[m] === 'function') this[m] = et[m].bind(et); }
        this._type = type || 'local';
        this.onreset = null;
    }
    XRReferenceSpace.prototype = Object.create(XRSpace.prototype);
    XRReferenceSpace.prototype.constructor = XRReferenceSpace;
    XRReferenceSpace.prototype.getOffsetReferenceSpace = function(transform) {
        var s = new XRReferenceSpace(this._type);
        s._offset = transform;
        return s;
    };
    globalThis.XRReferenceSpace = XRReferenceSpace;

    // ---------- XRView / XRViewerPose / XRFrame ----------
    function XRView(eye) {
        this.eye = eye || 'none';
        this.projectionMatrix = new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, -1, -1, 0, 0, -0.2, 0]);
        this.transform = new XRRigidTransform();
        this.recommendedViewportScale = 1;
    }
    globalThis.XRView = XRView;

    function XRViewerPose(views) {
        this.transform = new XRRigidTransform();
        this.views = views;
        this.emulatedPosition = false;
        this.linearVelocity = null;
        this.angularVelocity = null;
    }
    globalThis.XRViewerPose = XRViewerPose;

    function XRFrame(session) { this.session = session; this.predictedDisplayTime = 0; }
    XRFrame.prototype.getViewerPose = function(refSpace) {
        var stereo = (this.session._mode === 'immersive-vr' || this.session._mode === 'immersive-ar');
        var views = stereo ? [new XRView('left'), new XRView('right')] : [new XRView('none')];
        return new XRViewerPose(views);
    };
    XRFrame.prototype.getPose = function(space, baseSpace) { return new XRRigidTransform(); };
    XRFrame.prototype.getJointPose = function() { return null; };
    globalThis.XRFrame = XRFrame;

    // ---------- XRWebGLLayer ----------
    function XRWebGLLayer(session, context, options) {
        this._session = session;
        this.context = context;
        options = options || {};
        this.antialias = options.antialias !== false;
        this.ignoreDepthValues = !!options.ignoreDepthValues;
        this.framebuffer = null;  // null = default framebuffer (spec lo permite)
        this.framebufferWidth = 1280;
        this.framebufferHeight = 720;
    }
    XRWebGLLayer.prototype.getViewport = function(view) {
        var half = this.framebufferWidth / 2;
        if (view && view.eye === 'right') return { x: half, y: 0, width: half, height: this.framebufferHeight };
        return { x: 0, y: 0, width: (view && view.eye === 'left') ? half : this.framebufferWidth, height: this.framebufferHeight };
    };
    XRWebGLLayer.getNativeFramebufferScaleFactor = function(session) { return 1.0; };
    globalThis.XRWebGLLayer = XRWebGLLayer;

    // ---------- XRSession ----------
    function XRSession(mode) {
        var et = new globalThis.EventTarget();
        for (var m in et) { if (typeof et[m] === 'function') this[m] = et[m].bind(et); }
        this._id = nextId++;
        this._mode = mode || 'inline';
        this.visibilityState = 'visible';
        this.environmentBlendMode = (mode === 'immersive-ar') ? 'alpha-blend' : 'opaque';
        this.inputSources = [];
        this.renderState = { depthNear: 0.1, depthFar: 1000, inlineVerticalFieldOfView: null, baseLayer: null, layers: [] };
        this._rafCallbacks = [];
        this._rafNext = 1;
        this._ended = false;
        this.onend = null; this.onselect = null; this.oninputsourceschange = null;
        this.onvisibilitychange = null; this.onselectstart = null; this.onselectend = null;
        this.onsqueeze = null; this.onsqueezestart = null; this.onsqueezeend = null;
        globalThis.__puriy_xr_sessions = globalThis.__puriy_xr_sessions || {};
        globalThis.__puriy_xr_sessions[this._id] = this;
    }
    XRSession.prototype.requestReferenceSpace = function(type) {
        return Promise.resolve(new XRReferenceSpace(type));
    };
    XRSession.prototype.updateRenderState = function(state) {
        if (state) for (var k in state) this.renderState[k] = state[k];
    };
    XRSession.prototype.requestAnimationFrame = function(cb) {
        var handle = this._rafNext++;
        this._rafCallbacks.push({ handle: handle, cb: cb });
        return handle;
    };
    XRSession.prototype.cancelAnimationFrame = function(handle) {
        for (var i = 0; i < this._rafCallbacks.length; i++) {
            if (this._rafCallbacks[i].handle === handle) { this._rafCallbacks.splice(i, 1); return; }
        }
    };
    // Drena los callbacks pendientes con un XRFrame — lo invoca el chrome.
    XRSession.prototype._fireFrame = function(time) {
        var pend = this._rafCallbacks;
        this._rafCallbacks = [];
        var frame = new XRFrame(this);
        frame.predictedDisplayTime = time || 0;
        for (var i = 0; i < pend.length; i++) {
            try { pend[i].cb(time || 0, frame); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
    };
    XRSession.prototype.requestHitTestSource = function() { return Promise.reject(new globalThis.DOMException('hit-test no soportado', 'NotSupportedError')); };
    XRSession.prototype.end = function() {
        var self = this;
        return new Promise(function(resolve) {
            if (self._ended) { resolve(undefined); return; }
            self._ended = true;
            delete globalThis.__puriy_xr_sessions[self._id];
            var ev = (typeof globalThis.Event === 'function') ? new globalThis.Event('end') : { type: 'end' };
            if (typeof self.onend === 'function') { try { self.onend(ev); } catch (e) {} }
            if (typeof self.dispatchEvent === 'function') self.dispatchEvent(ev);
            resolve(undefined);
        });
    };
    globalThis.XRSession = XRSession;

    // ---------- XRSystem (navigator.xr) ----------
    function XRSystem() {
        var et = new globalThis.EventTarget();
        for (var m in et) { if (typeof et[m] === 'function') this[m] = et[m].bind(et); }
        this.ondevicechange = null;
    }
    XRSystem.prototype.isSessionSupported = function(mode) {
        return Promise.resolve(!!supported[mode]);
    };
    XRSystem.prototype.requestSession = function(mode, options) {
        mode = mode || 'inline';
        var id = nextId++;
        globalThis.__puriy_xr_pending = globalThis.__puriy_xr_pending || {};
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'xr-request-session', value: id + '' + mode });
        return new Promise(function(resolve, reject) {
            globalThis.__puriy_xr_pending[id] = { resolve: resolve, reject: reject, mode: mode };
        });
    };
    globalThis.XRSystem = XRSystem;

    // ---------- hooks del chrome ----------
    globalThis.__puriy_xr_set_supported = function(mode, ok) { supported[mode] = !!ok; };
    globalThis.__puriy_xr_session_resolve = function(id) {
        var p = globalThis.__puriy_xr_pending && globalThis.__puriy_xr_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_xr_pending[id];
        p.resolve(new XRSession(p.mode));
        return true;
    };
    globalThis.__puriy_xr_session_reject = function(id, name) {
        var p = globalThis.__puriy_xr_pending && globalThis.__puriy_xr_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_xr_pending[id];
        p.reject(new globalThis.DOMException('Sesión XR rechazada', (name != null) ? String(name) : 'NotSupportedError'));
        return true;
    };
    globalThis.__puriy_xr_frame = function(sessionId, time) {
        var s = globalThis.__puriy_xr_sessions && globalThis.__puriy_xr_sessions[sessionId];
        if (!s) return false;
        s._fireFrame(time);
        return true;
    };

    var nav = globalThis.navigator = globalThis.navigator || {};
    nav.xr = new XRSystem();
    void 0;
})();
"#;
