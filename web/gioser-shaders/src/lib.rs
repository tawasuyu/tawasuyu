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

/// Fragment del fondo cósmico: FBM en 3 capas + estrellas + viñeta.
/// Uniforms esperados: `u_resolution`, `u_time`, `u_parallax`,
/// `u_void`, `u_nebula_a`, `u_nebula_b`, `u_stardust`.
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
        p *= 2.07;
        a *= 0.55;
    }
    return v;
}
void main() {
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    vec2 uv = v_clip;
    uv.x *= aspect;

    vec2 d1 = vec2( u_time * 0.010,  u_time * 0.004) + u_parallax * 0.08;
    vec2 d2 = vec2(-u_time * 0.016,  u_time * 0.011) + u_parallax * 0.18;
    vec2 d3 = vec2( u_time * 0.024, -u_time * 0.019) + u_parallax * 0.34;

    float n1 = fbm(uv * 0.9 + d1);
    float n2 = fbm(uv * 2.1 + d2);
    float n3 = fbm(uv * 4.5 + d3);

    vec3 color = u_void;
    color = mix(color, u_nebula_a, pow(n1, 1.6) * 0.70);
    color = mix(color, u_nebula_b, pow(n2, 2.0) * 0.55);
    color += u_nebula_a * pow(n3, 3.2) * 0.22;

    float r = length(v_clip);
    color *= 1.0 - smoothstep(0.55, 1.35, r) * 0.85;

    // Estrellas brillantes (pocas, titilan).
    vec2 sgrid = uv * 90.0;
    vec2 sid = floor(sgrid);
    float sh = hash21(sid);
    float twinkle = 0.4 + 0.6 * sin(u_time * 1.7 + sh * 28.0);
    float starMask = smoothstep(0.997, 0.9985, sh);
    color += u_stardust * starMask * twinkle * 0.95;

    // Polvo (muchas, débiles).
    vec2 dgrid = uv * 220.0;
    float dh = hash21(floor(dgrid));
    float dustMask = smoothstep(0.985, 0.992, dh);
    color += u_stardust * dustMask * 0.25;

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

/// Fragment de la chacana: SDF de la cruz escalonada + glow + aro + sol pulsante.
/// Uniforms: `u_time`, `u_thickness`, `u_arm_extent`,
/// `u_line_color`, `u_rim_color`, `u_sun_color`, `u_sun_pulse`.
pub const FS_CHACANA: &str = "#version 300 es
precision highp float;
in vec2 v_world;
out vec4 fragColor;
uniform float u_time;
uniform float u_thickness;
uniform float u_arm_extent;
uniform vec3  u_line_color;
uniform vec3  u_rim_color;
uniform vec3  u_sun_color;
uniform float u_sun_pulse;

float sdBox(vec2 p, vec2 b) {
    vec2 d = abs(p) - b;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}
float sdChacana(vec2 p, float s, float L) {
    float s2 = s * 2.0;
    float halfArm = max((L - s2) * 0.5, 0.0);
    float armOff  = s2 + halfArm;
    float d = sdBox(p, vec2(s, s));
    d = min(d, sdBox(p - vec2(0.0,  1.5 * s), vec2(s2, 0.5 * s)));
    d = min(d, sdBox(p - vec2(0.0, -1.5 * s), vec2(s2, 0.5 * s)));
    d = min(d, sdBox(p - vec2( 1.5 * s, 0.0), vec2(0.5 * s, s2)));
    d = min(d, sdBox(p - vec2(-1.5 * s, 0.0), vec2(0.5 * s, s2)));
    d = min(d, sdBox(p - vec2(0.0,  armOff), vec2(s, halfArm)));
    d = min(d, sdBox(p - vec2(0.0, -armOff), vec2(s, halfArm)));
    d = min(d, sdBox(p - vec2( armOff, 0.0), vec2(halfArm, s)));
    d = min(d, sdBox(p - vec2(-armOff, 0.0), vec2(halfArm, s)));
    return d;
}

void main() {
    vec2 p = v_world;
    float d = sdChacana(p, u_thickness, u_arm_extent);

    // Línea: gaussiana alrededor del borde.
    float lineW = 0.013;
    float line = exp(-(d * d) / (2.0 * lineW * lineW));

    // Glow exterior cae más suave.
    float glow = exp(-max(d, 0.0) * 7.5) * 0.55;

    // Fill interior tenue (ligera niebla cyan dentro).
    float fill = smoothstep(0.0, -0.025, d);

    // Aro exterior: gran círculo que envuelve la chacana.
    float ringR = u_arm_extent * 1.18;
    float ringD = abs(length(p) - ringR);
    float ringW = 0.008;
    float ring = exp(-(ringD * ringD) / (2.0 * ringW * ringW)) * 0.75;

    // Rayos sutiles (12 divisiones del círculo, como husillos del calendario).
    float ang = atan(p.y, p.x);
    float rays = pow(abs(cos(ang * 6.0)), 80.0)
               * smoothstep(u_arm_extent * 1.05, ringR * 0.97, length(p))
               * (0.18 + 0.10 * sin(u_time * 0.6));

    // Sol central: gauss tight + corona suave + pulso.
    float sunR = u_thickness * 0.55;
    float sunDist = length(p);
    float sun = exp(-(sunDist * sunDist) / (2.0 * sunR * sunR));
    float corR = sunR * 4.5;
    float corona = exp(-(sunDist * sunDist) / (2.0 * corR * corR)) * 0.45;
    float sunMix = sun + corona * (0.75 + 0.25 * u_sun_pulse);

    vec3 col = vec3(0.0);
    col += u_line_color * line * 1.45;
    col += u_rim_color  * glow * 1.05;
    col += u_line_color * ring * 0.95;
    col += u_rim_color  * rays * 1.40;
    col += u_sun_color  * sunMix * 1.35;
    col += vec3(0.04, 0.06, 0.12) * fill * 0.55;

    float alpha = clamp(line * 1.2 + glow + ring + rays + sunMix + fill * 0.5, 0.0, 1.0);
    fragColor = vec4(col, alpha);
}
";

/// Geometría del quad fullscreen: dos triángulos en clip-space.
pub const FULLSCREEN_QUAD: [f32; 12] = [
    -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0,
];

/// Quad ligeramente mayor que la chacana para no recortar el glow ni el aro.
pub fn chacana_quad(arm_extent: f32) -> [f32; 12] {
    let e = arm_extent * 1.45;
    [-e, -e, e, -e, e, e, -e, -e, e, e, -e, e]
}
