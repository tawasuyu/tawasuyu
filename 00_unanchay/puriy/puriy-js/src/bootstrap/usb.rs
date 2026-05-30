pub(crate) const USB_BOOTSTRAP: &str = r#"
// Fase 7.122 — Web USB API (`navigator.usb`). Apps que hablan con hardware USB a bajo nivel
// (firmware, instrumentos, lectores) la usan. El motor no tiene bus USB real:
// `requestDevice({filters})` es host-driven y el permiso por defecto es denegado. El chrome
// resuelve con `__puriy_usb_resolve(info)` (un descriptor) o cancela con
// `__puriy_usb_reject(name)`. Sin selección, rechaza `NotFoundError`. Los dispositivos ya
// concedidos salen por `getDevices()`. Las transferencias (`transferIn`/`transferOut`)
// devuelven `USBInTransferResult`/`USBOutTransferResult` resueltos por el host vía
// `__puriy_usb_transfer_resolve`.
(function() {
    if (globalThis.navigator == null) globalThis.navigator = {};
    var nav = globalThis.navigator;
    if (nav.usb != null) return;

    var pendingRequest = null;
    var grantedDevices = [];
    var nextId = 1;
    var nextTransfer = 1;
    var pendingTransfers = {};

    function USBDevice(info) {
        info = info || {};
        this._id = info.id || ('usb-' + (nextId++));
        this.vendorId = info.vendorId || 0;
        this.productId = info.productId || 0;
        this.productName = info.productName || '';
        this.manufacturerName = info.manufacturerName || '';
        this.serialNumber = info.serialNumber || '';
        this.usbVersionMajor = info.usbVersionMajor || 2;
        this.deviceClass = info.deviceClass || 0;
        this.configuration = null;
        this.configurations = info.configurations || [];
        this.opened = false;
    }
    USBDevice.prototype.open = function() {
        if (this.opened) return Promise.resolve();
        this.opened = true;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'usb-open', value: this._id });
        return Promise.resolve();
    };
    USBDevice.prototype.close = function() {
        this.opened = false;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'usb-close', value: this._id });
        return Promise.resolve();
    };
    USBDevice.prototype.forget = function() {
        var i = grantedDevices.indexOf(this);
        if (i >= 0) grantedDevices.splice(i, 1);
        return Promise.resolve();
    };
    USBDevice.prototype.selectConfiguration = function(configurationValue) {
        this.configuration = { configurationValue: configurationValue, interfaces: [] };
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'usb-select-config',
            value: this._id + ':' + configurationValue
        });
        return Promise.resolve();
    };
    USBDevice.prototype.claimInterface = function(interfaceNumber) {
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'usb-claim-interface',
            value: this._id + ':' + interfaceNumber
        });
        return Promise.resolve();
    };
    USBDevice.prototype.releaseInterface = function(interfaceNumber) {
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'usb-release-interface',
            value: this._id + ':' + interfaceNumber
        });
        return Promise.resolve();
    };
    USBDevice.prototype.selectAlternateInterface = function(iface, alt) {
        return Promise.resolve();
    };
    USBDevice.prototype.reset = function() { return Promise.resolve(); };
    USBDevice.prototype.transferIn = function(endpointNumber, length) {
        var self = this;
        return new Promise(function(resolve) {
            var tid = nextTransfer++;
            pendingTransfers[tid] = { dir: 'in', resolve: resolve };
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'usb-transfer-in',
                value: self._id + ':' + endpointNumber + ':' + length + ':' + tid
            });
        });
    };
    USBDevice.prototype.transferOut = function(endpointNumber, data) {
        var self = this;
        var len = (data && data.byteLength != null) ? data.byteLength
            : ((data && data.length != null) ? data.length : 0);
        return new Promise(function(resolve) {
            var tid = nextTransfer++;
            pendingTransfers[tid] = { dir: 'out', resolve: resolve, bytes: len };
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'usb-transfer-out',
                value: self._id + ':' + endpointNumber + ':' + len + ':' + tid
            });
        });
    };
    USBDevice.prototype.controlTransferIn = function(setup, length) {
        return this.transferIn(0, length);
    };
    USBDevice.prototype.controlTransferOut = function(setup, data) {
        return this.transferOut(0, data);
    };

    function USB() { this._listeners = {}; }
    USB.prototype.addEventListener = function(type, fn) {
        (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    USB.prototype.removeEventListener = function(type, fn) {
        var a = this._listeners[type]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    USB.prototype.dispatchEvent = function(ev) {
        var a = this._listeners[ev.type];
        if (a) { for (var i = 0; i < a.length; i++) a[i].call(this, ev); }
        if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
        return true;
    };
    USB.prototype.getDevices = function() {
        return Promise.resolve(grantedDevices.slice());
    };
    USB.prototype.requestDevice = function(options) {
        if (pendingRequest) {
            return Promise.reject(new globalThis.DOMException(
                'requestDevice ya en curso', 'InvalidStateError'));
        }
        return new Promise(function(resolve, reject) {
            pendingRequest = { resolve: resolve, reject: reject };
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'usb-request',
                value: JSON.stringify((options && options.filters) || [])
            });
        });
    };

    var usb = new USB();

    function findDevice(id) {
        for (var i = 0; i < grantedDevices.length; i++) {
            if (grantedDevices[i]._id === id) return grantedDevices[i];
        }
        return null;
    }

    // El host concede un dispositivo tras el picker.
    globalThis.__puriy_usb_resolve = function(info) {
        if (!pendingRequest) return false;
        var p = pendingRequest; pendingRequest = null;
        var dev = new USBDevice(info);
        grantedDevices.push(dev);
        p.resolve(dev);
        return true;
    };
    // Cancela el picker. Sin selección, name === 'NotFoundError'.
    globalThis.__puriy_usb_reject = function(name) {
        if (!pendingRequest) return false;
        var p = pendingRequest; pendingRequest = null;
        p.reject(new globalThis.DOMException(
            'requestDevice cancelado', name || 'NotFoundError'));
        return true;
    };
    // El host registra un dispositivo ya concedido (visible en getDevices).
    globalThis.__puriy_usb_add_device = function(info) {
        var dev = new USBDevice(info);
        grantedDevices.push(dev);
        return dev;
    };
    // El host resuelve una transferencia pendiente. result: USBInTransferResult |
    // USBOutTransferResult ({ status, data?/bytesWritten? }).
    globalThis.__puriy_usb_transfer_resolve = function(transferId, result) {
        var t = pendingTransfers[transferId]; if (!t) return false;
        delete pendingTransfers[transferId];
        if (result) { t.resolve(result); return true; }
        if (t.dir === 'in') t.resolve({ status: 'ok', data: null });
        else t.resolve({ status: 'ok', bytesWritten: t.bytes || 0 });
        return true;
    };
    globalThis.__puriy_usb_connect = function(deviceId) {
        var dev = findDevice(deviceId);
        usb.dispatchEvent({ type: 'connect', device: dev || null });
        return true;
    };
    globalThis.__puriy_usb_disconnect = function(deviceId) {
        var dev = findDevice(deviceId);
        usb.dispatchEvent({ type: 'disconnect', device: dev || null });
        return true;
    };

    nav.usb = usb;
    globalThis.USB = USB;
    globalThis.USBDevice = USBDevice;
    void 0;
})();
"#;
