pub(crate) const SERIAL_BOOTSTRAP: &str = r#"
// Fase 7.120 — Web Serial API (`navigator.serial`). Apps de IoT/microcontroladores la usan
// para hablar con dispositivos serie (Arduino, lectores, etc.). El motor no tiene puertos
// serie reales: el picker `requestPort()` es host-driven y el permiso por defecto es
// denegado. El chrome resuelve la selección vía `__puriy_serial_resolve(info)` o cancela con
// `__puriy_serial_reject(name)`. Sin selección, rechaza `NotFoundError`. Los puertos ya
// concedidos se exponen por `getPorts()` (poblado con `__puriy_serial_add_port`). `open()`
// publica una mutación `serial-open` al chrome y deja `readable`/`writable` listos.
(function() {
    if (globalThis.navigator == null) globalThis.navigator = {};
    var nav = globalThis.navigator;
    if (nav.serial != null) return;

    var pendingRequest = null;
    var grantedPorts = [];

    function SerialPort(info) {
        info = info || {};
        this._info = {
            usbVendorId: info.usbVendorId,
            usbProductId: info.usbProductId
        };
        this.readable = null;
        this.writable = null;
        this.connected = true;
        this._open = false;
        this._listeners = {};
    }
    SerialPort.prototype.getInfo = function() {
        return { usbVendorId: this._info.usbVendorId, usbProductId: this._info.usbProductId };
    };
    SerialPort.prototype.open = function(options) {
        if (this._open) {
            return Promise.reject(new globalThis.DOMException(
                'puerto ya abierto', 'InvalidStateError'));
        }
        options = options || {};
        if (typeof options.baudRate !== 'number') {
            return Promise.reject(new globalThis.DOMException(
                'baudRate requerido', 'TypeError'));
        }
        this._open = true;
        this.readable = (typeof globalThis.ReadableStream === 'function')
            ? new globalThis.ReadableStream() : {};
        this.writable = (typeof globalThis.WritableStream === 'function')
            ? new globalThis.WritableStream() : {};
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'serial-open', value: String(options.baudRate)
        });
        return Promise.resolve();
    };
    SerialPort.prototype.close = function() {
        this._open = false;
        this.readable = null;
        this.writable = null;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'serial-close', value: '' });
        return Promise.resolve();
    };
    SerialPort.prototype.forget = function() {
        var i = grantedPorts.indexOf(this);
        if (i >= 0) grantedPorts.splice(i, 1);
        return Promise.resolve();
    };
    SerialPort.prototype.addEventListener = function(type, fn) {
        (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    SerialPort.prototype.removeEventListener = function(type, fn) {
        var a = this._listeners[type]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    SerialPort.prototype.dispatchEvent = function(ev) {
        var a = this._listeners[ev.type];
        if (a) { for (var i = 0; i < a.length; i++) a[i].call(this, ev); }
        if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
        return true;
    };

    function Serial() { this._listeners = {}; }
    Serial.prototype.addEventListener = function(type, fn) {
        (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    Serial.prototype.removeEventListener = function(type, fn) {
        var a = this._listeners[type]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    Serial.prototype.dispatchEvent = function(ev) {
        var a = this._listeners[ev.type];
        if (a) { for (var i = 0; i < a.length; i++) a[i].call(this, ev); }
        if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
        return true;
    };
    Serial.prototype.getPorts = function() {
        return Promise.resolve(grantedPorts.slice());
    };
    Serial.prototype.requestPort = function(options) {
        if (pendingRequest) {
            return Promise.reject(new globalThis.DOMException(
                'requestPort ya en curso', 'InvalidStateError'));
        }
        return new Promise(function(resolve, reject) {
            pendingRequest = { resolve: resolve, reject: reject };
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'serial-request',
                value: JSON.stringify((options && options.filters) || [])
            });
        });
    };

    var serial = new Serial();

    // El host concede un puerto tras el picker. info: { usbVendorId, usbProductId }.
    globalThis.__puriy_serial_resolve = function(info) {
        if (!pendingRequest) return false;
        var p = pendingRequest; pendingRequest = null;
        var port = new SerialPort(info);
        if (grantedPorts.indexOf(port) < 0) grantedPorts.push(port);
        p.resolve(port);
        return true;
    };
    // Cancela el picker. Sin selección, name === 'NotFoundError'.
    globalThis.__puriy_serial_reject = function(name) {
        if (!pendingRequest) return false;
        var p = pendingRequest; pendingRequest = null;
        p.reject(new globalThis.DOMException(
            'requestPort cancelado', name || 'NotFoundError'));
        return true;
    };
    // El host registra un puerto ya concedido (visible en getPorts).
    globalThis.__puriy_serial_add_port = function(info) {
        var port = new SerialPort(info);
        grantedPorts.push(port);
        return port;
    };
    // Eventos de conexión/desconexión física.
    globalThis.__puriy_serial_connect = function(port) {
        serial.dispatchEvent({ type: 'connect', target: port || null });
        return true;
    };
    globalThis.__puriy_serial_disconnect = function(port) {
        if (port) port.connected = false;
        serial.dispatchEvent({ type: 'disconnect', target: port || null });
        return true;
    };

    nav.serial = serial;
    globalThis.Serial = Serial;
    globalThis.SerialPort = SerialPort;
    void 0;
})();
"#;
