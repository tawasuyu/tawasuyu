pub(crate) const DEVICE_ORIENTATION_BOOTSTRAP: &str = r#"
// Fase 7.112 — DeviceOrientation + DeviceMotion (eventos de sensores). Las apps
// (juegos, niveles de burbuja, VR) escuchan `deviceorientation` (alpha/beta/gamma:
// brújula + inclinación) y `devicemotion` (acceleration + rotationRate del
// acelerómetro/giroscopio) sobre window. Host-driven: el chrome empuja lecturas
// con `__puriy_deliver_device_orientation(alpha, beta, gamma, absolute)` y
// `__puriy_deliver_device_motion(accel, accelG, rot, interval)`, que caen por
// `__puriy_dispatch_window` (Fase 7.39) con los campos pegados al evento. En iOS
// estos eventos exigen permiso explícito: replicamos los estáticos
// `DeviceOrientationEvent.requestPermission()` / `DeviceMotionEvent.requestPermission()`
// → Promise<'granted'|'denied'> gated por `__puriy_device_sensor_permission`
// (default 'granted'; el chrome lo cambia con `__puriy_set_device_sensor_permission`).
(function() {
    if (globalThis.DeviceOrientationEvent != null) return;

    globalThis.__puriy_device_sensor_permission =
        globalThis.__puriy_device_sensor_permission || 'granted';

    function DeviceOrientationEvent(type, init) {
        init = init || {};
        this.type = String(type);
        this.alpha = init.alpha != null ? Number(init.alpha) : null;
        this.beta = init.beta != null ? Number(init.beta) : null;
        this.gamma = init.gamma != null ? Number(init.gamma) : null;
        this.absolute = !!init.absolute;
        this.target = globalThis;
        this.currentTarget = globalThis;
        this.defaultPrevented = false;
        this.preventDefault = function() { this.defaultPrevented = true; };
        this.stopPropagation = function() {};
    }
    DeviceOrientationEvent.requestPermission = function() {
        return Promise.resolve(globalThis.__puriy_device_sensor_permission);
    };
    globalThis.DeviceOrientationEvent = DeviceOrientationEvent;

    function DeviceMotionEvent(type, init) {
        init = init || {};
        this.type = String(type);
        this.acceleration = init.acceleration || null;
        this.accelerationIncludingGravity = init.accelerationIncludingGravity || null;
        this.rotationRate = init.rotationRate || null;
        this.interval = init.interval != null ? Number(init.interval) : 0;
        this.target = globalThis;
        this.currentTarget = globalThis;
        this.defaultPrevented = false;
        this.preventDefault = function() { this.defaultPrevented = true; };
        this.stopPropagation = function() {};
    }
    DeviceMotionEvent.requestPermission = function() {
        return Promise.resolve(globalThis.__puriy_device_sensor_permission);
    };
    globalThis.DeviceMotionEvent = DeviceMotionEvent;

    globalThis.__puriy_deliver_device_orientation = function(alpha, beta, gamma, absolute) {
        globalThis.__puriy_dispatch_window('deviceorientation', {
            alpha: alpha != null ? Number(alpha) : null,
            beta: beta != null ? Number(beta) : null,
            gamma: gamma != null ? Number(gamma) : null,
            absolute: !!absolute
        });
        return true;
    };
    globalThis.__puriy_deliver_device_motion = function(accel, accelG, rot, interval) {
        globalThis.__puriy_dispatch_window('devicemotion', {
            acceleration: accel || null,
            accelerationIncludingGravity: accelG || null,
            rotationRate: rot || null,
            interval: interval != null ? Number(interval) : 0
        });
        return true;
    };
    globalThis.__puriy_set_device_sensor_permission = function(state) {
        globalThis.__puriy_device_sensor_permission = String(state);
        return true;
    };
})();
"#;
