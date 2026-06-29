// disolver.wgsl — espejo GPU de `tullpu_render::fundir_disolver`.
//
// Disolver es un umbralizador estocástico estable: por píxel se compara el alfa
// efectivo contra un umbral pseudoaleatorio sembrado por el `Uuid` de la capa
// (splitmix64, 1 sample/píxel). Si el alfa gana, el src reemplaza al dst opaco;
// si no, el dst queda intacto. La decisión es binaria → la paridad con la CPU
// es **exacta** (no ±1), siempre que el RNG coincida bit a bit.
//
// WGSL no tiene u64, así que emulamos el splitmix64 con `vec2<u32>` (lo, hi).
// Comparte el `struct Params` y el bind group layout de `blend.wgsl`: los dos
// words de cola (`seed_lo`/`seed_hi`) llevan la semilla.

struct Params {
    modo: u32,
    has_mask: u32,
    has_clip: u32,
    n: u32,
    opacidad: f32,
    stride: u32,
    seed_lo: u32,
    seed_hi: u32,
    band_offset: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
};

@group(0) @binding(0) var<storage, read_write> acc: array<u32>;
@group(0) @binding(1) var<storage, read>       src: array<u32>;
@group(0) @binding(2) var<storage, read>       mask: array<u32>;
@group(0) @binding(3) var<storage, read>       clip: array<f32>;
@group(0) @binding(4) var<storage, read_write> cobertura: array<f32>;
@group(0) @binding(5) var<uniform>             P: Params;

// --- aritmética u64 emulada con vec2<u32> = (lo, hi) ---

fn u64_add(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.x + b.x;
    let carry = select(0u, 1u, lo < a.x);
    return vec2<u32>(lo, a.y + b.y + carry);
}

fn u64_xor(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    return vec2<u32>(a.x ^ b.x, a.y ^ b.y);
}

// Desplazamiento a derecha por `k` (0 < k < 64). Todos los sub-shifts quedan
// < 32 para no toparse con el comportamiento indefinido de `>> 32`.
fn u64_shr(a: vec2<u32>, k: u32) -> vec2<u32> {
    if (k == 0u) { return a; }
    if (k < 32u) {
        let lo = (a.x >> k) | (a.y << (32u - k));
        return vec2<u32>(lo, a.y >> k);
    }
    return vec2<u32>(a.y >> (k - 32u), 0u);
}

// Producto completo 32×32 → 64 bits (descomposición en mitades de 16 bits).
fn mul_u32_full(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 0xffffu; let a1 = a >> 16u;
    let b0 = b & 0xffffu; let b1 = b >> 16u;
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    var low = p00;
    var high = p11;
    let s1 = low + (p01 << 16u);
    high = high + (p01 >> 16u) + select(0u, 1u, s1 < low);
    low = s1;
    let s2 = low + (p10 << 16u);
    high = high + (p10 >> 16u) + select(0u, 1u, s2 < low);
    low = s2;
    return vec2<u32>(low, high);
}

// Producto módulo 2^64 (low 64 bits de a·b).
fn u64_mul(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let ll = mul_u32_full(a.x, b.x);
    // Los términos cruzados (<<32) sólo aportan su parte baja al word alto.
    let hi = ll.y + (a.x * b.y) + (a.y * b.x);
    return vec2<u32>(ll.x, hi);
}

// splitmix64 de `tullpu_render::umbral_dissolve` → [0,1) con mantisa de 24 bits.
fn umbral(seed: vec2<u32>, i: u32) -> f32 {
    let phi = vec2<u32>(0x7F4A7C15u, 0x9E3779B9u);   // 0x9E3779B97F4A7C15
    var x = u64_add(seed, u64_mul(vec2<u32>(i, 0u), phi));
    let c1 = vec2<u32>(0x1CE4E5B9u, 0xBF58476Du);    // 0xBF58476D1CE4E5B9
    x = u64_mul(u64_xor(x, u64_shr(x, 30u)), c1);
    let c2 = vec2<u32>(0x133111EBu, 0x94D049BBu);    // 0x94D049BB133111EB
    x = u64_mul(u64_xor(x, u64_shr(x, 27u)), c2);
    x = u64_xor(x, u64_shr(x, 31u));
    let top = u64_shr(x, 40u);                        // 24 bits altos
    return f32(top.x) / 16777216.0;                  // / 2^24
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.y * P.stride + gid.x;
    if (i >= P.n) { return; }

    let s = unpack4x8unorm(src[i]);
    var m: f32 = 1.0;
    if (P.has_mask != 0u) {
        let word = mask[i >> 2u];
        let byte = (word >> ((i & 3u) * 8u)) & 0xffu;
        m = f32(byte) / 255.0;
    }
    var c: f32 = 1.0;
    if (P.has_clip != 0u) {
        c = clip[i];
    }
    let alfa = s.w * P.opacidad * m * c;
    // El índice del RNG es GLOBAL (offset de banda + local) para que el patrón
    // de disolución sea idéntico se tilee o no el lienzo.
    let gi = P.band_offset + i;
    let u = umbral(vec2<u32>(P.seed_lo, P.seed_hi), gi);

    if (alfa > u) {
        // src gana: rgb del src, opaco. (idempotente sobre el grid u8)
        acc[i] = pack4x8unorm(vec4<f32>(s.xyz, 1.0));
        cobertura[i] = 1.0;
    } else {
        cobertura[i] = 0.0;   // dst intacto
    }
}
