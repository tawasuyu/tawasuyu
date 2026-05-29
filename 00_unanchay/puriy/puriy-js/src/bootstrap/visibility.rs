pub(crate) const VISIBILITY_BOOTSTRAP: &str = r#"
// Fase 7.42 — Page Visibility API. document.hidden + document.visibilityState
// ('visible'/'hidden') + 'visibilitychange' event. Apps usan esto para pausar
// videos/polling/animaciones cuando la pestaña pasa a background. El bootstrap
// inicializa todos los tabs como visible; el chrome llama `set_visibility(true)`
// sobre el runtime de la tab que pasa a background y `set_visibility(false)`
// sobre el que pasa a foreground — el helper dispatcha `'visibilitychange'`
// al window (spec lo dispatcha al document pero bubblea; nuestros listeners
// están sobre window).
globalThis.__puriy_set_visibility = function(hidden) {
    var newState = hidden ? 'hidden' : 'visible';
    var prev = globalThis.document && globalThis.document.visibilityState;
    if (globalThis.document) {
        globalThis.document.hidden = !!hidden;
        globalThis.document.visibilityState = newState;
    }
    if (prev !== newState) {
        globalThis.__puriy_dispatch_window('visibilitychange', null);
    }
};
"#;
