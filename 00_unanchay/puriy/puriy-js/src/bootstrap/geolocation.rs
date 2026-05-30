pub(crate) const GEOLOCATION_BOOTSTRAP: &str = r#"
// Fase 7.95 — `navigator.geolocation`. `getCurrentPosition(success, error,
// opts)` (one-shot) + `watchPosition(...)` → id + `clearWatch(id)`. El motor
// no tiene GPS: cada pedido se publica al chrome por el canal `__puriy_dirty`
// (`kind: 'geolocation'`, value `once:<id>` o `watch:<id>`) y el chrome entrega
// la posición con los hooks `__puriy_deliver_position` /
// `__puriy_deliver_position_error` (wiring nativo pendiente). Mismo patrón
// fire-and-wire que websocket/eventsource.
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.geolocation != null) return;

    globalThis.__puriy_geo_watchers = globalThis.__puriy_geo_watchers || {};   // id -> {success, error}
    globalThis.__puriy_geo_next_id = globalThis.__puriy_geo_next_id || 1;
    var oneShots = {};   // id -> {success, error} de getCurrentPosition pendientes

    function makePosition(coords) {
        coords = coords || {};
        function num(v, dflt) { return (v != null) ? +v : dflt; }
        return {
            coords: {
                latitude: num(coords.latitude, 0),
                longitude: num(coords.longitude, 0),
                accuracy: num(coords.accuracy, 0),
                altitude: num(coords.altitude, null),
                altitudeAccuracy: num(coords.altitudeAccuracy, null),
                heading: num(coords.heading, null),
                speed: num(coords.speed, null)
            },
            timestamp: num(coords.timestamp, 0)
        };
    }

    function Geolocation() {}
    Geolocation.prototype.getCurrentPosition = function(success, error) {
        var id = globalThis.__puriy_geo_next_id++;
        oneShots[id] = { success: success, error: error };
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'geolocation', value: 'once:' + id });
    };
    Geolocation.prototype.watchPosition = function(success, error) {
        var id = globalThis.__puriy_geo_next_id++;
        globalThis.__puriy_geo_watchers[id] = { success: success, error: error };
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'geolocation', value: 'watch:' + id });
        return id;
    };
    Geolocation.prototype.clearWatch = function(id) {
        delete globalThis.__puriy_geo_watchers[id];
    };
    nav.geolocation = new Geolocation();

    // Entrega una posición. one-shot → llama success y lo consume; watch activo
    // → llama success sin consumirlo. Devuelve si encontró a quién entregar.
    globalThis.__puriy_deliver_position = function(id, coords) {
        var pos = makePosition(coords);
        if (oneShots[id]) {
            var cb = oneShots[id].success;
            delete oneShots[id];
            if (typeof cb === 'function') {
                try { cb(pos); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
            }
            return true;
        }
        var w = globalThis.__puriy_geo_watchers[id];
        if (w && typeof w.success === 'function') {
            try { w.success(pos); } catch (e2) { globalThis.__puriy_stderr += String(e2) + '\n'; }
            return true;
        }
        return false;
    };
    // Entrega un error (GeolocationPositionError-like): PERMISSION_DENIED=1,
    // POSITION_UNAVAILABLE=2, TIMEOUT=3.
    globalThis.__puriy_deliver_position_error = function(id, code, message) {
        var err = {
            code: (code != null) ? (code | 0) : 2,
            message: (message != null) ? String(message) : '',
            PERMISSION_DENIED: 1, POSITION_UNAVAILABLE: 2, TIMEOUT: 3
        };
        var target = null;
        if (oneShots[id]) { target = oneShots[id]; delete oneShots[id]; }
        else if (globalThis.__puriy_geo_watchers[id]) { target = globalThis.__puriy_geo_watchers[id]; }
        if (target && typeof target.error === 'function') {
            try { target.error(err); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
            return true;
        }
        return false;
    };
})();
"#;
