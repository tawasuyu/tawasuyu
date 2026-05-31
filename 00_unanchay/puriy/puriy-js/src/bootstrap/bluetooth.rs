pub(crate) const BLUETOOTH_BOOTSTRAP: &str = r#"
// Fase 7.125 — Web Bluetooth API (`navigator.bluetooth`). Apps de IoT, wearables y
// periféricos BLE la usan para hablar con dispositivos Bluetooth Low Energy. Continúa la
// familia device-access (serial 7.120 / hid 7.121 / usb 7.122): picker host-driven, permiso
// por defecto denegado, rechazo `NotFoundError` cuando el usuario no elige nada, dispositivos
// ya concedidos vía `getDevices()`, y mutaciones publicadas al chrome para cada acción.
// `requestDevice({filters|acceptAllDevices})` publica `kind: 'bluetooth-request'` y devuelve
// `Promise<BluetoothDevice>` resuelta por `__puriy_bluetooth_resolve(info)` o cancelada con
// `__puriy_bluetooth_reject(name)`. `getAvailability()` → host-decided
// (`__puriy_set_bluetooth_availability`, default true). El `BluetoothRemoteGATTServer`
// (`device.gatt`) publica `bluetooth-gatt-connect`/`-disconnect`; `connect()` queda pendiente
// hasta `__puriy_bluetooth_gatt_resolve(deviceId)`. Eventos `gattserverdisconnected` en el
// device + `availabilitychanged` en el namespace.
(function() {
    if (globalThis.navigator == null) globalThis.navigator = {};
    var nav = globalThis.navigator;
    if (nav.bluetooth != null) return;

    var pendingRequest = null;
    var grantedDevices = [];
    var available = true;
    var deviceSeq = 1;
    var gattPending = {};

    function BluetoothRemoteGATTServer(device) {
        this.device = device;
        this.connected = false;
    }
    BluetoothRemoteGATTServer.prototype.connect = function() {
        var self = this;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'bluetooth-gatt-connect', value: String(self.device.id)
        });
        return new Promise(function(resolve, reject) {
            gattPending[self.device.id] = { resolve: resolve, reject: reject, server: self };
        });
    };
    BluetoothRemoteGATTServer.prototype.disconnect = function() {
        this.connected = false;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'bluetooth-gatt-disconnect', value: String(this.device.id)
        });
        this.device.dispatchEvent({ type: 'gattserverdisconnected', target: this.device });
    };

    function BluetoothDevice(info) {
        info = info || {};
        this.id = (info.id != null) ? String(info.id) : ('bt-' + (deviceSeq++));
        this.name = (info.name != null) ? String(info.name) : undefined;
        this.gatt = new BluetoothRemoteGATTServer(this);
        this._listeners = {};
    }
    BluetoothDevice.prototype.addEventListener = function(type, fn) {
        (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    BluetoothDevice.prototype.removeEventListener = function(type, fn) {
        var a = this._listeners[type]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    BluetoothDevice.prototype.dispatchEvent = function(ev) {
        var a = this._listeners[ev.type];
        if (a) { for (var i = 0; i < a.length; i++) a[i].call(this, ev); }
        if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
        return true;
    };

    function Bluetooth() { this._listeners = {}; }
    Bluetooth.prototype.addEventListener = function(type, fn) {
        (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    Bluetooth.prototype.removeEventListener = function(type, fn) {
        var a = this._listeners[type]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    Bluetooth.prototype.dispatchEvent = function(ev) {
        var a = this._listeners[ev.type];
        if (a) { for (var i = 0; i < a.length; i++) a[i].call(this, ev); }
        if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
        return true;
    };
    Bluetooth.prototype.getAvailability = function() {
        return Promise.resolve(available);
    };
    Bluetooth.prototype.getDevices = function() {
        return Promise.resolve(grantedDevices.slice());
    };
    Bluetooth.prototype.requestDevice = function(options) {
        if (pendingRequest) {
            return Promise.reject(new globalThis.DOMException(
                'requestDevice ya en curso', 'InvalidStateError'));
        }
        options = options || {};
        if (!options.acceptAllDevices && (!options.filters || options.filters.length === 0)) {
            return Promise.reject(new globalThis.DOMException(
                'requestDevice requiere filters o acceptAllDevices', 'TypeError'));
        }
        return new Promise(function(resolve, reject) {
            pendingRequest = { resolve: resolve, reject: reject };
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'bluetooth-request',
                value: JSON.stringify(options.filters || (options.acceptAllDevices ? 'all' : []))
            });
        });
    };

    var bluetooth = new Bluetooth();

    // El host concede un dispositivo tras el picker. info: { id?, name? }.
    globalThis.__puriy_bluetooth_resolve = function(info) {
        if (!pendingRequest) return false;
        var p = pendingRequest; pendingRequest = null;
        var dev = new BluetoothDevice(info);
        if (grantedDevices.indexOf(dev) < 0) grantedDevices.push(dev);
        p.resolve(dev);
        return true;
    };
    // Cancela el picker. Sin selección, name === 'NotFoundError'.
    globalThis.__puriy_bluetooth_reject = function(name) {
        if (!pendingRequest) return false;
        var p = pendingRequest; pendingRequest = null;
        p.reject(new globalThis.DOMException(
            'requestDevice cancelado', name || 'NotFoundError'));
        return true;
    };
    // El host registra un dispositivo ya concedido (visible en getDevices).
    globalThis.__puriy_bluetooth_add_device = function(info) {
        var dev = new BluetoothDevice(info);
        grantedDevices.push(dev);
        return dev;
    };
    // El chrome confirma la conexión GATT solicitada por device.gatt.connect().
    globalThis.__puriy_bluetooth_gatt_resolve = function(deviceId) {
        var key = String(deviceId);
        var pend = gattPending[key];
        if (!pend) return false;
        delete gattPending[key];
        pend.server.connected = true;
        pend.resolve(pend.server);
        return true;
    };
    globalThis.__puriy_bluetooth_gatt_reject = function(deviceId, name) {
        var key = String(deviceId);
        var pend = gattPending[key];
        if (!pend) return false;
        delete gattPending[key];
        pend.reject(new globalThis.DOMException(
            'GATT connect falló', name || 'NetworkError'));
        return true;
    };
    // El host cambia la disponibilidad del adaptador (dispara availabilitychanged).
    globalThis.__puriy_set_bluetooth_availability = function(flag) {
        available = !!flag;
        bluetooth.dispatchEvent({ type: 'availabilitychanged', value: available });
        return true;
    };

    nav.bluetooth = bluetooth;
    globalThis.Bluetooth = Bluetooth;
    globalThis.BluetoothDevice = BluetoothDevice;
    globalThis.BluetoothRemoteGATTServer = BluetoothRemoteGATTServer;
    void 0;
})();
"#;
