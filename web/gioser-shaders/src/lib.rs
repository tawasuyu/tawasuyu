//! Fuentes GLSL ES 3.00 para GioSer.
//!
//! Cada `const &str` es un shader completo, listo para pasar a
//! `gl.shaderSource()`. No dependemos de ningún backend; el cliente
//! decide cómo compilarlos. Convención: precision `highp float`,
//! atributo `a_pos`, varying `v_*`, uniforms `u_*`.

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

/// Fragment del fondo cósmico: nubes FBM en 3 capas, 3 estratos de
/// estrellas con titilación independiente, viñeta, y 4 meteoros
/// procedurales que cruzan el cielo periódicamente.
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

// Meteoro procedural: trazo brillante con cola, vida 1.6s, respawnea solo.
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

    // === NUBES (drift visible, 5× más rápido que la versión anterior) ===
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

    // Viñeta radial.
    float r = length(v_clip);
    color *= 1.0 - smoothstep(0.55, 1.40, r) * 0.85;

    // === ESTRELLAS — 3 estratos con titilación distinta ===
    // Brillantes, pocas, titilan rápido.
    {
        vec2 sgrid = uv * 75.0;
        vec2 sid = floor(sgrid);
        float sh = hash21(sid);
        float tw = 0.45 + 0.55 * sin(u_time * 2.6 + sh * 41.0);
        float mask = smoothstep(0.9935, 0.999, sh);
        color += u_stardust * mask * tw * 1.15;
    }
    // Medianas, densas, titilan lento.
    {
        vec2 sgrid = uv * 135.0 + vec2(7.0, 11.0);
        vec2 sid = floor(sgrid);
        float sh = hash21(sid);
        float tw = 0.55 + 0.45 * sin(u_time * 1.1 + sh * 28.0);
        float mask = smoothstep(0.987, 0.994, sh);
        color += u_stardust * mask * tw * 0.75;
    }
    // Polvo de fondo, muchas, casi sin twinkle.
    {
        vec2 sgrid = uv * 260.0 + vec2(13.0, 3.0);
        vec2 sid = floor(sgrid);
        float sh = hash21(sid);
        float tw = 0.7 + 0.3 * sin(u_time * 0.5 + sh * 15.0);
        float mask = smoothstep(0.982, 0.989, sh);
        color += u_stardust * mask * tw * 0.40;
    }

    // === METEOROS (4 procedurales, respawn independiente) ===
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

/// Fragment de la chacana mística: SDF de 2 escalones por brazo,
/// líneas glow + aro + rayos zodiacales + sol central pulsante.
/// Uniforms: `u_time`, `u_thickness` (s), `u_center_half` (c), `u_arm_extent`,
/// `u_line_color`, `u_rim_color`, `u_sun_color`, `u_sun_pulse`.
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
uniform float u_sun_pulse;

float sdBox(vec2 p, vec2 b) {
    vec2 d = abs(p) - b;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

// Chacana de 2 escalones (mística clásica): centro 2c×2c + 4 brazos
// con 2 niveles. Inner level half-width = 2s, outer (tip) = s.
float sdChacana(vec2 p, float s, float c) {
    float d = sdBox(p, vec2(c, c));
    float hd = 0.5 * s;
    // Nivel interno (más ancho, pegado al centro).
    float mid1 = c + 0.5 * s;
    float hw1 = 2.0 * s;
    d = min(d, sdBox(p - vec2(0.0,  mid1), vec2(hw1, hd))); // N
    d = min(d, sdBox(p - vec2(0.0, -mid1), vec2(hw1, hd))); // S
    d = min(d, sdBox(p - vec2( mid1, 0.0), vec2(hd, hw1))); // E
    d = min(d, sdBox(p - vec2(-mid1, 0.0), vec2(hd, hw1))); // W
    // Punta (más angosta, externa).
    float mid2 = c + 1.5 * s;
    float hw2 = 1.0 * s;
    d = min(d, sdBox(p - vec2(0.0,  mid2), vec2(hw2, hd)));
    d = min(d, sdBox(p - vec2(0.0, -mid2), vec2(hw2, hd)));
    d = min(d, sdBox(p - vec2( mid2, 0.0), vec2(hd, hw2)));
    d = min(d, sdBox(p - vec2(-mid2, 0.0), vec2(hd, hw2)));
    return d;
}

void main() {
    vec2 p = v_world;
    float d = sdChacana(p, u_thickness, u_center_half);
    float r = length(p);

    // Línea principal: gaussiana sobre el borde de la chacana.
    float lineW = 0.011;
    float line = exp(-(d * d) / (2.0 * lineW * lineW));

    // Glow exterior cae suave hacia el infinito.
    float glow = exp(-max(d, 0.0) * 8.0) * 0.55;

    // Fill interior, una niebla cyan muy tenue.
    float fill = smoothstep(0.0, -0.025, d);

    // Aro circular que envuelve la chacana (rasgo del logo).
    float ringR_outer = u_arm_extent * 1.32;
    float ringD_outer = abs(r - ringR_outer);
    float ring_outer = exp(-(ringD_outer * ringD_outer) / (2.0 * 0.008 * 0.008)) * 0.80;

    // Aro interior fino (segundo orbital).
    float ringR_inner = u_arm_extent * 1.18;
    float ringD_inner = abs(r - ringR_inner);
    float ring_inner = exp(-(ringD_inner * ringD_inner) / (2.0 * 0.0035 * 0.0035)) * 0.42;

    // Ventana radial entre arm_extent y el aro exterior — para rayos y muescas.
    float ang = atan(p.y, p.x);
    float band = smoothstep(u_arm_extent * 1.00, u_arm_extent * 1.10, r)
               * (1.0 - smoothstep(ringR_outer * 0.92, ringR_outer * 1.00, r));

    // Rayos: 12 divisiones (meses andinos / horas), modulados en el tiempo.
    float rays = pow(abs(cos(ang * 6.0)), 24.0) * band
               * (0.55 + 0.45 * sin(u_time * 0.7));

    // Marcas cardinales (4 muescas finas) — exponente alto = picos angostos.
    float card = pow(abs(cos(ang * 2.0)), 120.0) * band * 1.10;

    // Sol central: gauss tight + corona suave + pulso.
    float sunR = u_thickness * 0.50;
    float sunDist = r;
    float sun = exp(-(sunDist * sunDist) / (2.0 * sunR * sunR));
    float corR = sunR * 5.0;
    float corona = exp(-(sunDist * sunDist) / (2.0 * corR * corR)) * 0.50;
    float sunMix = sun * (1.0 + 0.2 * u_sun_pulse) + corona * (0.7 + 0.3 * u_sun_pulse);

    // Halo del centro: cuadrado oscuro detrás de la chacana para profundidad.
    float coreShadow = smoothstep(u_center_half * 0.95, u_center_half * 0.3, max(abs(p.x), abs(p.y))) * 0.20;

    vec3 col = vec3(0.0);
    col += u_line_color * line * 1.55;
    col += u_rim_color  * glow * 1.05;
    col += u_line_color * ring_outer * 1.00;
    col += u_rim_color  * ring_inner * 1.15;
    col += u_rim_color  * rays * 1.20;
    col += u_line_color * card * 1.30;
    col += u_sun_color  * sunMix * 1.45;
    col += vec3(0.05, 0.08, 0.14) * (fill + coreShadow) * 0.6;

    float alpha = clamp(
        line * 1.2 + glow + ring_outer + ring_inner + rays + card + sunMix + fill * 0.5,
        0.0, 1.0);
    fragColor = vec4(col, alpha);
}
";

/// Geometría del quad fullscreen: dos triángulos en clip-space.
pub const FULLSCREEN_QUAD: [f32; 12] = [
    -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0,
];

/// Quad ligeramente mayor que la chacana para no recortar aros ni glow.
/// `arm_extent` es la distancia centro→punta; multiplicamos por un factor
/// que cubre el aro exterior (1.32×) más halo.
pub fn chacana_quad(arm_extent: f32) -> [f32; 12] {
    let e = arm_extent * 1.65;
    [-e, -e, e, -e, e, e, -e, -e, e, e, -e, e]
}
