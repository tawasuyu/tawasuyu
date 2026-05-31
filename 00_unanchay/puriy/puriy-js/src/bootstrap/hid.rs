pub(crate) const HID_BOOTSTRAP: &str = r#"
// Fase 7.121 — Web HID API (`navigator.hid`). Apps de periféricos exóticos (gamepads no
// estándar, teclados macro, dispositivos médicos) la usan cuando WebUSB/Gamepad no bastan.
// El motor no tiene HID real: `requestDevice()` es host-driven y el permiso por defecto es
// denegado. El chrome resuelve con `__puriy_hid_resolve(list)` (una lista de descriptores) o
// cancela con `__puriy_hid_reject(name)`. Sin selección, resuelve `[]` (la spec de HID
// devuelve un array vacío, no rechaza). Los dispositivos ya concedidos salen por
// `getDevices()`. `sendReport()` publica una mutación; el host inyecta input con
// `__puriy_hid_inputreport(deviceId, reportId, data)`.
(function() {
    if (globalThis.navigator == null) globalThis.navigator = {};
    var nav = globalThis.navigator;
    if (nav.hid != null) return;

    var pendingRequest = null;
    var grantedDevices = [];
    var nextId = 1;

    function HIDDevice(info) {
        info = info || {};
        this._id = info.id || ('hid-' + (nextId++));
        this.vendorId = info.vendorId || 0;
        this.productId = info.productId || 0;
        this.productName = info.productName || '';
        this.collections = info.collections || [];
        this.opened = false;
        this._listeners = {};
    }
    HIDDevice.prototype.open = function() {
        if (this.opened) return Promise.resolve();
        this.opened = true;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'hid-open', value: this._id
        });
        return Promise.resolve();
    };
    HIDDevice.prototype.close = function() {
        this.opened = false;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'hid-close', value: this._id
        });
        return Promise.resolve();
    };
    HIDDevice.prototype.forget = function() {
        var i = grantedDevices.indexOf(this);
        if (i >= 0) grantedDevices.splice(i, 1);
        return Promise.resolve();
    };
    HIDDevice.prototype.sendReport = function(reportId, data) {
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'hid-send-report',
            value: this._id + ':' + reportId + ':' +
                Array.prototype.slice.call((data && data.length != null) ? data : []).join(',')
        });
        return Promise.resolve();
    };
    HIDDevice.prototype.sendFeatureReport = function(reportId, data) {
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'hid-send-feature',
            value: this._id + ':' + reportId
        });
        return Promise.resolve();
    };
    HIDDevice.prototype.receiveFeatureReport = function(reportId) {
        // El host responde con __puriy_hid_feature_resolve; sin host, DataView vacío.
        var self = this;
        return new Promise(function(resolve) {
            self._pendingFeature = resolve;
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'hid-receive-feature',
                value: self._id + ':' + reportId
            });
        });
    };
    HIDDevice.prototype.addEventListener = function(type, fn) {
        (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    HIDDevice.prototype.removeEventListener = function(type, fn) {
        var a = this._listeners[type]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    HIDDevice.prototype.dispatchEvent = function(ev) {
        var a = this._listeners[ev.type];
        if (a) { for (var i = 0; i < a.length; i++) a[i].call(this, ev); }
        if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
        return true;
    };

    function HID() { this._listeners = {}; }
    HID.prototype.addEventListener = function(type, fn) {
        (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    HID.prototype.removeEventListener = function(type, fn) {
        var a = this._listeners[type]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    HID.prototype.dispatchEvent = function(ev) {
        var a = this._listeners[ev.type];
        if (a) { for (var i = 0; i < a.length; i++) a[i].call(this, ev); }
        if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
        return true;
    };
    HID.prototype.getDevices = function() {
        return Promise.resolve(grantedDevices.slice());
    };
    HID.prototype.requestDevice = function(options) {
        if (pendingRequest) {
            return Promise.reject(new globalThis.DOMException(
                'requestDevice ya en curso', 'InvalidStateError'));
        }
        return new Promise(function(resolve, reject) {
            pendingRequest = { resolve: resolve, reject: reject };
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'hid-request',
                value: JSON.stringify((options && options.filters) || [])
            });
        });
    };

    var hid = new HID();

    function findDevice(id) {
        for (var i = 0; i < grantedDevices.length; i++) {
            if (grantedDevices[i]._id === id) return grantedDevices[i];
        }
        return null;
    }

    // El host concede dispositivos tras el picker. list: array de descriptores.
    globalThis.__puriy_hid_resolve = function(list) {
        if (!pendingRequest) return false;
        var p = pendingRequest; pendingRequest = null;
        var out = [];
        list = list || [];
        for (var i = 0; i < list.length; i++) {
            var dev = new HIDDevice(list[i]);
            grantedDevices.push(dev);
            out.push(dev);
        }
        p.resolve(out);
        return true;
    };
    // Cancela el picker. Sin selección, name === 'NotFoundError'.
    globalThis.__puriy_hid_reject = function(name) {
        if (!pendingRequest) return false;
        var p = pendingRequest; pendingRequest = null;
        p.reject(new globalThis.DOMException(
            'requestDevice cancelado', name || 'NotFoundError'));
        return true;
    };
    // El host registra un dispositivo ya concedido (visible en getDevices).
    globalThis.__puriy_hid_add_device = function(info) {
        var dev = new HIDDevice(info);
        grantedDevices.push(dev);
        return dev;
    };
    // El host inyecta un input report al dispositivo abierto.
    globalThis.__puriy_hid_inputreport = function(deviceId, reportId, data) {
        var dev = findDevice(deviceId); if (!dev) return false;
        dev.dispatchEvent({ type: 'inputreport', device: dev, reportId: reportId, data: data });
        return true;
    };
    // El host resuelve un receiveFeatureReport pendiente.
    globalThis.__puriy_hid_feature_resolve = function(deviceId, data) {
        var dev = findDevice(deviceId); if (!dev || !dev._pendingFeature) return false;
        var r = dev._pendingFeature; dev._pendingFeature = null;
        r(data);
        return true;
    };
    globalThis.__puriy_hid_connect = function(deviceId) {
        var dev = findDevice(deviceId);
        hid.dispatchEvent({ type: 'connect', device: dev || null });
        return true;
    };
    globalThis.__puriy_hid_disconnect = function(deviceId) {
        var dev = findDevice(deviceId);
        hid.dispatchEvent({ type: 'disconnect', device: dev || null });
        return true;
    };

    nav.hid = hid;
    globalThis.HID = HID;
    globalThis.HIDDevice = HIDDevice;
    void 0;
})();
"#;
