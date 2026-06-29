// blend.wgsl — espejo GPU de `tullpu_render::fundir_buffer`.
//
// Una invocación por píxel: funde `src` sobre `acc` aplicando el modo de
// fusión, la opacidad, la máscara y el recorte (clip) de la capa. Escribe el
// resultado empaquetado rgba8 de vuelta en `acc` y la cobertura (alfa
// efectiva) en `cobertura` — esta última alimenta el clip de la próxima capa.
//
// `acc`/`src` son `array<u32>`: cada elemento es un píxel rgba8 empaquetado
// little-endian (byte0 = R), idéntico al `Vec<u8>` del compositor CPU.
// `pack4x8unorm` redondea al entero más cercano igual que `clamp_u8`, así que
// la paridad con la CPU es de ±1 por canal (sólo difiere el desempate en .5).

struct Params {
    modo: u32,
    has_mask: u32,
    has_clip: u32,
    n: u32,
    opacidad: f32,
    stride: u32,   // anchura en hilos de la grilla 2D de dispatch
    _p0: u32,
    _p1: u32,
};

@group(0) @binding(0) var<storage, read_write> acc: array<u32>;
@group(0) @binding(1) var<storage, read>       src: array<u32>;
@group(0) @binding(2) var<storage, read>       mask: array<u32>;       // bytes empaquetados 4/word
@group(0) @binding(3) var<storage, read>       clip: array<f32>;       // base_alpha
@group(0) @binding(4) var<storage, read_write> cobertura: array<f32>;
@group(0) @binding(5) var<uniform>             P: Params;

const EPS: f32 = 1.1920929e-7;   // f32::EPSILON

fn lum(c: vec3<f32>) -> f32 {
    return 0.3 * c.x + 0.59 * c.y + 0.11 * c.z;
}

fn sat(c: vec3<f32>) -> f32 {
    let mx = max(c.x, max(c.y, c.z));
    let mn = min(c.x, min(c.y, c.z));
    return mx - mn;
}

fn clip_color(c0: vec3<f32>) -> vec3<f32> {
    var c = c0;
    let l = lum(c);
    let n = min(c.x, min(c.y, c.z));
    let x = max(c.x, max(c.y, c.z));
    if (n < 0.0) {
        let k = l / (l - n);
        c = vec3<f32>(l) + (c - vec3<f32>(l)) * k;
    }
    if (x > 1.0) {
        let k = (1.0 - l) / (x - l);
        c = vec3<f32>(l) + (c - vec3<f32>(l)) * k;
    }
    return c;
}

fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    let d = l - lum(c);
    return clip_color(c + vec3<f32>(d));
}

// SetSat del spec W3C: reescala `c` para que su saturación sea `s`,
// preservando el orden relativo de canales. Ordena los tres canales con un
// insertion sort estable (sólo desplaza ante un > estricto), de modo que los
// empates conservan el orden de índice — igual que el `sort_by` estable de la
// CPU.
fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    var vals = array<f32, 3>(c.x, c.y, c.z);
    var idx  = array<u32, 3>(0u, 1u, 2u);
    for (var i = 1u; i < 3u; i = i + 1u) {
        let cv = vals[i];
        let ci = idx[i];
        var j = i;
        loop {
            if (j == 0u) { break; }
            if (vals[j - 1u] > cv) {
                vals[j] = vals[j - 1u];
                idx[j] = idx[j - 1u];
                j = j - 1u;
            } else {
                break;
            }
        }
        vals[j] = cv;
        idx[j] = ci;
    }
    let cmin = vals[0];
    let cmid = vals[1];
    let cmax = vals[2];
    var outc = array<f32, 3>(0.0, 0.0, 0.0);
    if (cmax > cmin) {
        outc[idx[1]] = ((cmid - cmin) * s) / (cmax - cmin);
        outc[idx[2]] = s;
    }
    // outc[idx[0]] queda en 0.0 (el canal mínimo).
    return vec3<f32>(outc[0], outc[1], outc[2]);
}

// Función de fusión por canal — espejo del closure `f` de `mezclar_canal`.
fn blend_ch(modo: u32, s: f32, d: f32) -> f32 {
    switch modo {
        case 0u:  { return s; }                                    // Normal
        case 1u:  { return s * d; }                                // Multiplicar
        case 2u:  { return 1.0 - (1.0 - s) * (1.0 - d); }          // Pantalla
        case 3u:  {                                                // Superponer
            if (d < 0.5) { return 2.0 * s * d; }
            return 1.0 - 2.0 * (1.0 - s) * (1.0 - d);
        }
        case 4u:  { return max(s, d); }                            // Aclarar
        case 5u:  { return min(s, d); }                            // Oscurecer
        case 6u:  { return abs(s - d); }                           // Diferencia
        case 7u:  { return clamp(s + d, 0.0, 1.0); }               // Aditivo
        case 8u:  {                                                // SubExpQuemado (Color Burn)
            if (s <= EPS) { return 0.0; }
            return clamp(1.0 - (1.0 - d) / s, 0.0, 1.0);
        }
        case 9u:  { return clamp(s + d - 1.0, 0.0, 1.0); }         // SubLinealQuemado
        case 10u: {                                                // SobreExpAclarado (Color Dodge)
            if (s >= 1.0 - EPS) { return 1.0; }
            return clamp(d / (1.0 - s), 0.0, 1.0);
        }
        case 11u: {                                                // LuzFuerte (Hard Light)
            if (s < 0.5) { return 2.0 * s * d; }
            return 1.0 - 2.0 * (1.0 - s) * (1.0 - d);
        }
        case 12u: {                                                // LuzSuave (Soft Light)
            var g_d: f32;
            if (d <= 0.25) { g_d = ((16.0 * d - 12.0) * d + 4.0) * d; }
            else { g_d = sqrt(d); }
            if (s <= 0.5) { return clamp(d - (1.0 - 2.0 * s) * d * (1.0 - d), 0.0, 1.0); }
            return clamp(d + (2.0 * s - 1.0) * (g_d - d), 0.0, 1.0);
        }
        case 13u: {                                                // LuzViva (Vivid Light)
            if (s < 0.5) {
                let s2 = 2.0 * s;
                if (s2 <= EPS) { return 0.0; }
                return clamp(1.0 - (1.0 - d) / s2, 0.0, 1.0);
            }
            let s2 = 2.0 * s - 1.0;
            if (s2 >= 1.0 - EPS) { return 1.0; }
            return clamp(d / (1.0 - s2), 0.0, 1.0);
        }
        case 14u: { return clamp(d + 2.0 * s - 1.0, 0.0, 1.0); }   // LuzLineal
        case 15u: {                                                // LuzPunto (Pin Light)
            if (s < 0.5) { return min(d, 2.0 * s); }
            return max(d, 2.0 * s - 1.0);
        }
        case 16u: {                                                // MezclaDura (Hard Mix)
            if (s + d >= 1.0) { return 1.0; }
            return 0.0;
        }
        case 17u: { return clamp(s + d - 2.0 * s * d, 0.0, 1.0); } // Exclusion
        case 18u: { return clamp(d - s, 0.0, 1.0); }               // Resta
        case 19u: {                                                // Division
            if (s <= EPS) { return 1.0; }
            return clamp(d / s, 0.0, 1.0);
        }
        default:  { return s; }
    }
}

fn mezclar(modo: u32, s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    // HSL y comparativos por-luminosidad: operan sobre el triple completo.
    switch modo {
        case 20u: { return set_lum(set_sat(s, sat(d)), lum(d)); }  // HslTono
        case 21u: { return set_lum(set_sat(d, sat(s)), lum(d)); }  // HslSaturacion
        case 22u: { return set_lum(s, lum(d)); }                   // HslColor
        case 23u: { return set_lum(d, lum(s)); }                   // HslLuminosidad
        case 24u: { if (lum(s) < lum(d)) { return s; } return d; } // ColorMasOscuro
        case 25u: { if (lum(s) > lum(d)) { return s; } return d; } // ColorMasClaro
        default:  {}
    }
    return vec3<f32>(
        blend_ch(modo, s.x, d.x),
        blend_ch(modo, s.y, d.y),
        blend_ch(modo, s.z, d.z),
    );
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.y * P.stride + gid.x;
    if (i >= P.n) { return; }

    let s = unpack4x8unorm(src[i]);    // rgba straight en [0,1]
    let dpx = unpack4x8unorm(acc[i]);

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

    let src_alpha = s.w * P.opacidad * m * c;
    cobertura[i] = src_alpha;

    let da = dpx.w;
    let blended = mezclar(P.modo, s.xyz, dpx.xyz);

    let out_a = src_alpha + da * (1.0 - src_alpha);
    var out_rgb = vec3<f32>(0.0, 0.0, 0.0);
    if (out_a > EPS) {
        out_rgb = (blended * src_alpha + dpx.xyz * da * (1.0 - src_alpha)) / out_a;
    }
    acc[i] = pack4x8unorm(vec4<f32>(out_rgb, out_a));
}
