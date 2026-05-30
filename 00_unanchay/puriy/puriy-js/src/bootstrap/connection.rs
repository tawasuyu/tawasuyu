pub(crate) const CONNECTION_BOOTSTRAP: &str = r#"
// Fase 7.89 — `navigator.connection` (NetworkInformation API). Cierra el trío
// de net-awareness con `navigator.onLine` + eventos online/offline (Fase 7.86):
// las libs leen `connection.effectiveType`/`saveData` para degradar contenido
// en redes lentas o con ahorro de datos, y escuchan `change` para reaccionar.
//
// `NetworkInformation` hereda de `EventTarget` (Fase 7.76) vía cadena de
// prototipos real (`instanceof EventTarget` se cumple). Valores por defecto:
// red rápida desconocida (4g, 10 Mbps, 50 ms). El chrome los actualiza con el
// hook de ingreso `__puriy_set_connection(props)`, que dispara `change`.
(function() {
    function NetworkInformation() {
        globalThis.EventTarget.call(this);
        this.effectiveType = '4g';      // 'slow-2g' | '2g' | '3g' | '4g'
        this.type = 'unknown';          // 'wifi' | 'cellular' | 'ethernet' | ...
        this.downlink = 10;             // Mbps estimados
        this.downlinkMax = Infinity;    // Mbps máximos del enlace subyacente
        this.rtt = 50;                  // round-trip estimado en ms
        this.saveData = false;          // modo ahorro de datos del usuario
        this.onchange = null;
    }
    NetworkInformation.prototype = Object.create(globalThis.EventTarget.prototype);
    NetworkInformation.prototype.constructor = NetworkInformation;
    globalThis.NetworkInformation = NetworkInformation;

    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.connection == null) {
        nav.connection = new NetworkInformation();
    }
    // Aliases con prefijo de vendor que el código viejo chequea a ciegas.
    if (nav.mozConnection == null) nav.mozConnection = nav.connection;
    if (nav.webkitConnection == null) nav.webkitConnection = nav.connection;
})();
// Hook de ingreso: el chrome llama esto cuando cambian las características de
// la red. Actualiza los campos presentes en `props` y dispara `change` sobre
// `navigator.connection` (handler `onchange` + addEventListener('change')).
globalThis.__puriy_set_connection = function(props) {
    var c = globalThis.navigator && globalThis.navigator.connection;
    if (!c) return false;
    if (props && typeof props === 'object') {
        if (props.effectiveType != null) c.effectiveType = String(props.effectiveType);
        if (props.type != null) c.type = String(props.type);
        if (props.downlink != null) c.downlink = +props.downlink;
        if (props.downlinkMax != null) c.downlinkMax = +props.downlinkMax;
        if (props.rtt != null) c.rtt = +props.rtt;
        if (props.saveData != null) c.saveData = !!props.saveData;
    }
    var ev = new globalThis.Event('change');
    if (typeof c.onchange === 'function') {
        try { c.onchange.call(c, ev); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    c.dispatchEvent(ev);
    return true;
};
"#;
