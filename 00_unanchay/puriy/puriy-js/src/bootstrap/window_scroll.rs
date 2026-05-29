pub(crate) const WINDOW_SCROLL_BOOTSTRAP: &str = r#"
// Fase 7.26 — Window scroll APIs. Mirror local del scroll position: el
// JS sólo lee lo que el JS mismo seteó. El wheel/keys del usuario que
// mueven scroll_y del chrome NO se reflejan acá (gap honesto — sync
// inverso requeriría que el chrome publique `__puriy_scroll_y` al
// runtime antes de cada eval/tick, lo cual agregaría I/O en el hot
// path). Para apps que necesiten leer el scroll "real", la solución
// futura es agregar `JsRuntime::set_scroll(x, y)` y llamarla desde el
// chrome en cada Msg::Scroll y antes de cada eval.
globalThis.__puriy_scroll_x = 0;
globalThis.__puriy_scroll_y = 0;
// Fase 7.28 — viewport dims. Default 1024×768 — el chrome los
// sobreescribe con set_viewport(w, h) post-Loaded. Valores razonables
// para que código que checkea innerWidth al startup tenga algo válido.
globalThis.__puriy_inner_width = 1024;
globalThis.__puriy_inner_height = 768;
Object.defineProperty(globalThis, 'innerWidth', {
    get: function() { return globalThis.__puriy_inner_width; },
    configurable: true
});
Object.defineProperty(globalThis, 'innerHeight', {
    get: function() { return globalThis.__puriy_inner_height; },
    configurable: true
});
// Legacy aliases — outerWidth/outerHeight son spec-distintos (incluyen
// chrome UI) pero el modelo headless no distingue. Devolver lo mismo.
Object.defineProperty(globalThis, 'outerWidth', {
    get: function() { return globalThis.__puriy_inner_width; },
    configurable: true
});
Object.defineProperty(globalThis, 'outerHeight', {
    get: function() { return globalThis.__puriy_inner_height; },
    configurable: true
});
Object.defineProperty(globalThis, 'scrollX', {
    get: function() { return globalThis.__puriy_scroll_x; },
    configurable: true
});
Object.defineProperty(globalThis, 'scrollY', {
    get: function() { return globalThis.__puriy_scroll_y; },
    configurable: true
});
Object.defineProperty(globalThis, 'pageXOffset', {
    get: function() { return globalThis.__puriy_scroll_x; },
    configurable: true
});
Object.defineProperty(globalThis, 'pageYOffset', {
    get: function() { return globalThis.__puriy_scroll_y; },
    configurable: true
});
globalThis.scrollTo = function(x, y) {
    if (typeof x === 'object' && x !== null) {
        y = x.top;
        x = x.left;
    }
    globalThis.__puriy_scroll_x = Number(x) || 0;
    globalThis.__puriy_scroll_y = Number(y) || 0;
    globalThis.__puriy_dirty.push({
        id: '__window__',
        kind: 'scroll',
        value: globalThis.__puriy_scroll_x + ',' + globalThis.__puriy_scroll_y
    });
};
globalThis.scroll = globalThis.scrollTo;
globalThis.scrollBy = function(dx, dy) {
    if (typeof dx === 'object' && dx !== null) {
        dy = dx.top;
        dx = dx.left;
    }
    globalThis.scrollTo(
        globalThis.__puriy_scroll_x + (Number(dx) || 0),
        globalThis.__puriy_scroll_y + (Number(dy) || 0)
    );
};
"#;
