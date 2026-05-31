pub(crate) const TEXTCODEC_BOOTSTRAP: &str = r#"
// Fase 7.52 — TextEncoder / TextDecoder (sólo UTF-8). Complementa los
// bytes que ya viajan por ReadableStream (Fase 7.45/7.49) y por
// `response.arrayBuffer()` (Fase 7.31): `new TextDecoder().decode(uint8)`
// vuelve el chunk a string, `new TextEncoder().encode(str)` produce el
// Uint8Array UTF-8.
//
// UTF-8 manual sobre code points (maneja pares surrogate de UTF-16). No se
// soportan otras codificaciones (label se guarda pero se ignora — siempre
// UTF-8); `decode` con `stream:true` no mantiene estado entre llamadas
// (no hay buffering de secuencias parciales) — divergencia documentada.
globalThis.TextEncoder = function() {
    this.encoding = 'utf-8';
};
globalThis.TextEncoder.prototype.encode = function(input) {
    var str = (input === undefined) ? '' : String(input);
    var bytes = [];
    for (var i = 0; i < str.length; i++) {
        var c = str.charCodeAt(i);
        if (c < 0x80) {
            bytes.push(c);
        } else if (c < 0x800) {
            bytes.push(0xC0 | (c >> 6), 0x80 | (c & 0x3F));
        } else if (c >= 0xD800 && c <= 0xDBFF) {
            // High surrogate: combinar con el siguiente low surrogate.
            var c2 = str.charCodeAt(i + 1);
            if (c2 >= 0xDC00 && c2 <= 0xDFFF) {
                var cp = 0x10000 + ((c - 0xD800) << 10) + (c2 - 0xDC00);
                bytes.push(0xF0 | (cp >> 18), 0x80 | ((cp >> 12) & 0x3F),
                           0x80 | ((cp >> 6) & 0x3F), 0x80 | (cp & 0x3F));
                i++;
            } else {
                // Surrogate suelto → U+FFFD.
                bytes.push(0xEF, 0xBF, 0xBD);
            }
        } else if (c >= 0xDC00 && c <= 0xDFFF) {
            bytes.push(0xEF, 0xBF, 0xBD);
        } else {
            bytes.push(0xE0 | (c >> 12), 0x80 | ((c >> 6) & 0x3F), 0x80 | (c & 0x3F));
        }
    }
    var u = new Uint8Array(bytes.length);
    for (var k = 0; k < bytes.length; k++) u[k] = bytes[k];
    return u;
};
globalThis.TextDecoder = function(label) {
    this.encoding = label ? String(label).toLowerCase() : 'utf-8';
    this.fatal = false;
    this.ignoreBOM = false;
};
globalThis.TextDecoder.prototype.decode = function(input) {
    if (input == null) return '';
    var bytes;
    if (input instanceof ArrayBuffer) {
        bytes = new Uint8Array(input);
    } else if (input && typeof input.length === 'number') {
        // TypedArray o array-like de bytes.
        bytes = input;
    } else {
        return '';
    }
    var out = '';
    var i = 0;
    var n = bytes.length;
    while (i < n) {
        var b = bytes[i++];
        var cp;
        if (b < 0x80) {
            cp = b;
        } else if ((b & 0xE0) === 0xC0) {
            cp = ((b & 0x1F) << 6) | (bytes[i++] & 0x3F);
        } else if ((b & 0xF0) === 0xE0) {
            cp = ((b & 0x0F) << 12) | ((bytes[i++] & 0x3F) << 6) | (bytes[i++] & 0x3F);
        } else if ((b & 0xF8) === 0xF0) {
            cp = ((b & 0x07) << 18) | ((bytes[i++] & 0x3F) << 12) |
                 ((bytes[i++] & 0x3F) << 6) | (bytes[i++] & 0x3F);
        } else {
            cp = 0xFFFD;
        }
        out += String.fromCodePoint(cp);
    }
    return out;
};
"#;
