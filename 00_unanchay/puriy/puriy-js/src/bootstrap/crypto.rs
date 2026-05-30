pub(crate) const CRYPTO_BOOTSTRAP: &str = r#"
// Fase 7.64 — Web Crypto mínimo: `crypto.getRandomValues` + `crypto.randomUUID`.
// Muy usados por apps de red (nonces, idempotency keys, request ids). La
// entropía sale de `Math.random` (QuickJS trae su propio RNG — no dependemos
// del import WASI `random_get`).
//
// Divergencias documentadas: (1) `getRandomValues` no distingue Float32/64
// (el spec tira TypeMismatchError; acá los llena igual). (2) `Math.random` no
// es criptográficamente seguro — suficiente para ids/nonces de red, NO para
// material de clave real. (3) `crypto.subtle` NO está (sin SHA/AES/etc.).
globalThis.crypto = globalThis.crypto || {};
globalThis.crypto.getRandomValues = function(typedArray) {
    if (!typedArray || typeof typedArray.length !== 'number'
        || !(typedArray.buffer instanceof ArrayBuffer)) {
        throw new TypeError("getRandomValues: el argumento no es un TypedArray entero");
    }
    var bytesPerElem = typedArray.BYTES_PER_ELEMENT || 1;
    // Límite del spec: a lo sumo 65536 bytes por llamada.
    if (typedArray.length * bytesPerElem > 65536) {
        throw new Error('QuotaExceededError: getRandomValues admite hasta 65536 bytes');
    }
    for (var i = 0; i < typedArray.length; i++) {
        // La asignación al TypedArray auto-trunca al ancho del elemento.
        typedArray[i] = Math.floor(Math.random() * 4294967296);
    }
    return typedArray;
};
globalThis.crypto.randomUUID = function() {
    // UUID v4: xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx con y ∈ {8,9,a,b}.
    var hex = '0123456789abcdef';
    var s = '';
    for (var i = 0; i < 36; i++) {
        if (i === 8 || i === 13 || i === 18 || i === 23) { s += '-'; continue; }
        if (i === 14) { s += '4'; continue; }
        var r = Math.floor(Math.random() * 16);
        if (i === 19) r = (r & 0x3) | 0x8;
        s += hex.charAt(r);
    }
    return s;
};
"#;
