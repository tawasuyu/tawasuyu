// ajuste.wgsl — espejo GPU de `tullpu_render::aplicar_ajuste`.
//
// Una capa de ajuste copia el compuesto-hasta-aquí, le aplica una op per-píxel
// RGB (alfa intacto) y mezcla el resultado de vuelta por `opacidad·máscara·clip`
// por píxel. Acá `acc` ya ES el compuesto: leemos el píxel, calculamos el RGB
// ajustado y lo mezclamos in-place.
//
// `op_kind`:
//   0 = LUT       — ops independientes por canal (Invertir/Brillo/Contraste/
//                   Niveles/Curvas). La LUT de 256 entradas la precalcula la CPU
//                   con su código exacto → bit-idéntico salvo el redondeo final.
//   1 = Saturacion — HSL: s' = clamp(s·param)
//   2 = Tonalidad  — HSL: h' = rem_euclid(h + param)
//
// Los ajustes NO aportan base de clipping (igual que la CPU): no escriben
// cobertura.

struct AjusteParams {
    op_kind: u32,
    has_mask: u32,
    has_clip: u32,
    n: u32,
    opacidad: f32,
    param: f32,    // factor (Saturacion) o delta=grados/360 (Tonalidad)
    stride: u32,
    _p: u32,
};

@group(0) @binding(0) var<storage, read_write> acc: array<u32>;
@group(0) @binding(1) var<storage, read>       lut: array<u32>;   // 256 bytes empaquetados 4/word
@group(0) @binding(2) var<storage, read>       mask: array<u32>;  // bytes empaquetados 4/word
@group(0) @binding(3) var<storage, read>       clip: array<f32>;
@group(0) @binding(4) var<uniform>             P: AjusteParams;

fn rgb2hsl(c: vec3<f32>) -> vec3<f32> {
    let r = c.x; let g = c.y; let b = c.z;
    let mx = max(r, max(g, b));
    let mn = min(r, min(g, b));
    let l = (mx + mn) * 0.5;
    if (abs(mx - mn) < 1e-6) { return vec3<f32>(0.0, 0.0, l); }
    let d = mx - mn;
    var s: f32;
    if (l < 0.5) { s = d / (mx + mn); } else { s = d / (2.0 - mx - mn); }
    var h: f32;
    if (abs(mx - r) < 1e-6) {
        h = (g - b) / d + select(0.0, 6.0, g < b);
    } else if (abs(mx - g) < 1e-6) {
        h = (b - r) / d + 2.0;
    } else {
        h = (r - g) / d + 4.0;
    }
    return vec3<f32>(h / 6.0, s, l);
}

fn hue2rgb(p: f32, q: f32, t0: f32) -> f32 {
    var t = t0;
    if (t < 0.0) { t = t + 1.0; } else if (t > 1.0) { t = t - 1.0; }
    if (t < 1.0 / 6.0) { return p + (q - p) * 6.0 * t; }
    if (t < 0.5) { return q; }
    if (t < 2.0 / 3.0) { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

fn hsl2rgb(h: f32, s: f32, l: f32) -> vec3<f32> {
    if (abs(s) < 1e-6) { return vec3<f32>(l, l, l); }
    var q: f32;
    if (l < 0.5) { q = l * (1.0 + s); } else { q = l + s - l * s; }
    let p = 2.0 * l - q;
    return vec3<f32>(
        hue2rgb(p, q, h + 1.0 / 3.0),
        hue2rgb(p, q, h),
        hue2rgb(p, q, h - 1.0 / 3.0),
    );
}

fn lut_val(c: f32) -> f32 {
    let idx = u32(round(c * 255.0));
    let word = lut[idx >> 2u];
    let byte = (word >> ((idx & 3u) * 8u)) & 0xffu;
    return f32(byte) / 255.0;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.y * P.stride + gid.x;
    if (i >= P.n) { return; }

    let base = unpack4x8unorm(acc[i]);
    var adj = base.xyz;

    switch P.op_kind {
        case 0u: {                                  // LUT por canal
            adj = vec3<f32>(lut_val(base.x), lut_val(base.y), lut_val(base.z));
        }
        case 1u: {                                  // Saturacion
            let hsl = rgb2hsl(base.xyz);
            let s_in = clamp(hsl.y, 0.0, 1.0);
            let l_in = clamp(hsl.z, 0.0, 1.0);
            adj = hsl2rgb(hsl.x, clamp(s_in * P.param, 0.0, 1.0), l_in);
        }
        case 2u: {                                  // Tonalidad
            let hsl = rgb2hsl(base.xyz);
            let s_in = clamp(hsl.y, 0.0, 1.0);
            let l_in = clamp(hsl.z, 0.0, 1.0);
            let h2 = (hsl.x + P.param) - floor(hsl.x + P.param);   // rem_euclid(·, 1.0)
            adj = hsl2rgb(h2, s_in, l_in);
        }
        default: {}
    }

    // La CPU cuantiza el RGB ajustado a u8 (`clamp_u8`) ANTES de mezclar.
    // Replicarlo deja la mezcla bit-idéntica (sólo difiere el redondeo en .5).
    adj = round(clamp(adj, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0) / 255.0;

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
    let f = clamp(P.opacidad, 0.0, 1.0) * m * c;

    let out_rgb = base.xyz * (1.0 - f) + adj * f;
    acc[i] = pack4x8unorm(vec4<f32>(out_rgb, base.w));   // alfa intacto
}
