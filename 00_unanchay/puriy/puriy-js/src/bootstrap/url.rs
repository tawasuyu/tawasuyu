pub(crate) const URL_BOOTSTRAP: &str = r#"
// Fase 7.37 — resolver URL relativa contra una base.
// Casos cubiertos:
//   - URL ya absoluta (`scheme:...`)            → tal cual
//   - protocol-relative (`//host/path`)         → scheme de base + url
//   - absoluta de path (`/path`)                → origin de base + url
//   - solo query (`?q=1`)                       → base sin query + url
//   - solo hash (`#h`)                          → base sin hash + url
//   - relativa de path (`foo/bar`, `../x`)      → dir(base) + url, con
//     colapso de segmentos `..`/`.` (Fase 7.46 — antes se dejaban crudos).
// Sin base, devuelve la url tal cual (matchea spec — `new URL(rel)` tira).

// Fase 7.46 — colapsa segmentos `.`/`..` de un path estilo WHATWG.
// Preserva query/hash, el slash inicial, y el slash final cuando el
// último segmento era vacío/`.`/`..` (p.ej. `/a/b/..` → `/a/`).
globalThis.__puriy_normalize_path = function(p) {
    var tail = '';
    var qi = p.search(/[?#]/);
    if (qi >= 0) { tail = p.substring(qi); p = p.substring(0, qi); }
    var leadingSlash = p.charAt(0) === '/';
    var segs = p.split('/');
    var lastSeg = segs[segs.length - 1];
    var wantTrailing = (lastSeg === '' || lastSeg === '.' || lastSeg === '..');
    var out = [];
    for (var i = 0; i < segs.length; i++) {
        var s = segs[i];
        if (s === '' || s === '.') continue;
        if (s === '..') { if (out.length > 0) out.pop(); continue; }
        out.push(s);
    }
    var res = (leadingSlash ? '/' : '') + out.join('/');
    if (wantTrailing && res.charAt(res.length - 1) !== '/') res = res + '/';
    return res + tail;
};
globalThis.__puriy_resolve_url = function(url, base) {
    if (url == null) return base || '';
    url = String(url);
    if (!url) return base || '';
    // Absolute con scheme.
    if (/^[a-z][a-z0-9+.\-]*:/i.test(url)) return url;
    if (!base) return url;
    base = String(base);
    var m = /^([a-z][a-z0-9+.\-]*:)\/\/([^\/?#]+)(\/[^?#]*)?/i.exec(base);
    if (!m) return url;
    var scheme = m[1];
    var origin = scheme + '//' + m[2];
    var basePath = m[3] || '/';
    if (url.indexOf('//') === 0) return scheme + url;
    if (url.charAt(0) === '/') return origin + globalThis.__puriy_normalize_path(url);
    if (url.charAt(0) === '?') return origin + basePath + url;
    if (url.charAt(0) === '#') {
        var hp = base.indexOf('#');
        return (hp >= 0 ? base.substring(0, hp) : base) + url;
    }
    var lastSlash = basePath.lastIndexOf('/');
    var dir = lastSlash >= 0 ? basePath.substring(0, lastSlash + 1) : '/';
    return origin + globalThis.__puriy_normalize_path(dir + url);
};
"#;
