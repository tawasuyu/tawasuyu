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
"#;
