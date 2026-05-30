pub(crate) const URLCLASS_BOOTSTRAP: &str = r#"
// Fase 7.58 — `new URL(url, base)` real. Hasta acá `URL` era sólo un objeto
// con los estáticos `createObjectURL`/`revokeObjectURL` (módulo `objecturl`,
// Fase 7.50). Ahora es un constructor WHATWG-ish: resuelve `url` contra `base`
// vía `__puriy_resolve_url` (reusa el resolver de Fase 7.37/7.46), parsea el
// href absoluto en componentes y expone `searchParams` (un URLSearchParams
// vivo sobre el query). Preservamos los estáticos que objecturl.rs ya puso.
//
// Cargado DESPUÉS de `urlsearchparams` (que provee `.searchParams`) y de
// `objecturl` (cuyos estáticos rescatamos). Limitaciones: parser de host
// simple (`hostname[:port]`, sin IPv6 entre corchetes con `:` interno); sólo
// reconstruye href para schemes con autoridad (`scheme://…`), suficiente para
// http/https/ws/wss/ftp/blob — un `mailto:` no round-trip perfecto.
(function() {
    var prevCreate = (globalThis.URL && globalThis.URL.createObjectURL) || null;
    var prevRevoke = (globalThis.URL && globalThis.URL.revokeObjectURL) || null;

    function parseInto(self, url, base) {
        var resolved = globalThis.__puriy_resolve_url(url, (base != null) ? String(base) : '');
        var m = /^([a-z][a-z0-9+.\-]*:)(\/\/([^\/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/i.exec(resolved);
        if (!m || !m[1]) throw new TypeError('Invalid URL: ' + url);
        self.protocol = m[1];
        self._hasAuthority = (m[2] != null);
        var authority = m[3] || '';
        var userinfo = '';
        var hostport = authority;
        var at = authority.lastIndexOf('@');
        if (at >= 0) { userinfo = authority.substring(0, at); hostport = authority.substring(at + 1); }
        var username = userinfo, password = '';
        var uc = userinfo.indexOf(':');
        if (uc >= 0) { username = userinfo.substring(0, uc); password = userinfo.substring(uc + 1); }
        self.username = username;
        self.password = password;
        var hostname = hostport, port = '';
        var hc = hostport.lastIndexOf(':');
        if (hc >= 0 && hostport.indexOf(']') < hc) {
            hostname = hostport.substring(0, hc);
            port = hostport.substring(hc + 1);
        }
        self.hostname = hostname;
        self.port = port;
        self.host = hostport;
        self.pathname = m[4] || (self._hasAuthority ? '/' : '');
        self.hash = m[6] || '';
        self._search = m[5] || '';
        var sp = self.protocol;
        if (sp === 'http:' || sp === 'https:' || sp === 'ftp:' || sp === 'ws:' || sp === 'wss:') {
            self.origin = sp + '//' + self.host;
        } else {
            self.origin = 'null';
        }
        self.searchParams = new globalThis.URLSearchParams(self._search);
    }

    function URL(url, base) {
        parseInto(this, url, base);
    }
    Object.defineProperty(URL.prototype, 'search', {
        get: function() {
            var s = this.searchParams.toString();
            return s ? '?' + s : '';
        },
        set: function(v) {
            v = String(v);
            this._search = v ? (v.charAt(0) === '?' ? v : '?' + v) : '';
            this.searchParams = new globalThis.URLSearchParams(this._search);
        }
    });
    Object.defineProperty(URL.prototype, 'href', {
        get: function() {
            var auth = '';
            if (this.username) {
                auth = this.username;
                if (this.password) auth += ':' + this.password;
                auth += '@';
            }
            var s = this.protocol;
            if (this._hasAuthority) s += '//' + auth + this.host;
            return s + this.pathname + this.search + this.hash;
        },
        set: function(v) {
            parseInto(this, String(v), '');
        }
    });
    URL.prototype.toString = function() { return this.href; };
    URL.prototype.toJSON = function() { return this.href; };

    if (prevCreate) URL.createObjectURL = prevCreate;
    if (prevRevoke) URL.revokeObjectURL = prevRevoke;
    globalThis.URL = URL;
})();
"#;
