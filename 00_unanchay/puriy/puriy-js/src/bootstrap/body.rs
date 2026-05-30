pub(crate) const BODY_BOOTSTRAP: &str = r#"
// Fase 7.57 — Serialización del body de fetch/XHR. Cierra dos divergencias
// abiertas: (1) Fase 7.54 dejó FormData SIN serializar a multipart/form-data;
// (2) Fase 7.51 dejó URLSearchParams sin auto-setear el Content-Type. Acá un
// único helper normaliza cualquier body a `{ text, contentType, hasBody }`:
//   - FormData         → multipart/form-data con boundary acuñado; los Blob
//                        parts llevan filename + su Content-Type propio.
//   - URLSearchParams  → application/x-www-form-urlencoded;charset=UTF-8.
//   - Blob             → bytes crudos como binary string + el .type del Blob.
//   - string / otros   → String(body), sin Content-Type implícito (null).
// El caller (fetch.rs / xhr.rs) aplica el Content-Type implícito SÓLO si el
// user no seteó ya un Content-Type — `__puriy_apply_content_type` lo hace.
//
// Divergencia documentada: para FormData respetamos un Content-Type que el
// user haya seteado a mano, aunque el spec fuerce el del boundary acuñado
// (un Content-Type sin el boundary correcto rompería el parse del lado
// server). En la práctica nadie setea Content-Type a mano con FormData.
globalThis.__puriy_multipart_boundary_next = 1;
globalThis.__puriy_serialize_body = function(body) {
    if (body == null) return { text: '', contentType: null, hasBody: false };
    // FormData → multipart/form-data.
    if (globalThis.FormData && body instanceof globalThis.FormData) {
        var boundary = '----puriyFormBoundary' + (globalThis.__puriy_multipart_boundary_next++);
        var out = '';
        for (var i = 0; i < body._list.length; i++) {
            var name = body._list[i][0];
            var value = body._list[i][1];
            var filename = body._list[i][2];
            out += '--' + boundary + '\r\n';
            if (globalThis.Blob && value instanceof globalThis.Blob) {
                var fn = (filename != null) ? filename : 'blob';
                out += 'Content-Disposition: form-data; name="' + name + '"; filename="' + fn + '"\r\n';
                out += 'Content-Type: ' + (value.type || 'application/octet-stream') + '\r\n\r\n';
                var bs = '';
                for (var j = 0; j < value._bytes.length; j++) bs += String.fromCharCode(value._bytes[j]);
                out += bs + '\r\n';
            } else {
                if (filename != null) {
                    out += 'Content-Disposition: form-data; name="' + name + '"; filename="' + filename + '"\r\n\r\n';
                } else {
                    out += 'Content-Disposition: form-data; name="' + name + '"\r\n\r\n';
                }
                out += String(value) + '\r\n';
            }
        }
        out += '--' + boundary + '--\r\n';
        return { text: out, contentType: 'multipart/form-data; boundary=' + boundary, hasBody: true };
    }
    // URLSearchParams → form-urlencoded.
    if (globalThis.URLSearchParams && body instanceof globalThis.URLSearchParams) {
        return {
            text: body.toString(),
            contentType: 'application/x-www-form-urlencoded;charset=UTF-8',
            hasBody: true
        };
    }
    // Blob → bytes crudos + su type.
    if (globalThis.Blob && body instanceof globalThis.Blob) {
        var s = '';
        for (var k = 0; k < body._bytes.length; k++) s += String.fromCharCode(body._bytes[k]);
        return { text: s, contentType: body.type || null, hasBody: true };
    }
    return { text: String(body), contentType: null, hasBody: true };
};
// Aplica el Content-Type implícito a un array de pares `[name, value, ...]`
// (el formato que fetch.rs/xhr.rs serializan) SÓLO si todavía no hay un
// Content-Type (match case-insensitive sobre las posiciones pares). Muta y
// devuelve el mismo array.
globalThis.__puriy_apply_content_type = function(hdrPairs, contentType) {
    if (!contentType) return hdrPairs;
    for (var i = 0; i + 1 < hdrPairs.length; i += 2) {
        if (String(hdrPairs[i]).toLowerCase() === 'content-type') return hdrPairs;
    }
    hdrPairs.push('Content-Type');
    hdrPairs.push(contentType);
    return hdrPairs;
};
// Fase 7.63 — inverso del serializer: parsea un body crudo a un FormData
// según el Content-Type. Usado por `Response.formData()` / `Request.formData()`.
//   - application/x-www-form-urlencoded → vía URLSearchParams (cada par append).
//   - multipart/form-data; boundary=… → parte por el delimitador, lee el
//     Content-Disposition (name/filename) y el Content-Type de cada part; los
//     parts con filename se envuelven en Blob (FormData luego los hace File).
//   - cualquier otro Content-Type → TypeError (spec).
globalThis.__puriy_parse_form_body = function(text, contentType) {
    contentType = String(contentType || '');
    var ct = contentType.toLowerCase();
    var fd = new globalThis.FormData();
    if (ct.indexOf('application/x-www-form-urlencoded') === 0) {
        var usp = new globalThis.URLSearchParams(text);
        usp.forEach(function(v, k) { fd.append(k, v); });
        return fd;
    }
    if (ct.indexOf('multipart/form-data') === 0) {
        var bm = /boundary=("?)([^";]+)\1/i.exec(contentType);
        if (!bm) throw new TypeError('multipart/form-data sin boundary');
        var delim = '--' + bm[2];
        var chunks = text.split(delim);
        for (var i = 0; i < chunks.length; i++) {
            var part = chunks[i];
            // Saltea el preámbulo ('') y el epílogo del cierre ('--\r\n');
            // los parts reales arrancan con el CRLF que sigue al delimitador.
            if (part === '' || part.substring(0, 2) === '--') continue;
            if (part.substring(0, 2) === '\r\n') part = part.substring(2);
            var sep = part.indexOf('\r\n\r\n');
            if (sep < 0) continue;
            var rawHeaders = part.substring(0, sep);
            var body = part.substring(sep + 4);
            // El body cierra con el CRLF previo al próximo delimitador.
            if (body.substring(body.length - 2) === '\r\n') body = body.substring(0, body.length - 2);
            var name = null, filename = null, partCT = '';
            var hlines = rawHeaders.split('\r\n');
            for (var h = 0; h < hlines.length; h++) {
                var line = hlines[h];
                var low = line.toLowerCase();
                if (low.indexOf('content-disposition:') === 0) {
                    var nm = /name="([^"]*)"/i.exec(line); if (nm) name = nm[1];
                    var fm = /filename="([^"]*)"/i.exec(line); if (fm) filename = fm[1];
                } else if (low.indexOf('content-type:') === 0) {
                    partCT = line.substring(line.indexOf(':') + 1).replace(/^\s+|\s+$/g, '');
                }
            }
            if (name == null) continue;
            if (filename != null) {
                fd.append(name, new globalThis.Blob([body], { type: partCT }), filename);
            } else {
                fd.append(name, body);
            }
        }
        return fd;
    }
    throw new TypeError('Could not parse content as FormData');
};
"#;
