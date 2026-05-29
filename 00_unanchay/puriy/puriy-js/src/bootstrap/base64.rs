pub(crate) const BASE64_BOOTSTRAP: &str = r#"
// Fase 7.53 — btoa / atob (base64 sobre binary strings, igual que el
// browser). `btoa` toma una string donde cada char es un byte 0..255 y
// devuelve base64; tira si algún char excede 0xFF (InvalidCharacterError,
// como el spec). `atob` invierte: ignora whitespace, valida la longitud y
// el alfabeto. Nota spec: NO operan sobre UTF-8 — para texto Unicode el
// patrón es `btoa(new TextEncoder()... )` no aplica directo; el idiom web
// es `btoa(unescape(encodeURIComponent(s)))`. Lo dejamos fiel al browser.
(function() {
    var ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    globalThis.btoa = function(input) {
        var str = String(input);
        for (var i = 0; i < str.length; i++) {
            if (str.charCodeAt(i) > 0xff) {
                throw new Error("InvalidCharacterError: btoa: caracter fuera del rango Latin1");
            }
        }
        var out = '';
        for (var j = 0; j < str.length; j += 3) {
            var c0 = str.charCodeAt(j);
            var has1 = j + 1 < str.length;
            var has2 = j + 2 < str.length;
            var c1 = has1 ? str.charCodeAt(j + 1) : 0;
            var c2 = has2 ? str.charCodeAt(j + 2) : 0;
            var n = (c0 << 16) | (c1 << 8) | c2;
            out += ALPHABET.charAt((n >> 18) & 63)
                 + ALPHABET.charAt((n >> 12) & 63)
                 + (has1 ? ALPHABET.charAt((n >> 6) & 63) : '=')
                 + (has2 ? ALPHABET.charAt(n & 63) : '=');
        }
        return out;
    };
    globalThis.atob = function(input) {
        var str = String(input).replace(/[ \t\n\f\r]/g, '');
        if (str.length % 4 === 1) {
            throw new Error("InvalidCharacterError: atob: longitud de base64 invalida");
        }
        str = str.replace(/=+$/, '');
        var out = '';
        var buffer = 0;
        var bits = 0;
        for (var i = 0; i < str.length; i++) {
            var idx = ALPHABET.indexOf(str.charAt(i));
            if (idx < 0) {
                throw new Error("InvalidCharacterError: atob: caracter base64 invalido");
            }
            buffer = (buffer << 6) | idx;
            bits += 6;
            if (bits >= 8) {
                bits -= 8;
                out += String.fromCharCode((buffer >> bits) & 0xff);
            }
        }
        return out;
    };
})();
"#;
