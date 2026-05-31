pub(crate) const SENSORS_BOOTSTRAP: &str = r#"
// Fase 7.132 — Generic Sensor API (`Accelerometer`, `Gyroscope`, `Magnetometer`,
// `AmbientLightSensor`, `AbsoluteOrientationSensor`/`RelativeOrientationSensor`,
// `LinearAccelerationSensor`, `GravitySensor`). Hermana de DeviceOrientation
// (Fase 7.111) pero con el modelo moderno: cada sensor es una clase con
// `start()`/`stop()`, flags `activated`/`hasReading`, `timestamp`, y eventos
// `reading`/`activate`/`error`. El motor no tiene sensores reales: `start()` es
// host-driven (publica kind: 'sensor-start' value `<type>`) y el chrome inyecta
// lecturas con `__puriy_sensor_reading(type, data)` o errores con
// `__puriy_sensor_error(type, name, message)`. Un sensor sólo recibe lecturas
// mientras está corriendo (en la lista `active`).
(function() {
    if (globalThis.Sensor != null) return;
    var active = [];  // sensores en estado activado

    function mix(proto) {
        proto.addEventListener = function(type, fn) {
            (this._listeners[type] = this._listeners[type] || []).push(fn);
        };
        proto.removeEventListener = function(type, fn) {
            var a = this._listeners[type]; if (!a) return;
            var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
        };
        proto.dispatchEvent = function(ev) {
            var a = this._listeners[ev.type];
            if (a) { var c = a.slice(); for (var i = 0; i < c.length; i++) c[i].call(this, ev); }
            if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
            return true;
        };
    }

    function Sensor(options) {
        this._listeners = {};
        this.activated = false;
        this.hasReading = false;
        this.timestamp = null;
        this._type = 'sensor';
        this.frequency = (options && options.frequency != null) ? options.frequency : null;
    }
    mix(Sensor.prototype);
    Sensor.prototype.start = function() {
        if (this.activated) return;
        this.activated = true;
        if (active.indexOf(this) === -1) active.push(this);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'sensor-start', value: this._type });
        this.dispatchEvent({ type: 'activate' });
    };
    Sensor.prototype.stop = function() {
        if (!this.activated) return;
        this.activated = false;
        var i = active.indexOf(this); if (i >= 0) active.splice(i, 1);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'sensor-stop', value: this._type });
    };
    globalThis.Sensor = Sensor;

    function subclass(name, type, apply) {
        function Ctor(options) { Sensor.call(this, options); this._type = type; }
        Ctor.prototype = Object.create(Sensor.prototype);
        Ctor.prototype.constructor = Ctor;
        Ctor.prototype._apply = apply;
        globalThis[name] = Ctor;
    }
    function xyz(d) { this.x = d.x != null ? d.x : null; this.y = d.y != null ? d.y : null; this.z = d.z != null ? d.z : null; }
    function quat(d) { this.quaternion = d.quaternion != null ? d.quaternion : null; }

    subclass('Accelerometer', 'accelerometer', xyz);
    subclass('LinearAccelerationSensor', 'linear-acceleration', xyz);
    subclass('GravitySensor', 'gravity', xyz);
    subclass('Gyroscope', 'gyroscope', xyz);
    subclass('Magnetometer', 'magnetometer', xyz);
    subclass('AmbientLightSensor', 'ambient-light', function(d) {
        this.illuminance = d.illuminance != null ? d.illuminance : null;
    });
    subclass('AbsoluteOrientationSensor', 'absolute-orientation', quat);
    subclass('RelativeOrientationSensor', 'relative-orientation', quat);

    // El host empuja una lectura a todos los sensores activos de ese tipo.
    globalThis.__puriy_sensor_reading = function(type, data) {
        data = data || {};
        var any = false;
        for (var i = 0; i < active.length; i++) {
            var s = active[i];
            if (s._type !== type) continue;
            if (typeof s._apply === 'function') s._apply.call(s, data);
            s.hasReading = true;
            s.timestamp = data.timestamp != null ? data.timestamp : 0;
            s.dispatchEvent({ type: 'reading' });
            any = true;
        }
        return any;
    };
    // El host señala un error en los sensores activos de ese tipo.
    globalThis.__puriy_sensor_error = function(type, name, message) {
        var any = false;
        for (var i = 0; i < active.length; i++) {
            var s = active[i];
            if (s._type !== type) continue;
            s.dispatchEvent({
                type: 'error',
                error: new globalThis.DOMException(message || '', name || 'NotReadableError')
            });
            any = true;
        }
        return any;
    };
    void 0;
})();
"#;
