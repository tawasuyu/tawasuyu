//! Fuentes GLSL ES 3.00 para GioSer.
//!
//! Cada `const &str` es un shader completo listo para `gl.shaderSource()`.
//! Convención: precision `highp float`, atributo `a_pos`, varying `v_*`,
//! uniforms `u_*`.

#![no_std]

/// Vertex shader para quads en clip-space `[-1, 1]²`.
pub const VS_FULLSCREEN: &str = "#version 300 es
precision highp float;
in vec2 a_pos;
out vec2 v_clip;
out vec2 v_uv;
void main() {
    v_clip = a_pos;
    v_uv = a_pos * 0.5 + 0.5;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
";

/// Fragment del fondo cósmico: nubes FBM en 3 capas con drift visible,
/// 3 estratos de estrellas con titilación independiente, viñeta radial,
/// 4 meteoros procedurales con vida cíclica.
pub const FS_COSMOS: &str = "#version 300 es
precision highp float;
in vec2 v_clip;
in vec2 v_uv;
out vec4 fragColor;
uniform vec2  u_resolution;
uniform float u_time;
uniform vec2  u_parallax;
uniform vec3  u_void;
uniform vec3  u_nebula_a;
uniform vec3  u_nebula_b;
uniform vec3  u_stardust;

float hash21(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}
float hash11(float n) {
    return fract(sin(n * 78.233) * 43758.5453);
}
float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash21(i);
    float b = hash21(i + vec2(1.0, 0.0));
    float c = hash21(i + vec2(0.0, 1.0));
    float d = hash21(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}
float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.55;
    for (int i = 0; i < 5; i++) {
        v += a * vnoise(p);
        p = p * 2.07 + vec2(11.3, 7.7);
        a *= 0.55;
    }
    return v;
}

float meteor(vec2 uv, float seed) {
    float period = 6.5 + 4.0 * hash11(seed * 17.0);
    float t_seeded = u_time + seed * 19.0;
    float phase = mod(t_seeded, period);
    float life = 1.6;
    if (phase > life) return 0.0;
    float t = phase / life;

    float epoch = floor(t_seeded / period);
    vec2 origin = vec2(
        hash21(vec2(seed, epoch)) * 2.6 - 1.3,
        0.55 + hash21(vec2(seed + 5.0, epoch)) * 0.55
    );
    vec2 dir = normalize(vec2(
        hash21(vec2(seed + 1.0, epoch)) * 1.6 - 0.8,
        -0.7 - hash21(vec2(seed + 2.0, epoch)) * 0.6
    ));
    vec2 head = origin + dir * t * 2.1;
    vec2 tail = head - dir * 0.24;
    vec2 pa = uv - tail;
    vec2 ba = head - tail;
    float h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    float dist = length(pa - ba * h);
    float perpGlow = exp(-dist * 420.0);
    float trailFalloff = smoothstep(0.0, 1.0, h);
    float headPulse = exp(-dist * 900.0);
    float lifeFade = sin(t * 3.14159);
    return (perpGlow * trailFalloff + headPulse * 1.4) * lifeFade;
}

void main() {
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    vec2 uv = v_clip;
    uv.x *= aspect;

    vec2 d1 = vec2( u_time * 0.055,  u_time * 0.022) + u_parallax * 0.10;
    vec2 d2 = vec2(-u_time * 0.085,  u_time * 0.058) + u_parallax * 0.22;
    vec2 d3 = vec2( u_time * 0.130, -u_time * 0.095) + u_parallax * 0.40;
    float n1 = fbm(uv * 0.85 + d1);
    float n2 = fbm(uv * 2.05 + d2);
    float n3 = fbm(uv * 4.40 + d3);

    vec3 color = u_void;
    color = mix(color, u_nebula_a, pow(n1, 1.5) * 0.80);
    color = mix(color, u_nebula_b, pow(n2, 1.85) * 0.62);
    color += u_nebula_a * pow(n3, 3.0) * 0.28;

    float r = length(v_clip);
    color *= 1.0 - smoothstep(0.55, 1.40, r) * 0.85;

    {
        vec2 sgrid = uv * 75.0;
        vec2 sid = floor(sgrid);
        float sh = hash21(sid);
        float tw = 0.45 + 0.55 * sin(u_time * 2.6 + sh * 41.0);
        float mask = smoothstep(0.9935, 0.999, sh);
        color += u_stardust * mask * tw * 1.15;
    }
    {
        vec2 sgrid = uv * 135.0 + vec2(7.0, 11.0);
        vec2 sid = floor(sgrid);
        float sh = hash21(sid);
        float tw = 0.55 + 0.45 * sin(u_time * 1.1 + sh * 28.0);
        float mask = smoothstep(0.987, 0.994, sh);
        color += u_stardust * mask * tw * 0.75;
    }
    {
        vec2 sgrid = uv * 260.0 + vec2(13.0, 3.0);
        vec2 sid = floor(sgrid);
        float sh = hash21(sid);
        float tw = 0.7 + 0.3 * sin(u_time * 0.5 + sh * 15.0);
        float mask = smoothstep(0.982, 0.989, sh);
        color += u_stardust * mask * tw * 0.40;
    }

    float meteors = 0.0;
    meteors += meteor(uv, 0.31);
    meteors += meteor(uv, 1.73);
    meteors += meteor(uv, 4.29);
    meteors += meteor(uv, 7.11);
    color += vec3(1.0, 0.94, 0.78) * meteors * 1.1;

    fragColor = vec4(color, 1.0);
}
";

/// Vertex de la chacana: aplica MVP y pasa la posición de mundo al fragment.
pub const VS_CHACANA: &str = "#version 300 es
precision highp float;
in vec2 a_pos;
out vec2 v_world;
uniform mat4 u_mvp;
void main() {
    v_world = a_pos;
    gl_Position = u_mvp * vec4(a_pos, 0.0, 1.0);
}
";

/// Fragment de la chacana mística (estética dorada del logo GioSer):
/// 1. **Sol detrás**: halo gauss + corona, visible SÓLO dentro de la superficie
///    de la chacana (clip por SDF), apenas asomando por las junturas de los pasos.
/// 2. **Doble outline**: dos líneas paralelas en dorado/ámbar — la chacana se
///    siente "grabada" sobre el cielo.
/// 3. **Interior**: niebla oscura translúcida con sutiles rayos radiales
///    desde el centro (el sol los proyecta a través de la superficie).
/// 4. **Aro doble exterior**: ring fino + ring grueso (este último con marcas
///    cardinales de 3 puntos cada una, como en el logo).
///
/// Uniforms:
///   `u_time`, `u_thickness` (s), `u_center_half` (c), `u_arm_extent` (L),
///   `u_line_color` (gold rim), `u_rim_color` (gold rim oscuro),
///   `u_sun_color`, `u_sun_pulse`, `u_dark_color` (interior fill).
pub const FS_CHACANA: &str = "#version 300 es
precision highp float;
in vec2 v_world;
out vec4 fragColor;
uniform float u_time;
uniform float u_thickness;
uniform float u_center_half;
uniform float u_arm_extent;
uniform vec3  u_line_color;
uniform vec3  u_rim_color;
uniform vec3  u_sun_color;
uniform vec3  u_dark_color;
uniform float u_sun_pulse;

const float PI = 3.14159265;

float sdBox(vec2 p, vec2 b) {
    vec2 d = abs(p) - b;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

// Chacana de 2 escalones (mística clásica de Tiwanaku).
float sdChacana(vec2 p, float s, float c) {
    float d = sdBox(p, vec2(c, c));
    float hd = 0.5 * s;
    float mid1 = c + 0.5 * s;
    float hw1 = 2.0 * s;
    d = min(d, sdBox(p - vec2(0.0,  mid1), vec2(hw1, hd)));
    d = min(d, sdBox(p - vec2(0.0, -mid1), vec2(hw1, hd)));
    d = min(d, sdBox(p - vec2( mid1, 0.0), vec2(hd, hw1)));
    d = min(d, sdBox(p - vec2(-mid1, 0.0), vec2(hd, hw1)));
    float mid2 = c + 1.5 * s;
    float hw2 = 1.0 * s;
    d = min(d, sdBox(p - vec2(0.0,  mid2), vec2(hw2, hd)));
    d = min(d, sdBox(p - vec2(0.0, -mid2), vec2(hw2, hd)));
    d = min(d, sdBox(p - vec2( mid2, 0.0), vec2(hd, hw2)));
    d = min(d, sdBox(p - vec2(-mid2, 0.0), vec2(hd, hw2)));
    return d;
}

// 3 puntos pequeños en cada uno de los 4 cardinales sobre el aro grueso.
float cardinal_dots(vec2 p, float ringR, float dotSize) {
    float r = length(p);
    float ang = atan(p.y, p.x);
    // Acercamiento al aro (gauss tight en r=ringR).
    float on_ring = exp(-((r - ringR) * (r - ringR)) / (2.0 * dotSize * dotSize));
    float dots = 0.0;
    // 4 cardinales en ángulos 0, π/2, π, -π/2.
    for (int i = 0; i < 4; i++) {
        float base = float(i) * (PI * 0.5);
        // 3 puntos por cardinal, offset angular pequeño.
        for (int j = -1; j <= 1; j++) {
            float a = base + float(j) * 0.075;
            float da = ang - a;
            da = da - 2.0 * PI * floor((da + PI) / (2.0 * PI));
            dots += exp(-(da * da) / (2.0 * 0.012 * 0.012));
        }
    }
    return on_ring * dots;
}

void main() {
    vec2 p = v_world;
    float d = sdChacana(p, u_thickness, u_center_half);
    float r = length(p);

    // === SOL DETRÁS ===
    // Halo grande, sólo visible dentro de la superficie de la chacana.
    float inside = 1.0 - smoothstep(-0.004, 0.004, d);
    float sunR = u_thickness * 0.42;
    float sun = exp(-(r * r) / (2.0 * sunR * sunR));
    float corR = u_center_half * 0.75;
    float corona = exp(-(r * r) / (2.0 * corR * corR));
    float halo = sun * (1.15 + 0.20 * u_sun_pulse) + corona * (0.55 + 0.15 * u_sun_pulse);

    // Rayos radiales sutiles desde el centro, sólo visibles donde la superficie
    // de la chacana los recibe.
    float ang = atan(p.y, p.x);
    float radial = pow(abs(cos(ang * 4.0 + sin(u_time * 0.3) * 0.2)), 8.0)
                 * smoothstep(0.0, u_center_half * 0.8, r)
                 * (1.0 - smoothstep(u_center_half * 0.85, u_center_half * 1.2, r))
                 * 0.30;

    // === DOBLE OUTLINE ===
    // Línea interior (sobre la SDF=0).
    float lineW1 = 0.0085;
    float line_in = exp(-(d * d) / (2.0 * lineW1 * lineW1));
    // Línea exterior paralela, offset 0.018 hacia afuera.
    float dOff = d - 0.020;
    float lineW2 = 0.005;
    float line_out = exp(-(dOff * dOff) / (2.0 * lineW2 * lineW2));
    float line = line_in * 1.0 + line_out * 0.65;

    // Glow exterior leve.
    float glow = exp(-max(d, 0.0) * 14.0) * 0.30;

    // === AROS EXTERIORES ===
    float ringR_main = u_arm_extent * 1.45;
    float ringD_main = abs(r - ringR_main);
    float ring_main = exp(-(ringD_main * ringD_main) / (2.0 * 0.005 * 0.005));

    float ringR_inner = u_arm_extent * 1.30;
    float ringD_inner = abs(r - ringR_inner);
    float ring_inner = exp(-(ringD_inner * ringD_inner) / (2.0 * 0.003 * 0.003)) * 0.40;

    // 4 grupos de 3 puntos cardinales sobre el aro principal.
    float dots = cardinal_dots(p, ringR_main, 0.008) * 1.10;

    // === COMPOSICIÓN ===
    vec3 col = vec3(0.0);
    // Sol detrás (clip a interior).
    col += u_sun_color * halo * inside * 1.55;
    col += u_line_color * radial * inside * 0.6;
    // Niebla oscura translúcida en el interior para profundidad.
    col += u_dark_color * inside * 0.20;
    // Líneas y aros.
    col += u_line_color * line * 1.70;
    col += u_line_color * glow * 0.95;
    col += u_line_color * ring_main * 1.45;
    col += u_rim_color  * ring_inner * 1.05;
    col += u_line_color * dots * 1.85;

    float alpha = clamp(
        halo * inside + line + glow + ring_main + ring_inner + dots + inside * 0.12,
        0.0, 1.0);
    fragColor = vec4(col, alpha);
}
";

/// Geometría del quad fullscreen: dos triángulos en clip-space.
pub const FULLSCREEN_QUAD: [f32; 12] = [
    -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0,
];

/// Quad ligeramente mayor que la chacana + aros + glow.
pub fn chacana_quad(arm_extent: f32) -> [f32; 12] {
    // Aro principal vive a 1.45 * arm_extent; sumamos margen para el glow.
    let e = arm_extent * 1.70;
    [-e, -e, e, -e, e, e, -e, -e, e, e, -e, e]
}
