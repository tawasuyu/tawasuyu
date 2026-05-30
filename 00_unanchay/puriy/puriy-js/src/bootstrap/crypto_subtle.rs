pub(crate) const CRYPTO_SUBTLE_BOOTSTRAP: &str = r#"
// Fase 7.75 — crypto.subtle.digest (SHA-256 + SHA-1) en JS puro. Usado para
// integridad de subrecursos, ETags, deduplicación por contenido, etc. Devuelve
// Promise<ArrayBuffer>. La data de entrada debe ser un BufferSource
// (ArrayBuffer / TypedArray / DataView) — string tira (spec); para texto, pasar
// por TextEncoder primero.
//
// Divergencias: sólo SHA-1 y SHA-256 (SHA-384/512 necesitan aritmética de 64
// bits); todo el digesteo es síncrono envuelto en un Promise resuelto (no hay
// WebCrypto nativo). NO es para material de clave — sólo hashing.
(function() {
    var K = [
        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
    ];
    function padded(msg) {
        var out = msg.slice();
        out.push(0x80);
        while (out.length % 64 !== 56) out.push(0);
        var bitLen = msg.length * 8;
        for (var i = 7; i >= 0; i--) out.push((bitLen / Math.pow(2, 8 * i)) & 0xff);
        return out;
    }
    function rotr(x, n) { return ((x >>> n) | (x << (32 - n))) >>> 0; }
    function rol(x, n) { return ((x << n) | (x >>> (32 - n))) >>> 0; }
    function be(b, off) {
        return ((b[off] << 24) | (b[off + 1] << 16) | (b[off + 2] << 8) | b[off + 3]) >>> 0;
    }
    function words(H, count) {
        var out = [];
        for (var i = 0; i < count; i++) {
            out.push((H[i] >>> 24) & 0xff, (H[i] >>> 16) & 0xff, (H[i] >>> 8) & 0xff, H[i] & 0xff);
        }
        return out;
    }
    function sha256(msg) {
        var H = [0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19];
        var m = padded(msg);
        for (var off = 0; off < m.length; off += 64) {
            var w = new Array(64);
            for (var t = 0; t < 16; t++) w[t] = be(m, off + t * 4);
            for (var t = 16; t < 64; t++) {
                var s0 = (rotr(w[t-15],7) ^ rotr(w[t-15],18) ^ (w[t-15] >>> 3)) >>> 0;
                var s1 = (rotr(w[t-2],17) ^ rotr(w[t-2],19) ^ (w[t-2] >>> 10)) >>> 0;
                w[t] = (w[t-16] + s0 + w[t-7] + s1) >>> 0;
            }
            var a=H[0],b=H[1],c=H[2],d=H[3],e=H[4],f=H[5],g=H[6],h=H[7];
            for (var t = 0; t < 64; t++) {
                var S1 = (rotr(e,6) ^ rotr(e,11) ^ rotr(e,25)) >>> 0;
                var ch = ((e & f) ^ ((~e) & g)) >>> 0;
                var t1 = (h + S1 + ch + K[t] + w[t]) >>> 0;
                var S0 = (rotr(a,2) ^ rotr(a,13) ^ rotr(a,22)) >>> 0;
                var maj = ((a & b) ^ (a & c) ^ (b & c)) >>> 0;
                var t2 = (S0 + maj) >>> 0;
                h=g; g=f; f=e; e=(d+t1)>>>0; d=c; c=b; b=a; a=(t1+t2)>>>0;
            }
            H[0]=(H[0]+a)>>>0;H[1]=(H[1]+b)>>>0;H[2]=(H[2]+c)>>>0;H[3]=(H[3]+d)>>>0;
            H[4]=(H[4]+e)>>>0;H[5]=(H[5]+f)>>>0;H[6]=(H[6]+g)>>>0;H[7]=(H[7]+h)>>>0;
        }
        return words(H, 8);
    }
    function sha1(msg) {
        var H = [0x67452301,0xEFCDAB89,0x98BADCFE,0x10325476,0xC3D2E1F0];
        var m = padded(msg);
        for (var off = 0; off < m.length; off += 64) {
            var w = new Array(80);
            for (var t = 0; t < 16; t++) w[t] = be(m, off + t * 4);
            for (var t = 16; t < 80; t++) w[t] = rol((w[t-3] ^ w[t-8] ^ w[t-14] ^ w[t-16]) >>> 0, 1);
            var a=H[0],b=H[1],c=H[2],d=H[3],e=H[4];
            for (var t = 0; t < 80; t++) {
                var f, k;
                if (t < 20)      { f = ((b & c) | ((~b) & d)) >>> 0;        k = 0x5A827999; }
                else if (t < 40) { f = (b ^ c ^ d) >>> 0;                  k = 0x6ED9EBA1; }
                else if (t < 60) { f = ((b & c) | (b & d) | (c & d)) >>> 0; k = 0x8F1BBCDC; }
                else             { f = (b ^ c ^ d) >>> 0;                  k = 0xCA62C1D6; }
                var tmp = (rol(a,5) + f + e + k + w[t]) >>> 0;
                e=d; d=c; c=rol(b,30); b=a; a=tmp;
            }
            H[0]=(H[0]+a)>>>0;H[1]=(H[1]+b)>>>0;H[2]=(H[2]+c)>>>0;H[3]=(H[3]+d)>>>0;H[4]=(H[4]+e)>>>0;
        }
        return words(H, 5);
    }
    function toBytes(data) {
        if (data instanceof ArrayBuffer) data = new Uint8Array(data);
        else if (ArrayBuffer.isView(data)) data = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
        else throw new TypeError('digest: data debe ser un BufferSource (ArrayBuffer/TypedArray/DataView)');
        var a = [];
        for (var i = 0; i < data.length; i++) a.push(data[i]);
        return a;
    }
    globalThis.crypto = globalThis.crypto || {};
    globalThis.crypto.subtle = globalThis.crypto.subtle || {};
    globalThis.crypto.subtle.digest = function(algorithm, data) {
        var name = (typeof algorithm === 'string') ? algorithm : ((algorithm && algorithm.name) || '');
        name = String(name).toUpperCase();
        var bytes;
        try { bytes = toBytes(data); }
        catch (e) { return Promise.reject(e); }
        var out;
        if (name === 'SHA-256') out = sha256(bytes);
        else if (name === 'SHA-1') out = sha1(bytes);
        else return Promise.reject(new Error('NotSupportedError: digest no soporta ' + name));
        var buf = new ArrayBuffer(out.length);
        var view = new Uint8Array(buf);
        for (var i = 0; i < out.length; i++) view[i] = out[i];
        return Promise.resolve(buf);
    };
})();
"#;
