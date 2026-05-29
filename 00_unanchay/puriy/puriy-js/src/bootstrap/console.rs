pub(crate) const CONSOLE_BOOTSTRAP: &str = r#"
globalThis.__puriy_stdout = '';
globalThis.__puriy_stderr = '';
(function() {
    function fmt(args) {
        var parts = [];
        for (var i = 0; i < args.length; i++) {
            var v = args[i];
            if (v === null) parts.push('null');
            else if (v === undefined) parts.push('undefined');
            else if (typeof v === 'object') {
                try { parts.push(JSON.stringify(v)); }
                catch (_e) { parts.push(String(v)); }
            }
            else parts.push(String(v));
        }
        return parts.join(' ');
    }
    // Fase 7.27 — state para console.group/groupEnd: indent prefix
    // que se prepende a cada línea del stdout/stderr mientras un grupo
    // está abierto. Nesteable. console.assert/count/time/trace también
    // viven acá.
    var groupIndent = '';
    var counters = {};
    var timers = {};
    function writeOut(s) { globalThis.__puriy_stdout += groupIndent + s + '\n'; }
    function writeErr(s) { globalThis.__puriy_stderr += groupIndent + s + '\n'; }
    globalThis.console = {
        log: function() { writeOut(fmt(arguments)); },
        info: function() { writeOut(fmt(arguments)); },
        debug: function() { writeOut(fmt(arguments)); },
        error: function() { writeErr(fmt(arguments)); },
        warn: function() { writeErr(fmt(arguments)); },
        // Fase 7.27 — group/groupCollapsed: imprimen el label y aumentan
        // el indent. groupCollapsed es alias (no tenemos UI plegable).
        // groupEnd cierra el grupo más reciente. Nesteable.
        group: function() {
            writeOut(fmt(arguments));
            groupIndent += '  ';
        },
        groupCollapsed: function() {
            writeOut(fmt(arguments));
            groupIndent += '  ';
        },
        groupEnd: function() {
            if (groupIndent.length >= 2) groupIndent = groupIndent.slice(0, -2);
        },
        // Fase 7.27 — assert(cond, ...msg): si cond falsy, imprime
        // "Assertion failed: ..." en stderr. Si cond truthy, no-op
        // silencioso (matchea spec).
        assert: function() {
            if (arguments.length === 0 || arguments[0]) return;
            var rest = Array.prototype.slice.call(arguments, 1);
            writeErr('Assertion failed: ' + fmt(rest));
        },
        // Fase 7.27 — count(label): incrementa el counter y lo imprime
        // como "label: N". Default label 'default' (spec).
        count: function(label) {
            var k = (label == null) ? 'default' : String(label);
            counters[k] = (counters[k] || 0) + 1;
            writeOut(k + ': ' + counters[k]);
        },
        countReset: function(label) {
            var k = (label == null) ? 'default' : String(label);
            counters[k] = 0;
        },
        // Fase 7.27 — time(label)/timeEnd(label): mide tiempo entre
        // ambas calls usando __puriy_now_ms del runtime. Resolución
        // depende del tick del host (~33ms); útil para "rough timing".
        time: function(label) {
            var k = (label == null) ? 'default' : String(label);
            timers[k] = globalThis.__puriy_now_ms || 0;
        },
        timeEnd: function(label) {
            var k = (label == null) ? 'default' : String(label);
            if (timers[k] == null) {
                writeErr("Timer '" + k + "' does not exist");
                return;
            }
            var dt = (globalThis.__puriy_now_ms || 0) - timers[k];
            delete timers[k];
            writeOut(k + ': ' + dt + 'ms');
        },
        // Fase 7.27 — trace: equivalente a console.log + indent state.
        // No emitimos stack porque QuickJS no lo expone de forma estándar.
        trace: function() {
            writeOut('Trace: ' + fmt(arguments));
        },
        // Fase 7.27 — dir(obj): muestra la representación profunda del
        // objeto. Sin colorización ni expansion interactiva — texto
        // plano con JSON.stringify cuando podemos.
        dir: function(obj) {
            try { writeOut(JSON.stringify(obj, null, 2)); }
            catch (_e) { writeOut(String(obj)); }
        },
        // Fase 7.27 — table(data): render minimalista de una table. Si
        // data es array de objetos, muestra "[i] {k1: v1, k2: v2}".
        // Si es array de primitivos, "[i] v". Si es objeto, "k: v".
        // Sin formato ASCII (columnas alineadas) — sólo legible.
        table: function(data) {
            if (data == null) { writeOut(String(data)); return; }
            if (Array.isArray(data)) {
                for (var i = 0; i < data.length; i++) {
                    var row = data[i];
                    if (row !== null && typeof row === 'object') {
                        try { writeOut('[' + i + '] ' + JSON.stringify(row)); }
                        catch (_e) { writeOut('[' + i + '] ' + String(row)); }
                    } else {
                        writeOut('[' + i + '] ' + String(row));
                    }
                }
            } else if (typeof data === 'object') {
                for (var k in data) {
                    if (Object.prototype.hasOwnProperty.call(data, k)) {
                        writeOut(k + ': ' + String(data[k]));
                    }
                }
            } else {
                writeOut(String(data));
            }
        }
    };
})();
"#;
