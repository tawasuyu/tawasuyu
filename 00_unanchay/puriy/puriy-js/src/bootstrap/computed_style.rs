pub(crate) const COMPUTED_STYLE_BOOTSTRAP: &str = r#"
// Fase 7.30 — getComputedStyle(el) stub. Spec: devuelve un
// CSSStyleDeclaration con todos los propiedades computadas post-cascade
// del CSS (UA stylesheet + author rules + inline). Acá no tenemos
// acceso al style engine del chrome desde el runtime, así que devolvemos
// SÓLO los estilos inline que el JS mismo seteó via `el.style.X = Y`
// (almacenados en `el._style_store` con keys kebab). Suficiente para
// scripts que leen lo que setearon antes; insuficiente para "leer
// computed color heredado del padre via CSS".
globalThis.getComputedStyle = function(el) {
    if (!el) {
        return {
            getPropertyValue: function() { return ''; },
            length: 0
        };
    }
    var store = el._style_store || {};
    return {
        getPropertyValue: function(prop) {
            if (typeof prop !== 'string') return '';
            // CSS spec: keys son case-insensitive y se normalizan kebab.
            var k = prop.toLowerCase();
            return store[k] != null ? store[k] : '';
        },
        // Acceso por property camelCase (`.color`, `.fontSize`) — itera
        // el store y matchea convirtiendo a kebab. Sirve para los lookups
        // más comunes; lookups complejos requieren getPropertyValue.
        get color() { return store['color'] || ''; },
        get backgroundColor() { return store['background-color'] || ''; },
        get fontSize() { return store['font-size'] || ''; },
        get fontWeight() { return store['font-weight'] || ''; },
        get display() { return store['display'] || ''; },
        get visibility() { return store['visibility'] || ''; },
        get width() { return store['width'] || ''; },
        get height() { return store['height'] || ''; },
        get opacity() { return store['opacity'] || ''; },
        get length() {
            return Object.keys(store).length;
        }
    };
};
globalThis.__puriy_dispatch_event = function(id, event) {
    var target = globalThis.__puriy_elements[id];
    if (!target) return false;
    event.target = target;
    event.currentTarget = target;
    event._stopped = false;
    var chain = [target];
    if (event.bubbles) {
        var visited = {}; visited[target.id] = true;
        var cur = target; var depth = 0;
        while (cur && cur._parent_id && depth < 64) {
            var next = globalThis.__puriy_elements[cur._parent_id];
            if (!next || visited[next.id]) break;
            visited[next.id] = true;
            chain.push(next);
            cur = next;
            depth++;
        }
    }
    var type = event.type;
    function invoke(node, store) {
        var ls = store && store[type];
        if (!ls) return;
        var arr2 = ls.slice();
        var to_remove = [];
        for (var i = 0; i < arr2.length; i++) {
            var entry = arr2[i];
            var fn = typeof entry === 'function' ? entry : entry.fn;
            var once = typeof entry === 'object' && entry.once === true;
            try { fn.call(node, event); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
            if (once) to_remove.push(entry);
            if (event._stopped) break;
        }
        if (to_remove.length > 0) {
            var live = store[type];
            for (var k = 0; k < to_remove.length; k++) {
                var idx = live.indexOf(to_remove[k]);
                if (idx >= 0) live.splice(idx, 1);
            }
        }
    }
    event.eventPhase = 1;
    for (var i = chain.length - 1; i > 0; i--) {
        if (event._stopped) break;
        event.currentTarget = chain[i];
        invoke(chain[i], chain[i]._capture_listeners);
    }
    event.eventPhase = 2;
    event.currentTarget = target;
    if (!event._stopped) invoke(target, target._capture_listeners);
    var onName = 'on' + type;
    if (!event._stopped && typeof target[onName] === 'function') {
        try { target[onName].call(target, event); }
        catch (e3) { globalThis.__puriy_stderr += String(e3) + '\n'; }
    }
    if (!event._stopped) invoke(target, target._listeners);
    event.eventPhase = 3;
    for (var j = 1; j < chain.length; j++) {
        if (event._stopped) break;
        event.currentTarget = chain[j];
        if (typeof chain[j][onName] === 'function') {
            try { chain[j][onName].call(chain[j], event); }
            catch (e2) { globalThis.__puriy_stderr += String(e2) + '\n'; }
        }
        invoke(chain[j], chain[j]._listeners);
    }
    return !event.defaultPrevented;
};
"#;
