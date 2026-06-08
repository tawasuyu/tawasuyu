//! Fuentes GLSL ES 3.00 para Tawasuyu.
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

/// Fragment de la chacana mística (estética dorada del logo Tawasuyu):
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
uniform vec3  u_aire_color;
uniform vec3  u_fuego_color;
uniform vec3  u_tierra_color;
uniform vec3  u_agua_color;
uniform vec3  u_zodiac[12];
uniform float u_sun_pulse;
// Ciclo del cuerpo central: 0=sol, 1=luna, 2=tierra. Se interpola entre
// `u_body_a` y `u_body_b` con `u_body_blend ∈ [0, 1]`.
uniform int   u_body_a;
uniform int   u_body_b;
uniform float u_body_blend;

const float PI = 3.14159265;

float hash21c(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}
float hash11c(float n) {
    return fract(sin(n * 78.233) * 43758.5453);
}
// Value noise + fbm para superficies (lunar craters, continentes terrestres).
float vnoise_c(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash21c(i);
    float b = hash21c(i + vec2(1.0, 0.0));
    float c = hash21c(i + vec2(0.0, 1.0));
    float d = hash21c(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}
float fbm_c(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 4; i++) {
        v += a * vnoise_c(p);
        p = p * 2.03 + vec2(1.7, 9.2);
        a *= 0.5;
    }
    return v;
}

float sdBox(vec2 p, vec2 b) {
    vec2 d = abs(p) - b;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

// ===== CUERPO CENTRAL: Sol / Luna / Tierra =====
//
// Cada uno renderea dentro del cuadrado central de la chacana con su
// propia personalidad realista. El loop temporal (cambio entre cuerpos
// + transiciones graduales) lo decide el host vía u_body_a/b/blend.

vec3 render_sun(vec2 p, float r, float pulse) {
    // Núcleo brillante + corona difusa con pulso.
    float coreR = u_thickness * 0.42;
    float core  = exp(-(r * r) / (2.0 * coreR * coreR));
    float halR  = u_center_half * 0.70;
    float halo  = exp(-(r * r) / (2.0 * halR * halR));
    // Superficie boiling (plasma) muy sutil.
    float plasma = fbm_c(p * 18.0 + vec2(u_time * 0.15, u_time * 0.10)) * 0.20;
    vec3 base = u_sun_color * (core * (1.20 + 0.25 * pulse) + halo * (0.55 + 0.15 * pulse));
    return base + u_sun_color * core * plasma * 0.6;
}

vec3 render_moon(vec2 p, float r, float time) {
    float moonR = u_thickness * 1.40;
    if (r > moonR * 1.20) return vec3(0.0);

    // Normales de la esfera proyectadas en pantalla (sólo la cara visible).
    float nx = p.x / moonR;
    float ny = p.y / moonR;
    float n2 = nx * nx + ny * ny;
    float nz = sqrt(max(1.0 - n2, 0.0));
    float disk = 1.0 - smoothstep(moonR * 0.88, moonR * 1.00, r);

    // Limb darkening realista (regolito lunar dispersa más al borde).
    float limb_factor = pow(max(nz, 0.0), 0.45);

    // === SUPERFICIE: 4 capas + crater rings ===
    // Maria — mares oscuros grandes (Mare Imbrium, Tranquillitatis, etc.)
    float maria_n = fbm_c(p * 4.5 + vec2(2.3, 1.1));
    float maria = smoothstep(0.42, 0.60, maria_n);
    // Cráteres grandes (radio mid).
    float craters_mid = fbm_c(p * 13.0 + vec2(8.5, 3.2));
    // Cráteres chicos.
    float craters_small = fbm_c(p * 28.0 + vec2(17.1, 5.8));
    // Detalle fino y micro (granularidad de polvo).
    float fine = fbm_c(p * 55.0 + vec2(7.3, 11.4));
    float micro = fbm_c(p * 110.0);
    // Rims (bordes elevados de cráteres) — picos donde el fbm cruza 0.5.
    float ring_mid = pow(abs(craters_mid - 0.5) * 2.0, 3.5);
    float ring_small = pow(abs(craters_small - 0.5) * 2.0, 5.0);

    float albedo = 0.80;
    albedo += (craters_mid - 0.5) * 0.32;
    albedo += (craters_small - 0.5) * 0.22;
    albedo += (fine - 0.5) * 0.20;
    albedo += (micro - 0.5) * 0.10;
    albedo += ring_mid * 0.22;
    albedo += ring_small * 0.16;
    albedo -= maria * 0.50;
    albedo = clamp(albedo, 0.10, 1.15);

    // === FASE LUNAR CURVA ===
    // Ciclo lineal — un mes lunar comprimido en ~40 s. Avanza 0→1
    // pasando por new(0) → first-q(0.25) → full(0.5) → last-q(0.75) → new(1).
    float phase = fract(time / 40.0);
    float phi = phase * 2.0 * PI;
    // Dirección del sol relativa al observador.
    //   phi=0   → sun_dir = (0, 0, -1) ⇒ atrás de la luna (new moon).
    //   phi=π/2 → sun_dir = (+1, 0, 0) ⇒ a la derecha (first quarter).
    //   phi=π   → sun_dir = (0, 0, +1) ⇒ frente al observador (full moon).
    // Como `dot(normal, sun_dir) = nx*sin(phi) - nz*cos(phi)`, el terminador
    // resulta ser una elipse en la pantalla — curva como en la luna real.
    float lit_value = nx * sin(phi) - nz * cos(phi);
    float lit = smoothstep(-0.035, 0.035, lit_value);

    // Color superficie (gris ligeramente azulado).
    vec3 surface = vec3(0.86, 0.88, 0.94) * albedo;

    // Halo cercano al limb iluminado — luz dispersada en el regolito.
    float outer_glow = smoothstep(moonR * 1.15, moonR * 0.95, r) - disk;
    vec3 glow = vec3(0.55, 0.70, 0.95) * max(outer_glow, 0.0) * lit * 0.55;

    return surface * disk * lit * limb_factor + glow;
}

vec3 render_earth(vec2 p, float r, float time) {
    float earthR = u_thickness * 1.35;
    float disk = 1.0 - smoothstep(earthR * 0.86, earthR * 1.00, r);
    float limb_factor = sqrt(max(1.0 - (r * r) / (earthR * earthR), 0.0));
    // Rotación lenta: continentes drift horizontalmente.
    float rot = time * 0.08;
    vec2 rp = p + vec2(rot, 0.0);
    // Continentes con fbm orgánico.
    float landmass = fbm_c(rp * 7.5);
    float is_land = smoothstep(0.50, 0.56, landmass);
    vec3 ocean = vec3(0.08, 0.28, 0.52);
    vec3 land  = vec3(0.30, 0.52, 0.24);
    vec3 land_high = vec3(0.55, 0.45, 0.28); // montañas / desiertos
    land = mix(land, land_high, smoothstep(0.60, 0.78, landmass));
    vec3 surface = mix(ocean, land, is_land);
    // Casquetes polares.
    float ny = abs(p.y / max(earthR, 1e-4));
    float polar = smoothstep(0.70, 0.94, ny);
    surface = mix(surface, vec3(0.96, 0.97, 1.0), polar * 0.85);
    // Nubes flotando en otra capa rotando algo distinto.
    float clouds = fbm_c(rp * 6.0 + vec2(rot * 0.3, 0.0)) * 0.7;
    float cloud_mask = smoothstep(0.55, 0.75, clouds);
    surface = mix(surface, vec3(0.95, 0.96, 0.99), cloud_mask * 0.40);
    // Día / noche: hemisferio iluminado.
    float lit = smoothstep(-0.45, 0.55, p.x / max(earthR, 1e-4) + 0.15);
    surface *= 0.30 + 0.70 * lit;
    // Atmósfera azul en el limb (Rayleigh scattering simplificado).
    float atm_inner = smoothstep(earthR * 1.10, earthR * 0.95, r);
    float atm = max(atm_inner - disk, 0.0);
    return surface * disk * limb_factor + vec3(0.30, 0.55, 0.95) * atm * 0.55;
}

vec3 render_body(int kind, vec2 p, float r, float time, float pulse) {
    if (kind == 0) return render_sun(p, r, pulse);
    if (kind == 1) return render_moon(p, r, time);
    return render_earth(p, r, time);
}

// ===== AURA ELEMENTAL (NUBE ANCHA POR CARDINAL) =====
// Se suma a las partículas puntuales: una cobertura ancha del cuadrante
// del cardinal correspondiente, con personalidad por elemento.
vec3 element_cloud(vec2 p, vec2 tip, vec2 outward, vec3 color, float time, int kind) {
    vec2 perp = vec2(-outward.y, outward.x);
    // Centro de la nube: bien adentro del cuadrante (más allá del tip).
    vec2 cloud_center = tip + outward * 0.28;
    vec2 to_p = p - cloud_center;
    float along = dot(to_p, outward);
    float perp_d = dot(to_p, perp);
    // Anisotropía: el aura cubre TODO el cuadrante del cardinal. Sigmas
    // grandes → se solapan en las esquinas (NE/NW/SE/SW) y crean mezclas.
    float sigma_along = 0.62;
    float sigma_perp  = 0.62;
    float base = exp(-(along * along) / (2.0 * sigma_along * sigma_along)
                     -(perp_d * perp_d) / (2.0 * sigma_perp * sigma_perp));
    // Textura noise animada por elemento.
    vec2 noise_uv = (p - tip) * (3.5 + float(kind) * 0.4)
                  + vec2(time * (0.20 + float(kind) * 0.05), time * 0.10);
    float n = fbm_c(noise_uv);
    float modulation;
    if (kind == 0) {
        // AIRE: corrientes suaves que se mueven horizontalmente.
        modulation = 0.55 + 0.45 * (n * 0.7 + sin(time * 0.6 + perp_d * 4.0) * 0.3);
    } else if (kind == 1) {
        // FUEGO: lengüetazos que parpadean rápido.
        modulation = (0.40 + 0.60 * n) * (0.7 + 0.3 * sin(time * 3.2 + along * 8.0));
    } else if (kind == 2) {
        // TIERRA: densidad sólida con variación lenta.
        modulation = 0.65 + 0.35 * fbm_c(p * 4.0 + vec2(time * 0.05, 0.0));
    } else {
        // AGUA: ondulaciones grandes que viajan hacia afuera.
        modulation = 0.50 + 0.50 * sin(time * 0.9 - along * 5.0 + n * 4.0);
    }
    return color * base * max(modulation, 0.0) * 0.26;
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

// Emisor de partículas por tip cardinal. Cada elemento tiene su propio
// patrón de velocidad para sentirse vivo:
//   AIRE  → drift hacia afuera con sway lateral (viento)
//   FUEGO → asciende erráticamente con flicker amplio
//   TIERRA→ cae con gravedad y rebote sutil
//   AGUA  → ondula descendiendo (gotas que se deslizan)
//
// `element_kind`: 0=AIRE, 1=FUEGO, 2=TIERRA, 3=AGUA.
// `outward`: dirección unitaria desde el centro hacia el tip.
vec3 element_particles(vec2 p, vec2 tip, vec2 outward, vec3 color, int kind, float seed_base) {
    vec3 accum = vec3(0.0);
    vec2 perp = vec2(-outward.y, outward.x);
    // 4 partículas por tip — suficiente densidad sin saturar el costo del frag.
    for (int k = 0; k < 4; k++) {
        float seed = seed_base + float(k) * 1.31;
        float life = 1.5 + hash11c(seed * 11.0) * 0.7;
        float t_seeded = u_time + seed * 9.3;
        float phase = mod(t_seeded, life);
        float ph = phase / life; // 0..1

        // Random offsets por época (cuando el ciclo reinicia).
        float epoch = floor(t_seeded / life);
        vec2 jitter = vec2(
            hash21c(vec2(seed, epoch)) - 0.5,
            hash21c(vec2(epoch, seed)) - 0.5
        );

        // Velocidad por elemento — distinto carácter visual.
        vec2 vel;
        float sway = sin(u_time * 4.0 + seed * 7.3);
        if (kind == 0) {
            // AIRE: drift hacia afuera + sway perpendicular notable.
            vel = outward * 0.14 + perp * sway * 0.10;
        } else if (kind == 1) {
            // FUEGO: rise erratic — siempre con componente +Y (hacia arriba en el mundo),
            // independiente del tip → flamas suben.
            float erratic = sin(u_time * 6.0 + seed * 11.0) * 0.06;
            vel = outward * 0.10 + vec2(erratic, 0.18 + 0.04 * sway);
        } else if (kind == 2) {
            // TIERRA: cae — outward más componente -Y.
            vel = outward * 0.05 + vec2(0.03 * sway, -0.16);
        } else {
            // AGUA: drift outward con descenso y ondulación.
            float wave = sin(u_time * 3.2 + seed * 8.7) * 0.07;
            vel = outward * 0.12 + vec2(wave, -0.08);
        }

        vec2 pos = tip + vel * phase + jitter * 0.04;

        // Brillo gauss + envelope sinusoidal en la vida.
        float bright = sin(ph * PI);
        float dist = length(p - pos);
        float size = 0.014 + 0.006 * (kind == 1 ? sway : 0.0); // fuego pulsa
        float glow = exp(-(dist * dist) / (2.0 * size * size));
        accum += color * glow * bright;
    }
    return accum;
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

    // === CUERPO CENTRAL — SOL / LUNA / TIERRA ===
    // Sólo se computa dentro de la superficie de la chacana (perf + estética).
    float inside = 1.0 - smoothstep(-0.004, 0.004, d);
    vec3 central = vec3(0.0);
    if (inside > 0.001) {
        central = render_body(u_body_a, p, r, u_time, u_sun_pulse);
        if (u_body_blend > 0.001) {
            vec3 next_body = render_body(u_body_b, p, r, u_time, u_sun_pulse);
            central = mix(central, next_body, u_body_blend);
        }
    }

    // Rayos radiales sutiles desde el centro (el cuerpo los proyecta a
    // través de la superficie). El multiplicador disminuye cuando la luna
    // o la tierra están activas — el sol es el que más irradia.
    float ang = atan(p.y, p.x);
    float radial_mult = (u_body_a == 0) ? 1.0 : 0.35;
    if (u_body_blend > 0.001) {
        float next_mult = (u_body_b == 0) ? 1.0 : 0.35;
        radial_mult = mix(radial_mult, next_mult, u_body_blend);
    }
    float radial = pow(abs(cos(ang * 4.0 + sin(u_time * 0.3) * 0.2)), 8.0)
                 * smoothstep(0.0, u_center_half * 0.8, r)
                 * (1.0 - smoothstep(u_center_half * 0.85, u_center_half * 1.2, r))
                 * 0.30 * radial_mult;

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

    // === PARTÍCULAS POR ELEMENTO ===
    // Cada tip emite partículas con la personalidad del elemento.
    float L = u_arm_extent;
    vec3 particles = vec3(0.0);
    particles += element_particles(p, vec2(0.0,  L), vec2(0.0,  1.0), u_aire_color,   0, 0.31);
    particles += element_particles(p, vec2( L, 0.0), vec2( 1.0, 0.0), u_fuego_color,  1, 1.73);
    particles += element_particles(p, vec2(0.0, -L), vec2(0.0, -1.0), u_tierra_color, 2, 3.11);
    particles += element_particles(p, vec2(-L, 0.0), vec2(-1.0, 0.0), u_agua_color,   3, 5.97);

    // === AURA ELEMENTAL ANCHA ===
    // Una nube wide por cardinal — ocupa el cuadrante entero, no sólo la
    // punta. Las partículas puntuales aportan detalle agudo encima.
    vec3 clouds = vec3(0.0);
    clouds += element_cloud(p, vec2(0.0,  L), vec2(0.0,  1.0), u_aire_color,   u_time, 0);
    clouds += element_cloud(p, vec2( L, 0.0), vec2( 1.0, 0.0), u_fuego_color,  u_time, 1);
    clouds += element_cloud(p, vec2(0.0, -L), vec2(0.0, -1.0), u_tierra_color, u_time, 2);
    clouds += element_cloud(p, vec2(-L, 0.0), vec2(-1.0, 0.0), u_agua_color,   u_time, 3);

    // === TRAZOS ZODIACALES ===
    // 12 líneas radiales muy sutiles entre la chacana y el aro principal,
    // una por signo, con sus colores significativos (Aries=fuego rojo,
    // Tauro=tierra verde, Géminis=aire amarillo, Cáncer=agua plata, ...).
    // Aries arranca en el norte y giran en sentido horario (rueda zodiacal
    // clásica).
    vec3 zodiac = vec3(0.0);
    {
        float seg = 2.0 * PI / 12.0;
        // delta = ángulo medido desde el norte, en sentido horario, en [0, 2π).
        float delta = (PI * 0.5) - ang;
        delta = mod(delta + 8.0 * PI, 2.0 * PI);
        // Índice del signo más cercano.
        float k_round = mod(floor(delta / seg + 0.5), 12.0);
        int k = int(k_round);
        // Distancia angular al centro del segmento de ese signo.
        float center_delta = k_round * seg;
        float ang_diff = delta - center_delta;
        // Wrap a (-π, π].
        if (ang_diff >  PI) ang_diff -= 2.0 * PI;
        if (ang_diff < -PI) ang_diff += 2.0 * PI;
        float ang_dist = abs(ang_diff);

        // Línea fina, gaussiana.
        float lineW = 0.0042;
        float line = exp(-(ang_dist * ang_dist) / (2.0 * lineW * lineW));

        // Banda radial: arranca un poco fuera de la punta de la chacana y
        // termina antes del aro principal.
        float r_inner = u_arm_extent * 1.05;
        float r_outer = ringR_main * 0.96;
        float band = smoothstep(r_inner, r_inner + 0.035, r)
                   * (1.0 - smoothstep(r_outer - 0.035, r_outer, r));

        zodiac = u_zodiac[k] * line * band;
    }

    // === COMPOSICIÓN ===
    vec3 col = vec3(0.0);
    // Cuerpo central (sol / luna / tierra) clipeado al interior de la chacana.
    col += central * inside * 1.55;
    col += u_line_color * radial * inside * 0.6;
    // Niebla oscura translúcida en el interior para profundidad.
    col += u_dark_color * inside * 0.20;
    // Auras anchas de los elementos (debajo de los aros, sin clip al interior).
    col += clouds;
    // Líneas y aros.
    col += u_line_color * line * 1.70;
    col += u_line_color * glow * 0.95;
    col += u_line_color * ring_main * 1.45;
    col += u_rim_color  * ring_inner * 1.05;
    col += u_line_color * dots * 1.85;
    col += particles * 1.25;
    col += zodiac * 0.55; // muy sutil — apenas visible.

    float zodiac_lum = zodiac.r + zodiac.g + zodiac.b;
    float cloud_lum = clouds.r + clouds.g + clouds.b;
    float central_lum = central.r + central.g + central.b;
    float alpha = clamp(
        central_lum * inside * 0.7 + line + glow + ring_main + ring_inner
            + dots + inside * 0.12
            + (particles.r + particles.g + particles.b) * 0.5
            + cloud_lum * 0.45
            + zodiac_lum * 0.3,
        0.0, 1.0);
    fragColor = vec4(col, alpha);
}
";

/// Capa overlay de nubes apenas visible, dibujada DESPUÉS de la chacana
/// con `blend = SRC_ALPHA, ONE_MINUS_SRC_ALPHA` (compositing normal).
/// Dos capas FBM en parallax distinto del fondo, alpha máximo ~0.10.
/// Da sensación de niebla / cirros pasando por delante de la escena.
///
/// Uniforms: `u_resolution`, `u_time`, `u_parallax`.
pub const FS_OVERLAY_CLOUDS: &str = "#version 300 es
precision highp float;
in vec2 v_clip;
in vec2 v_uv;
out vec4 fragColor;
uniform vec2  u_resolution;
uniform float u_time;
uniform vec2  u_parallax;

float hash21o(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}
float vnoise_o(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash21o(i);
    float b = hash21o(i + vec2(1.0, 0.0));
    float c = hash21o(i + vec2(0.0, 1.0));
    float d = hash21o(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}
float fbm_o(vec2 p) {
    float v = 0.0;
    float a = 0.55;
    for (int i = 0; i < 4; i++) {
        v += a * vnoise_o(p);
        p = p * 2.10 + vec2(3.1, 9.4);
        a *= 0.55;
    }
    return v;
}

void main() {
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    vec2 uv = v_clip;
    uv.x *= aspect;

    // Parallax inverso (las nubes 'delante' se mueven al revés que las del
    // fondo) → percepción de capa más cercana.
    vec2 drift1 = vec2( u_time * 0.020,  u_time * 0.007) - u_parallax * 0.05;
    vec2 drift2 = vec2(-u_time * 0.028,  u_time * 0.013) - u_parallax * 0.09;

    // Escalas grandes (~0.55 y 1.30) = cúmulos amplios, no granulado.
    float n1 = fbm_o(uv * 0.55 + drift1);
    float n2 = fbm_o(uv * 1.30 + drift2);

    // Densidad: sólo las crestas del noise se vuelven nube. Mucha del
    // viewport queda transparente.
    float dens = smoothstep(0.55, 0.88, n1) * 0.65
               + smoothstep(0.50, 0.82, n2) * 0.35;

    // Color levemente azul-blanco; con baja densidad tira a gris.
    vec3 cloud_color = mix(vec3(0.55, 0.62, 0.74), vec3(0.90, 0.93, 1.00), dens);

    // Alpha bajísimo: 0.10 máximo (apenas visible, como pidieron).
    float alpha = dens * 0.10;
    fragColor = vec4(cloud_color, alpha);
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
