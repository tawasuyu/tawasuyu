pub(crate) const MIDI_BOOTSTRAP: &str = r#"
// Fase 7.119 — Web MIDI API (`navigator.requestMIDIAccess`). Apps musicales la usan para
// hablar con teclados/controladores MIDI. El motor no tiene hardware MIDI: el acceso es
// host-driven. Permiso por defecto: denegado; el chrome lo concede vía
// `__puriy_midi_grant()`. `requestMIDIAccess(opts)` rechaza `NotAllowedError` sin permiso
// y, concedido, resuelve un `MIDIAccess` con mapas `inputs`/`outputs` (Map de
// `MIDIInput`/`MIDIOutput`, ambos extienden `MIDIPort`). El host puebla puertos vía
// `__puriy_midi_add_port(info, dir)`, emite mensajes con `__puriy_midi_message(id, data)`
// y cambios de estado con `__puriy_midi_statechange(id, state)`. `MIDIOutput.send()`
// publica una mutación `kind: 'midi-send'` al chrome.
(function() {
    if (globalThis.navigator == null) globalThis.navigator = {};
    var nav = globalThis.navigator;
    if (typeof nav.requestMIDIAccess === 'function') return;

    globalThis.__puriy_midi_permission = globalThis.__puriy_midi_permission || 'denied';
    globalThis.__puriy_midi_access = null;

    function MIDIPort(info, type) {
        info = info || {};
        this.id = info.id || ('' + (info.vendorId || 0) + '-' + (info.productId || 0));
        this.manufacturer = info.manufacturer || '';
        this.name = info.name || '';
        this.version = info.version || '';
        this.type = type;
        this.state = 'connected';
        this.connection = 'closed';
        this._listeners = {};
    }
    MIDIPort.prototype.open = function() { this.connection = 'open'; return Promise.resolve(this); };
    MIDIPort.prototype.close = function() { this.connection = 'closed'; return Promise.resolve(this); };
    MIDIPort.prototype.addEventListener = function(type, fn) {
        (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    MIDIPort.prototype.removeEventListener = function(type, fn) {
        var a = this._listeners[type]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    MIDIPort.prototype.dispatchEvent = function(ev) {
        var a = this._listeners[ev.type];
        if (a) { for (var i = 0; i < a.length; i++) a[i].call(this, ev); }
        if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
        return true;
    };

    function MIDIInput(info) { MIDIPort.call(this, info, 'input'); }
    MIDIInput.prototype = Object.create(MIDIPort.prototype);
    MIDIInput.prototype.constructor = MIDIInput;

    function MIDIOutput(info) { MIDIPort.call(this, info, 'output'); }
    MIDIOutput.prototype = Object.create(MIDIPort.prototype);
    MIDIOutput.prototype.constructor = MIDIOutput;
    MIDIOutput.prototype.send = function(data, timestamp) {
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'midi-send',
            value: this.id + ':' + Array.prototype.slice.call(data || []).join(',')
        });
    };
    MIDIOutput.prototype.clear = function() {};

    function MIDIAccess(sysex) {
        this.inputs = new Map();
        this.outputs = new Map();
        this.sysexEnabled = !!sysex;
        this._listeners = {};
    }
    MIDIAccess.prototype.addEventListener = function(type, fn) {
        (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    MIDIAccess.prototype.removeEventListener = function(type, fn) {
        var a = this._listeners[type]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    MIDIAccess.prototype.dispatchEvent = function(ev) {
        var a = this._listeners[ev.type];
        if (a) { for (var i = 0; i < a.length; i++) a[i].call(this, ev); }
        if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
        return true;
    };

    nav.requestMIDIAccess = function(options) {
        if (globalThis.__puriy_midi_permission !== 'granted') {
            return Promise.reject(new globalThis.DOMException(
                'requestMIDIAccess sin permiso', 'NotAllowedError'));
        }
        var access = new MIDIAccess(!!(options && options.sysex));
        globalThis.__puriy_midi_access = access;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'midi-request', value: String(access.sysexEnabled)
        });
        return Promise.resolve(access);
    };

    globalThis.__puriy_midi_grant = function() { globalThis.__puriy_midi_permission = 'granted'; };
    globalThis.__puriy_midi_deny = function() { globalThis.__puriy_midi_permission = 'denied'; };

    // El host añade un puerto al MIDIAccess vigente. dir: 'input' | 'output'.
    globalThis.__puriy_midi_add_port = function(info, dir) {
        var access = globalThis.__puriy_midi_access; if (!access) return false;
        var port = (dir === 'output') ? new MIDIOutput(info) : new MIDIInput(info);
        if (dir === 'output') access.outputs.set(port.id, port);
        else access.inputs.set(port.id, port);
        access.dispatchEvent({ type: 'statechange', port: port });
        return true;
    };

    globalThis.__puriy_midi_statechange = function(portId, state) {
        var access = globalThis.__puriy_midi_access; if (!access) return false;
        var port = access.inputs.get(portId) || access.outputs.get(portId);
        if (!port) return false;
        port.state = state;
        port.dispatchEvent({ type: 'statechange', port: port });
        access.dispatchEvent({ type: 'statechange', port: port });
        return true;
    };

    globalThis.__puriy_midi_message = function(portId, data) {
        var access = globalThis.__puriy_midi_access; if (!access) return false;
        var port = access.inputs.get(portId); if (!port) return false;
        port.dispatchEvent({ type: 'midimessage', data: data, port: port });
        return true;
    };

    globalThis.MIDIAccess = MIDIAccess;
    globalThis.MIDIPort = MIDIPort;
    globalThis.MIDIInput = MIDIInput;
    globalThis.MIDIOutput = MIDIOutput;
    void 0;
})();
"#;
