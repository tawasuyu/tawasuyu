pub(crate) const URLPATTERN_BOOTSTRAP: &str = r#"
// Fase 7.156 — URLPattern API (`new URLPattern(input, baseURL)` + `.test()` + `.exec()`).
// Matcheo de URLs contra patrones con grupos nombrados — la base de los routers
// modernos (`/users/:id`, `/files/*`, `*.example.com`). JS-puro y **real**: cada
// componente (protocol/username/password/hostname/port/pathname/search/hash) se
// compila a una RegExp con captura por grupo; `:name` captura nombrado, `*` comodín
// indexado, `(regex)` grupo anónimo con regex propia, `{...}?` grupos opcionales,
// el resto literal escapado. No depende del chrome (no hay motor que consultar).
//   · Divergencias: el parseo de un patrón-string es heurístico (split por
//     `://`/`/`/`?`/`#`); el grueso del uso real pasa componentes por objeto o un
//     string canónico, ambos cubiertos. Sin `ignoreCase`/`hasRegExpGroups` todavía.
(function() {
    if (globalThis.URLPattern != null) return;
    var KEYS = ['protocol', 'username', 'password', 'hostname', 'port', 'pathname', 'search', 'hash'];

    function escapeRe(c) { return c.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); }

    // Lee un grupo balanceado (s.charAt(i) === open); devuelve {body, end} (end tras close).
    function readBalanced(s, i, open, close) {
        var depth = 1; i++; var body = '';
        while (i < s.length && depth > 0) {
            var ch = s.charAt(i);
            if (ch === open) { depth++; }
            else if (ch === close) { depth--; if (depth === 0) { i++; break; } }
            body += ch; i++;
        }
        return { body: body, end: i };
    }

    // Compila el cuerpo de un componente a regex; empuja nombres de grupo en `names`.
    function compileBody(pattern, names) {
        var out = '';
        var i = 0;
        var n = pattern.length;
        while (i < n) {
            var c = pattern.charAt(i);
            if (c === ':') {
                i++;
                var name = '';
                while (i < n && /[A-Za-z0-9_]/.test(pattern.charAt(i))) { name += pattern.charAt(i); i++; }
                names.push(name);
                if (pattern.charAt(i) === '(') {
                    var r = readBalanced(pattern, i, '(', ')');
                    out += '(' + r.body + ')'; i = r.end;
                } else {
                    out += '([^/]+?)';
                }
                if (pattern.charAt(i) === '?') { out += '?'; i++; }
            } else if (c === '*') {
                names.push(String(names.length));
                out += '(.*)'; i++;
            } else if (c === '(') {
                var r2 = readBalanced(pattern, i, '(', ')');
                names.push(String(names.length));
                out += '(' + r2.body + ')'; i = r2.end;
            } else if (c === '{') {
                var r3 = readBalanced(pattern, i, '{', '}');
                var subNames = [];
                var sub = compileBody(r3.body, subNames);
                for (var k = 0; k < subNames.length; k++) names.push(subNames[k]);
                i = r3.end;
                var mod = '';
                var nc = pattern.charAt(i);
                if (nc === '?' || nc === '*' || nc === '+') { mod = nc; i++; }
                out += '(?:' + sub + ')' + mod;
            } else {
                out += escapeRe(c); i++;
            }
        }
        return out;
    }

    function compileComponent(pattern) {
        if (pattern == null) pattern = '*';
        pattern = String(pattern);
        var names = [];
        var body = compileBody(pattern, names);
        return { regex: new RegExp('^' + body + '$'), names: names, source: pattern };
    }

    // Parseo heurístico de un patrón-string a componentes.
    function parseStringPattern(str) {
        var c = {};
        for (var i = 0; i < KEYS.length; i++) c[KEYS[i]] = '*';
        var rest = str;
        var h = rest.indexOf('#');
        if (h >= 0) { c.hash = rest.slice(h + 1); rest = rest.slice(0, h); }
        var q = rest.indexOf('?');
        if (q >= 0) { c.search = rest.slice(q + 1); rest = rest.slice(0, q); }
        var pm = rest.match(/^([^/:]+):\/\//);
        if (pm) {
            c.protocol = pm[1];
            rest = rest.slice(pm[0].length);
            var slash = rest.indexOf('/');
            var authority = slash >= 0 ? rest.slice(0, slash) : rest;
            var path = slash >= 0 ? rest.slice(slash) : '';
            var at = authority.indexOf('@');
            if (at >= 0) {
                var ui = authority.slice(0, at);
                authority = authority.slice(at + 1);
                var colon = ui.indexOf(':');
                if (colon >= 0) { c.username = ui.slice(0, colon); c.password = ui.slice(colon + 1); }
                else { c.username = ui; }
            }
            var pc = authority.lastIndexOf(':');
            if (pc >= 0) { c.hostname = authority.slice(0, pc); c.port = authority.slice(pc + 1); }
            else { c.hostname = authority; }
            c.pathname = path !== '' ? path : '*';
        } else {
            c.pathname = rest !== '' ? rest : '*';
        }
        return c;
    }

    function parseInput(input, baseURL) {
        var c = {};
        var k;
        if (typeof input === 'object' && input !== null) {
            for (var i = 0; i < KEYS.length; i++) {
                k = KEYS[i];
                c[k] = input[k] != null ? String(input[k]) : '*';
            }
        } else {
            c = parseStringPattern(String(input != null ? input : '*'));
        }
        // baseURL aporta defaults a los componentes no especificados (heurístico).
        if (baseURL != null) {
            try {
                var b = new globalThis.URL(String(baseURL));
                if (c.protocol === '*') c.protocol = b.protocol.replace(/:$/, '');
                if (c.hostname === '*') c.hostname = b.hostname;
            } catch (e) {}
        }
        return c;
    }

    // Valores reales del input a testear, por componente.
    function inputValues(input, baseURL) {
        if (typeof input === 'object' && input !== null) {
            var v = {};
            for (var i = 0; i < KEYS.length; i++) {
                var k = KEYS[i];
                v[k] = input[k] != null ? String(input[k]) : '';
            }
            return v;
        }
        var str = String(input);
        var u;
        try { u = baseURL != null ? new globalThis.URL(str, baseURL) : new globalThis.URL(str); }
        catch (e) { return null; }
        return {
            protocol: u.protocol.replace(/:$/, ''),
            username: u.username || '',
            password: u.password || '',
            hostname: u.hostname,
            port: u.port || '',
            pathname: u.pathname,
            search: u.search.replace(/^\?/, ''),
            hash: u.hash.replace(/^#/, '')
        };
    }

    function URLPattern(input, baseURL) {
        if (!(this instanceof URLPattern)) { throw new TypeError("URLPattern requiere 'new'"); }
        var comps = parseInput(input, baseURL);
        this._c = {};
        for (var i = 0; i < KEYS.length; i++) {
            var k = KEYS[i];
            this._c[k] = compileComponent(comps[k]);
            this[k] = comps[k];
        }
    }

    URLPattern.prototype.test = function(input, baseURL) {
        return this.exec(input, baseURL) !== null;
    };

    URLPattern.prototype.exec = function(input, baseURL) {
        var vals = inputValues(input, baseURL);
        if (vals == null) return null;
        var result = { inputs: [input] };
        for (var i = 0; i < KEYS.length; i++) {
            var k = KEYS[i];
            var comp = this._c[k];
            var m = comp.regex.exec(vals[k]);
            if (!m) return null;
            var groups = {};
            for (var gi = 0; gi < comp.names.length; gi++) {
                groups[comp.names[gi]] = m[gi + 1];
            }
            result[k] = { input: vals[k], groups: groups };
        }
        return result;
    };

    globalThis.URLPattern = URLPattern;
    void 0;
})();
"#;
