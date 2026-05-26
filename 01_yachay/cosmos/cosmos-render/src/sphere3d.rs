//! `sphere3d` — la esfera celeste en 3D, proyectada a primitivas 2D.
//!
//! La estrategia es de alambre: la esfera celeste es un objeto de
//! **alambre** —círculos máximos y puntos—, y eso se proyecta a
//! software con trigonometría pura. Cada superficie (canvas Llimphi
//! nativo, SVG del cliente web) ya
//! sabe traducir un [`DrawCommand`] (línea, círculo, texto); este
//! módulo solo decide DÓNDE cae cada trazo. Resultado: una esfera
//! celeste real, rotable, sin una sola línea de GPU.
//!
//! ## Marco de coordenadas — la eclíptica como plano de referencia
//!
//! El plano de la eclíptica es el plano `z = 0`. El eje `x` apunta al
//! 0° de Aries (el punto vernal). Una longitud eclíptica `λ` —con
//! latitud eclíptica ≈ 0, lo cual vale para los planetas— es el punto
//! unitario `(cos λ, sin λ, 0)`. El polo norte de la eclíptica es
//! `(0, 0, 1)`.
//!
//! El **ecuador celeste** es ese mismo círculo inclinado por la
//! oblicuidad ε ≈ 23.44° alrededor del eje `x`: los dos se cruzan en
//! los equinoccios, exactamente como en el cielo. Ver esa inclinación
//! —imposible en la rueda 2D— es el corazón de esta vista.
//!
//! ## Lo que esta primera entrega NO hace todavía
//!
//! El **horizonte local** (y con él el día/noche: qué planetas están
//! sobre el horizonte) necesita la latitud geográfica del lugar, que
//! hoy no viaja en el [`RenderModel`]. Queda para una segunda capa.

use serde::{Deserialize, Serialize};

use crate::draw::{planet_unicode_with_retro, sign_unicode, DrawCommand, Rgba, TextAnchor};
use crate::palette::Palette;
use crate::{LayerKind, RenderModel};

/// Oblicuidad media de la eclíptica, en grados. Varía ~0.013°/siglo —
/// despreciable para una vista de alambre, así que se fija.
pub const OBLICUIDAD_DEG: f32 = 23.4393;

const SIGN_NAMES: [&str; 12] = [
    "aries", "taurus", "gemini", "cancer", "leo", "virgo", "libra",
    "scorpio", "sagittarius", "capricorn", "aquarius", "pisces",
];

// =====================================================================
// Cámara — cómo se orienta la esfera frente al observador
// =====================================================================

/// Orientación de la esfera frente a la cámara. El usuario la muta
/// arrastrando: `yaw` gira alrededor del eje polar de la eclíptica,
/// `pitch` inclina la cámara hacia arriba o abajo.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SphereView {
    /// Giro alrededor del eje polar de la eclíptica, en grados.
    pub yaw_deg: f32,
    /// Inclinación de la cámara, en grados. Un `pitch` negativo mira la
    /// esfera desde el norte hacia abajo, dejando la eclíptica como una
    /// elipse abierta en vez de una raya de canto.
    pub pitch_deg: f32,
}

impl Default for SphereView {
    fn default() -> Self {
        // Tres-cuartos desde arriba: la eclíptica se ve como un aro
        // ancho y la inclinación del ecuador se lee de inmediato.
        Self { yaw_deg: 26.0, pitch_deg: -64.0 }
    }
}

/// Opciones de composición de la esfera.
#[derive(Debug, Clone)]
pub struct SphereOpts {
    /// Lado en px del cuadrado contenedor.
    pub size: f32,
    pub palette: Palette,
    /// Oblicuidad de la eclíptica en grados (ver [`OBLICUIDAD_DEG`]).
    pub obliquity_deg: f32,
    /// Rejilla de meridianos y paralelos eclípticos.
    pub show_grid: bool,
    /// El ecuador celeste y el eje de la Tierra.
    pub show_equator: bool,
    /// Los cuerpos natales sobre la eclíptica.
    pub show_bodies: bool,
    /// Los glifos y divisiones de los signos.
    pub show_signs: bool,
    /// El horizonte local, el cénit del observador y el meridiano.
    /// Necesita `RenderModel::geo_latitude_deg`.
    pub show_horizon: bool,
    /// El cielo de fondo: campo de estrellas + Vía Láctea. Solo se
    /// dibuja en tema oscuro (en papel rompería la metáfora de imprenta).
    pub show_sky: bool,
    /// La Tierra interior — un globo pequeño, transparente, con los
    /// continentes esquemáticos y el observador marcado en su lugar.
    pub show_earth: bool,
    /// Las figuras de las 88 constelaciones (catálogo d3-celestial).
    pub show_constellations: bool,
}

impl Default for SphereOpts {
    fn default() -> Self {
        Self {
            size: 600.0,
            palette: Palette::dark(),
            obliquity_deg: OBLICUIDAD_DEG,
            show_grid: true,
            show_equator: true,
            show_bodies: true,
            show_signs: true,
            show_horizon: true,
            show_sky: true,
            show_earth: true,
            show_constellations: true,
        }
    }
}

// =====================================================================
// Vector 3D y proyección
// =====================================================================

#[derive(Debug, Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    fn scale(self, k: f32) -> Self {
        Self::new(self.x * k, self.y * k, self.z * k)
    }
    fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    fn normalized(self) -> Vec3 {
        let len = (self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if len < 1e-9 {
            self
        } else {
            self.scale(1.0 / len)
        }
    }
}

/// Un punto ya proyectado a la pantalla, con su profundidad conservada
/// para ordenar de atrás hacia adelante y atenuar el hemisferio lejano.
#[derive(Debug, Clone, Copy)]
struct Projected {
    x: f32,
    y: f32,
    /// `+` hacia el observador (frente), `−` lejos (fondo).
    depth: f32,
}

/// Proyector ortográfico: gira un punto por la cámara (`yaw` alrededor
/// del eje polar, `pitch` alrededor del eje horizontal de pantalla) y
/// lo aplana a coordenadas de pantalla.
struct Projector {
    ys: f32,
    yc: f32,
    ps: f32,
    pc: f32,
    ox: f32,
    oy: f32,
    rad: f32,
}

impl Projector {
    fn new(view: &SphereView, ox: f32, oy: f32, rad: f32) -> Self {
        let (ys, yc) = view.yaw_deg.to_radians().sin_cos();
        let (ps, pc) = view.pitch_deg.to_radians().sin_cos();
        Self { ys, yc, ps, pc, ox, oy, rad }
    }

    fn project(&self, p: Vec3) -> Projected {
        // 1) yaw alrededor del eje Z (polar de la eclíptica).
        let x1 = p.x * self.yc - p.y * self.ys;
        let y1 = p.x * self.ys + p.y * self.yc;
        let z1 = p.z;
        // 2) pitch alrededor del eje X (horizontal de pantalla).
        let x2 = x1;
        let y2 = y1 * self.pc - z1 * self.ps;
        let z2 = y1 * self.ps + z1 * self.pc;
        // 3) ortográfica: la pantalla tiene la Y hacia abajo.
        Projected {
            x: self.ox + self.rad * x2,
            y: self.oy - self.rad * y2,
            depth: z2,
        }
    }
}

/// Punto unitario sobre la eclíptica a la longitud `deg`.
fn eclip(deg: f32) -> Vec3 {
    let (s, c) = deg.to_radians().sin_cos();
    Vec3::new(c, s, 0.0)
}

/// Punto unitario a longitud y latitud eclípticas (grados) — para los
/// cuerpos que NO yacen sobre la eclíptica, como las estrellas fijas.
fn eclip_latlon(lon_deg: f32, lat_deg: f32) -> Vec3 {
    let (sl, cl) = lon_deg.to_radians().sin_cos();
    let (sb, cb) = lat_deg.to_radians().sin_cos();
    Vec3::new(cb * cl, cb * sl, sb)
}

/// Rota `p` alrededor del eje X (la línea de los equinoccios).
fn rot_x(p: Vec3, ang_rad: f32) -> Vec3 {
    let (s, c) = ang_rad.sin_cos();
    Vec3::new(p.x, p.y * c - p.z * s, p.y * s + p.z * c)
}

/// Atenuación por profundidad: el frente brilla pleno, el fondo se
/// apaga hasta ~0.30 para que el ojo lea el volumen de la esfera.
fn depth_alpha(depth: f32) -> f32 {
    0.30 + 0.70 * ((depth + 1.0) * 0.5).clamp(0.0, 1.0)
}

/// `color` con su alpha modulada por la profundidad.
fn dim(color: Rgba, depth: f32) -> Rgba {
    color.with_alpha(color.a * depth_alpha(depth))
}

// =====================================================================
// Generadores de círculos
// =====================================================================

/// El círculo de la eclíptica (z = 0), `n` puntos.
fn ring_points(n: usize) -> Vec<Vec3> {
    (0..n)
        .map(|i| eclip((i as f32) / (n as f32) * 360.0))
        .collect()
}

/// Un meridiano eclíptico: círculo máximo por ambos polos a la
/// longitud `lon0`.
fn meridian_points(lon0: f32, n: usize) -> Vec<Vec3> {
    let (ls, lc) = lon0.to_radians().sin_cos();
    (0..n)
        .map(|i| {
            let a = (i as f32) / (n as f32) * std::f32::consts::TAU;
            let (asin, acos) = a.sin_cos();
            Vec3::new(acos * lc, acos * ls, asin)
        })
        .collect()
}

/// Un paralelo eclíptico: círculo menor a la latitud `beta`.
fn parallel_points(beta: f32, n: usize) -> Vec<Vec3> {
    let (bs, bc) = beta.to_radians().sin_cos();
    (0..n)
        .map(|i| {
            let lon = (i as f32) / (n as f32) * 360.0;
            let (ls, lc) = lon.to_radians().sin_cos();
            Vec3::new(bc * lc, bc * ls, bs)
        })
        .collect()
}

/// Sombreado del cuerpo de la esfera: un disco base sólido, un
/// degradado que aclara hacia el centro y un brillo especular
/// desplazado hacia la luz (arriba-izquierda). Da volumen sin
/// gradientes nativos — solo discos translúcidos que se acumulan.
fn add_sphere_shading(
    items: &mut Vec<(f32, DrawCommand)>,
    pal: &Palette,
    center: f32,
    rad: f32,
) {
    let (base, glow, highlight) = if pal.is_dark {
        (
            Rgba::opaque(0.12, 0.14, 0.24),
            Rgba::opaque(0.34, 0.40, 0.60),
            Rgba::opaque(0.62, 0.68, 0.88),
        )
    } else {
        (
            Rgba::opaque(0.82, 0.86, 0.93),
            Rgba::opaque(1.0, 1.0, 1.0),
            Rgba::opaque(1.0, 1.0, 1.0),
        )
    };
    // Disco base — uniforme, le da cuerpo sólido a la esfera.
    items.push((
        -99.0,
        DrawCommand::Circle {
            cx: center,
            cy: center,
            r: rad,
            stroke: None,
            fill: Some(base.with_alpha(0.55)),
            stroke_w: 0.0,
        },
    ));
    // Degradado: anillos concéntricos que se acumulan hacia el centro.
    const GLOW: usize = 12;
    for i in 0..GLOW {
        let t = i as f32 / (GLOW - 1) as f32;
        items.push((
            -98.0 + t * 1.5,
            DrawCommand::Circle {
                cx: center,
                cy: center,
                r: rad * (0.95 - 0.95 * t),
                stroke: None,
                fill: Some(glow.with_alpha(0.028)),
                stroke_w: 0.0,
            },
        ));
    }
    // Brillo especular desplazado hacia la luz — tenue: la luminosidad
    // viva la reparte la Vía Láctea, que sí gira con la esfera.
    let hx = center - rad * 0.34;
    let hy = center - rad * 0.34;
    const HALO: usize = 6;
    for i in 0..HALO {
        let t = i as f32 / (HALO - 1) as f32;
        items.push((
            -95.0 + t * 0.5,
            DrawCommand::Circle {
                cx: hx,
                cy: hy,
                r: rad * 0.5 * (1.0 - t),
                stroke: None,
                fill: Some(highlight.with_alpha(0.018)),
                stroke_w: 0.0,
            },
        ));
    }
    // Contorno nítido del limbo, encima del sombreado.
    items.push((
        -94.0,
        DrawCommand::Circle {
            cx: center,
            cy: center,
            r: rad,
            stroke: Some(pal.fg_muted.with_alpha(0.32)),
            fill: None,
            stroke_w: 1.0,
        },
    ));
}

/// Proyecta una polilínea cerrada y empuja un `Line` por segmento, con
/// la profundidad como clave de orden y la atenuación ya aplicada.
fn add_loop(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    pts: &[Vec3],
    color: Rgba,
    width: f32,
) {
    let n = pts.len();
    for i in 0..n {
        let a = proj.project(pts[i]);
        let b = proj.project(pts[(i + 1) % n]);
        let d = (a.depth + b.depth) * 0.5;
        items.push((
            d,
            DrawCommand::Line {
                x1: a.x,
                y1: a.y,
                x2: b.x,
                y2: b.y,
                color: dim(color, d),
                width,
                dash: None,
            },
        ));
    }
}

/// Proyecta una polilínea ABIERTA y empuja un `Line` por segmento.
fn add_path(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    pts: &[Vec3],
    color: Rgba,
    width: f32,
) {
    for i in 0..pts.len().saturating_sub(1) {
        let a = proj.project(pts[i]);
        let b = proj.project(pts[i + 1]);
        let d = (a.depth + b.depth) * 0.5;
        items.push((
            d,
            DrawCommand::Line {
                x1: a.x,
                y1: a.y,
                x2: b.x,
                y2: b.y,
                color: dim(color, d),
                width,
                dash: None,
            },
        ));
    }
}

/// Dibuja las figuras de las 88 constelaciones: cada trazo une estrellas
/// reales del catálogo (un punto por vértice), y el nombre va en el
/// centroide. Capa tenue — referencia, no protagonista.
fn add_constellations(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    eps: f32,
    size: f32,
    pal: &Palette,
) {
    let line_col = pal.fg_muted.with_alpha(0.42);
    let star = Rgba::opaque(0.92, 0.95, 1.0);
    for fig in crate::constellations_data::FIGURAS {
        let (mut sx, mut sy, mut sz, mut n) = (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
        for path in fig.paths {
            let pts: Vec<Vec3> = path
                .iter()
                .map(|&(ra, dec)| rot_x(equatorial_dir(ra, dec), eps))
                .collect();
            add_path(items, proj, &pts, line_col, 0.7);
            for v in &pts {
                sx += v.x;
                sy += v.y;
                sz += v.z;
                n += 1.0;
                let p = proj.project(*v);
                items.push((
                    p.depth - 0.01,
                    DrawCommand::Circle {
                        cx: p.x,
                        cy: p.y,
                        r: size * 0.0017,
                        stroke: None,
                        fill: Some(star.with_alpha(0.70 * depth_alpha(p.depth))),
                        stroke_w: 0.0,
                    },
                ));
            }
        }
        if n > 0.0 {
            let c = Vec3::new(sx / n, sy / n, sz / n).normalized();
            let lp = proj.project(c);
            items.push((
                lp.depth + 0.001,
                DrawCommand::Text {
                    x: lp.x,
                    y: lp.y,
                    content: fig.nombre.into(),
                    color: pal.fg_muted.with_alpha(0.42 * depth_alpha(lp.depth)),
                    size: size * 0.0135,
                    anchor: TextAnchor::Middle,
                },
            ));
        }
    }
}

/// Los `n` puntos de un círculo máximo perpendicular a `normal`.
fn great_circle_perp(normal: Vec3, n: usize) -> Vec<Vec3> {
    let z = normal.normalized();
    // Una referencia que no sea casi-paralela a `z`.
    let r = if z.z.abs() < 0.9 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let u = z.cross(r).normalized();
    let v = z.cross(u);
    (0..n)
        .map(|i| {
            let t = (i as f32) / (n as f32) * std::f32::consts::TAU;
            let (s, c) = t.sin_cos();
            Vec3::new(u.x * c + v.x * s, u.y * c + v.y * s, u.z * c + v.z * s)
        })
        .collect()
}

/// RAMC — ascensión recta del Medio Cielo, en grados: la AR del punto
/// eclíptico del MC (latitud eclíptica 0).
fn ramc_deg(mc_deg: f32, eps_rad: f32) -> f32 {
    let lmc = mc_deg.to_radians();
    (lmc.sin() * eps_rad.cos())
        .atan2(lmc.cos())
        .to_degrees()
}

/// El cénit del observador en el marco eclíptico — el punto del cielo
/// justo sobre su cabeza. Tiene declinación `φ` (la latitud geográfica)
/// y AR `RAMC`, y eso se lleva del marco ecuatorial al eclíptico
/// rotando por la oblicuidad.
fn zenith_ecliptic(lat_deg: f32, mc_deg: f32, eps_rad: f32) -> Vec3 {
    let phi = lat_deg.to_radians();
    let ramc = ramc_deg(mc_deg, eps_rad).to_radians();
    let (sphi, cphi) = phi.sin_cos();
    let (sr, cr) = ramc.sin_cos();
    rot_x(Vec3::new(cphi * cr, cphi * sr, sphi), eps_rad)
}

/// Marca un punto notable de la esfera: disco + etiqueta, y un anillo
/// extra si es `prominent`.
fn add_point_marker(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    pos: Vec3,
    color: Rgba,
    size: f32,
    label: &str,
    prominent: bool,
) {
    let p = proj.project(pos);
    let c = dim(color, p.depth);
    let r = if prominent { size * 0.013 } else { size * 0.008 };
    items.push((
        p.depth + 0.001,
        DrawCommand::Circle {
            cx: p.x,
            cy: p.y,
            r,
            stroke: Some(c),
            fill: Some(c.with_alpha(c.a * 0.40)),
            stroke_w: 1.4,
        },
    ));
    if prominent {
        items.push((
            p.depth + 0.001,
            DrawCommand::Circle {
                cx: p.x,
                cy: p.y,
                r: r * 1.95,
                stroke: Some(c.with_alpha(c.a * 0.55)),
                fill: None,
                stroke_w: 1.0,
            },
        ));
    }
    let lp = proj.project(pos.scale(1.13));
    items.push((
        lp.depth + 0.002,
        DrawCommand::Text {
            x: lp.x,
            y: lp.y,
            content: label.into(),
            color: dim(color, lp.depth),
            size: size * 0.019,
            anchor: TextAnchor::Middle,
        },
    ));
}

// --- Cielo de fondo: estrellas decorativas + Vía Láctea --------------

/// Polo norte galáctico (J2000): AR 192.859°, Dec +27.128° — constante
/// estándar IAU que fija el plano de la Vía Láctea.
const GAL_POLE_RA: f32 = 192.859;
const GAL_POLE_DEC: f32 = 27.128;
/// Centro galáctico (Sgr A*, J2000): AR 266.405°, Dec −28.936°. Hacia
/// ahí la Vía Láctea es más brillante.
const GAL_CENTER_RA: f32 = 266.405;
const GAL_CENTER_DEC: f32 = -28.936;

/// Hash entero → f32 en [0,1). Determinista (variante de splitmix32):
/// la misma entrada da siempre el mismo valor, así el campo de
/// estrellas no titila ni salta entre frames.
fn hash01(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    (x as f32) / (u32::MAX as f32)
}

/// Punto uniforme sobre la esfera unidad a partir de dos uniformes.
fn sphere_point(u1: f32, u2: f32) -> Vec3 {
    let z = 2.0 * u1 - 1.0;
    let rho = (1.0 - z * z).max(0.0).sqrt();
    let theta = std::f32::consts::TAU * u2;
    Vec3::new(rho * theta.cos(), rho * theta.sin(), z)
}

/// Vector unitario de una dirección ecuatorial (AR, Dec en grados).
fn equatorial_dir(ra_deg: f32, dec_deg: f32) -> Vec3 {
    let (sr, cr) = ra_deg.to_radians().sin_cos();
    let (sd, cd) = dec_deg.to_radians().sin_cos();
    Vec3::new(cd * cr, cd * sr, sd)
}

/// Empuja una estrella: un disco diminuto con brillo y un leve tinte
/// (azulado o cálido). Va detrás de la rejilla pero delante del
/// sombreado — un fondo de planetario.
fn push_star(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    size: f32,
    pos: Vec3,
    brightness: f32,
    tint: f32,
) {
    let p = proj.project(pos);
    let bright = brightness * brightness; // sesga hacia las tenues
    let r = size * (0.0011 + 0.0026 * bright);
    let alpha = (0.20 + 0.62 * bright) * depth_alpha(p.depth);
    let col = if tint < 0.22 {
        Rgba { r: 0.74, g: 0.81, b: 1.0, a: alpha }
    } else if tint > 0.86 {
        Rgba { r: 1.0, g: 0.86, b: 0.72, a: alpha }
    } else {
        Rgba { r: 0.95, g: 0.96, b: 1.0, a: alpha }
    };
    items.push((
        p.depth - 3.0,
        DrawCommand::Circle {
            cx: p.x,
            cy: p.y,
            r,
            stroke: None,
            fill: Some(col),
            stroke_w: 0.0,
        },
    ));
}

/// El cielo de fondo: un campo de estrellas isótropo —decorativo, no un
/// catálogo real— más una sobredensidad de estrellas tenues a lo largo
/// del plano galáctico, que dibuja la Vía Láctea. Ambos giran con la
/// esfera, así que delatan su rotación de un vistazo.
fn add_starfield(items: &mut Vec<(f32, DrawCommand)>, proj: &Projector, size: f32, eps: f32) {
    const FONDO: u32 = 210;
    for i in 0..FONDO {
        let pos = sphere_point(hash01(i * 3), hash01(i * 3 + 1));
        push_star(items, proj, size, pos, hash01(i * 3 + 2), hash01(i * 7 + 1));
    }
    // Vía Láctea — el plano galáctico ubicado con el polo galáctico real.
    let gpole = rot_x(equatorial_dir(GAL_POLE_RA, GAL_POLE_DEC), eps);
    let geq = great_circle_perp(gpole, 256);
    const VIA: u32 = 240;
    for i in 0..VIA {
        let s = 9001 + i;
        let idx = (hash01(s * 5) * geq.len() as f32) as usize % geq.len();
        let on_eq = geq[idx];
        // Latitud galáctica pequeña, concentrada cerca de 0 — producto
        // de dos uniformes centrados → densa en el plano.
        let u = hash01(s * 5 + 1) - 0.5;
        let v = hash01(s * 5 + 2) - 0.5;
        let b = (u * v * 4.0 * 13.0).to_radians();
        let (sb, cb) = b.sin_cos();
        let pos = Vec3::new(
            on_eq.x * cb + gpole.x * sb,
            on_eq.y * cb + gpole.y * sb,
            on_eq.z * cb + gpole.z * sb,
        );
        push_star(items, proj, size, pos, hash01(s * 5 + 3) * 0.55, hash01(s * 5 + 4));
    }
}

/// El resplandor difuso de la Vía Láctea — una luminosidad repartida a
/// lo largo del plano galáctico, no un brillo fijo a la pantalla. Gira
/// con la esfera. Es más intensa hacia el centro galáctico (en
/// Sagitario, como en el cielo real) y, si hay horizonte, se atenúa en
/// la parte que queda bajo tierra esa noche — la franja como se ve
/// desde la Tierra ese día.
fn add_milky_way_glow(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    eps: f32,
    size: f32,
    zenith: Option<Vec3>,
) {
    let gpole = rot_x(equatorial_dir(GAL_POLE_RA, GAL_POLE_DEC), eps);
    let gcenter = rot_x(equatorial_dir(GAL_CENTER_RA, GAL_CENTER_DEC), eps);
    let band = Rgba::opaque(0.78, 0.82, 0.96);
    for p3 in great_circle_perp(gpole, 54) {
        // Más brillo hacia el centro galáctico.
        let toward = (p3.dot(gcenter) * 0.5 + 0.5).clamp(0.0, 1.0);
        let bright = 0.28 + 0.72 * toward * toward;
        // Atenuada bajo el horizonte local (no se ve esa noche).
        let vis = match zenith {
            Some(z) if p3.dot(z) < 0.0 => 0.40,
            _ => 1.0,
        };
        let p = proj.project(p3);
        items.push((
            p.depth - 4.0,
            DrawCommand::Circle {
                cx: p.x,
                cy: p.y,
                r: size * 0.045,
                stroke: None,
                fill: Some(band.with_alpha(0.030 * bright * vis * depth_alpha(p.depth))),
                stroke_w: 0.0,
            },
        ));
    }
}

// --- Estrellas fijas notables ----------------------------------------

/// Latitud eclíptica (grados, J2000) de las estrellas fijas notables
/// que emite el motor. La latitud apenas cambia con la precesión, así
/// que se fija aquí; la **longitud** —la coordenada astrológicamente
/// viva, que sí precesiona— la calcula el motor
/// (`build_fixed_stars_overlay`) y llega en el `Glyph`. Valores de
/// catálogo estándar, precisión ~0.5° (de sobra para el alambre).
fn fixed_star_latitude(name: &str) -> f32 {
    match name {
        "Regulus" => 0.47,
        "Spica" => -2.06,
        "Antares" => -4.57,
        "Aldebaran" => -5.47,
        "Pollux" => 6.68,
        "Algol" => 22.43,
        "Fomalhaut" => -21.14,
        "Sirius" => -39.61,
        "Vega" => 61.73,
        _ => 0.0,
    }
}

/// Dibuja una estrella fija: un disco brillante con destello de cuatro
/// rayos y su nombre.
fn add_fixed_star(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    pos: Vec3,
    size: f32,
    name: &str,
    pal: &Palette,
) {
    let p = proj.project(pos);
    let glow = Rgba::opaque(1.0, 0.96, 0.84);
    let c = dim(glow, p.depth);
    items.push((
        p.depth + 0.004,
        DrawCommand::Circle {
            cx: p.x,
            cy: p.y,
            r: size * 0.006,
            stroke: None,
            fill: Some(c),
            stroke_w: 0.0,
        },
    ));
    let ray = size * 0.018;
    let thin = c.with_alpha(c.a * 0.8);
    for (dx, dy) in [(ray, 0.0), (-ray, 0.0), (0.0, ray), (0.0, -ray)] {
        items.push((
            p.depth + 0.004,
            DrawCommand::Line {
                x1: p.x,
                y1: p.y,
                x2: p.x + dx,
                y2: p.y + dy,
                color: thin,
                width: 0.9,
                dash: None,
            },
        ));
    }
    let lp = proj.project(pos.scale(1.10));
    items.push((
        lp.depth + 0.005,
        DrawCommand::Text {
            x: lp.x,
            y: lp.y,
            content: name.into(),
            color: dim(pal.fg_text, lp.depth),
            size: size * 0.017,
            anchor: TextAnchor::Middle,
        },
    ));
}

// --- Tierra interior ------------------------------------------------

/// Contornos continentales **esquemáticos** (lat, lon en grados) — solo
/// referenciales, trazos muy gruesos para la Tierra interior. NO son un
/// mapa de precisión; dan el «ahí está tu continente» y nada más.
const CONTINENTES: &[&[(f32, f32)]] = &[
    // África
    &[
        (35.0, -6.0), (37.0, 10.0), (33.0, 22.0), (31.0, 32.0), (12.0, 43.0),
        (11.0, 51.0), (-4.0, 40.0), (-26.0, 33.0), (-34.0, 26.0), (-34.0, 19.0),
        (-18.0, 12.0), (0.0, 9.0), (5.0, -4.0), (11.0, -15.0), (21.0, -17.0),
        (28.0, -13.0),
    ],
    // Sudamérica
    &[
        (12.0, -72.0), (11.0, -61.0), (5.0, -52.0), (-5.0, -35.0), (-23.0, -43.0),
        (-34.0, -54.0), (-52.0, -69.0), (-55.0, -67.0), (-42.0, -74.0),
        (-18.0, -70.0), (-5.0, -81.0), (2.0, -79.0), (8.0, -77.0),
    ],
    // Norteamérica
    &[
        (70.0, -160.0), (71.0, -125.0), (68.0, -95.0), (63.0, -78.0),
        (47.0, -53.0), (45.0, -67.0), (30.0, -81.0), (25.0, -81.0),
        (20.0, -97.0), (23.0, -110.0), (34.0, -120.0), (48.0, -125.0),
        (60.0, -148.0),
    ],
    // Eurasia
    &[
        (36.0, -9.0), (43.0, -9.0), (58.0, 5.0), (71.0, 26.0), (73.0, 80.0),
        (73.0, 140.0), (66.0, 180.0), (53.0, 141.0), (40.0, 130.0), (30.0, 122.0),
        (22.0, 110.0), (9.0, 105.0), (8.0, 77.0), (21.0, 72.0), (25.0, 57.0),
        (13.0, 45.0), (30.0, 33.0), (41.0, 28.0), (38.0, 15.0), (40.0, 0.0),
    ],
    // Australia
    &[
        (-11.0, 131.0), (-12.0, 142.0), (-25.0, 153.0), (-38.0, 147.0),
        (-35.0, 138.0), (-32.0, 116.0), (-22.0, 114.0), (-14.0, 127.0),
    ],
    // Antártida (casquete polar aproximado)
    &[
        (-72.0, -180.0), (-70.0, -120.0), (-73.0, -60.0), (-70.0, 0.0),
        (-73.0, 60.0), (-70.0, 120.0), (-72.0, 170.0),
    ],
];

/// Dirección (marco eclíptico, unitaria) de un punto geográfico. La
/// longitud del observador y el RAMC fijan la fase de rotación de la
/// Tierra: el observador está en AR = RAMC, así que cualquier otra
/// longitud geográfica `lon` está en AR = RAMC + (lon − lon_obs).
fn geo_to_ecliptic(lat: f32, lon: f32, lon_obs: f32, ramc: f32, eps_rad: f32) -> Vec3 {
    let ra = (ramc + lon - lon_obs).to_radians();
    let dec = lat.to_radians();
    let (sra, cra) = ra.sin_cos();
    let (sd, cd) = dec.sin_cos();
    rot_x(Vec3::new(cd * cra, cd * sra, sd), eps_rad)
}

/// La Tierra interior: un globo pequeño en el centro de la esfera
/// celeste, con el **mar** y los **continentes** teñidos distinto, un
/// **sombreado día/noche** según la posición del Sol, y el observador
/// marcado en su lugar real. Orientada de modo que el punto geográfico
/// del observador mira al cénit — y gira con la vista.
#[allow(clippy::too_many_arguments)]
fn add_inner_earth(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    model: &RenderModel,
    eps: f32,
    size: f32,
    center: f32,
    rad: f32,
    pal: &Palette,
) {
    const R_EARTH: f32 = 0.26;
    let ramc = ramc_deg(model.midheaven_deg, eps);
    let lon_obs = model.geo_longitude_deg;
    // Dirección unitaria de un punto geográfico (sin escalar).
    let dir = |lat: f32, lon: f32| geo_to_ecliptic(lat, lon, lon_obs, ramc, eps);
    // El mismo punto, escalado al radio de la Tierra interior.
    let geo = |lat: f32, lon: f32| dir(lat, lon).scale(R_EARTH);

    // El Sol de la carta (si está) — para el día/noche. El lado de la
    // Tierra que mira al Sol es el día: un punto `d` está de día si
    // `d · sol > 0`.
    let sun_dir: Option<Vec3> = model
        .layers
        .iter()
        .filter(|l| matches!(l.kind, LayerKind::Bodies) && l.module_id == "natal")
        .flat_map(|l| l.glyphs.iter())
        .find(|g| g.symbol == "sun")
        .map(|g| eclip(g.deg));
    let es_dia = |d: Vec3| -> bool { sun_dir.map(|s| d.dot(s) > 0.0).unwrap_or(true) };

    // Mar — disco base teñido de azul.
    let sea = if pal.is_dark {
        Rgba::opaque(0.10, 0.21, 0.39)
    } else {
        Rgba::opaque(0.58, 0.72, 0.86)
    };
    items.push((
        -0.95,
        DrawCommand::Circle {
            cx: center,
            cy: center,
            r: R_EARTH * rad,
            stroke: Some(pal.fg_muted.with_alpha(0.30)),
            fill: Some(sea.with_alpha(0.55)),
            stroke_w: 0.8,
        },
    ));

    // Resplandor diurno — el hemisferio iluminado. Discos concéntricos
    // sobre el punto subsolar; se apagan si el Sol queda detrás de la
    // Tierra (entonces vemos su cara nocturna).
    if let Some(s) = sun_dir {
        let sub = proj.project(s.scale(R_EARTH));
        let face = ((sub.depth / R_EARTH) * 0.5 + 0.5).clamp(0.0, 1.0);
        let day = if pal.is_dark {
            Rgba::opaque(0.40, 0.60, 0.85)
        } else {
            Rgba::opaque(1.0, 0.98, 0.88)
        };
        for i in 0..10 {
            let t = i as f32 / 9.0;
            items.push((
                -0.93 + t * 0.04,
                DrawCommand::Circle {
                    cx: sub.x,
                    cy: sub.y,
                    r: R_EARTH * rad * (1.0 - 0.92 * t),
                    stroke: None,
                    fill: Some(day.with_alpha(0.07 * face)),
                    stroke_w: 0.0,
                },
            ));
        }
    }

    // Ecuador terrestre.
    let equator: Vec<Vec3> = (0..72)
        .map(|i| geo(0.0, (i as f32) / 72.0 * 360.0))
        .collect();
    add_loop(items, proj, &equator, pal.fg_muted.with_alpha(0.20), 0.5);

    // Terminador — la línea día/noche, círculo máximo ⊥ al Sol.
    if let Some(s) = sun_dir {
        let term: Vec<Vec3> = great_circle_perp(s, 72)
            .iter()
            .map(|p| p.scale(R_EARTH))
            .collect();
        add_loop(items, proj, &term, pal.angle_highlight.with_alpha(0.45), 0.7);
    }

    // Continentes — polígonos rellenos, teñidos de verde; el tono
    // depende de si la masa está de día o de noche.
    let land_day = if pal.is_dark {
        Rgba::opaque(0.38, 0.60, 0.34)
    } else {
        Rgba::opaque(0.52, 0.66, 0.40)
    };
    let land_night = if pal.is_dark {
        Rgba::opaque(0.13, 0.25, 0.19)
    } else {
        Rgba::opaque(0.40, 0.50, 0.40)
    };
    for outline in CONTINENTES {
        let pts3: Vec<Vec3> = outline.iter().map(|&(lat, lon)| geo(lat, lon)).collect();
        let mut cen = Vec3::new(0.0, 0.0, 0.0);
        let mut depth_sum = 0.0_f32;
        let pts2: Vec<(f32, f32)> = pts3
            .iter()
            .map(|v| {
                cen = Vec3::new(cen.x + v.x, cen.y + v.y, cen.z + v.z);
                let p = proj.project(*v);
                depth_sum += p.depth;
                (p.x, p.y)
            })
            .collect();
        let n = pts3.len().max(1) as f32;
        let depth = depth_sum / n;
        let base = if es_dia(cen.scale(1.0 / n)) {
            land_day
        } else {
            land_night
        };
        items.push((
            depth,
            DrawCommand::Polygon {
                points: pts2,
                fill: Some(dim(base, depth).with_alpha(0.62 * depth_alpha(depth))),
                stroke: Some(dim(base, depth)),
                stroke_w: 0.7,
            },
        ));
    }

    // El observador, en su lugar real sobre la Tierra.
    let obs_dir = dir(model.geo_latitude_deg, lon_obs);
    let p = proj.project(obs_dir.scale(R_EARTH));
    let oc = dim(pal.sun, p.depth);
    items.push((
        p.depth + 0.01,
        DrawCommand::Circle {
            cx: p.x,
            cy: p.y,
            r: size * 0.0075,
            stroke: Some(oc),
            fill: Some(oc.with_alpha(oc.a * if es_dia(obs_dir) { 0.6 } else { 0.15 })),
            stroke_w: 1.2,
        },
    ));
}

// =====================================================================
// Composición
// =====================================================================

/// Compone la esfera celeste como una lista de [`DrawCommand`]s, ya
/// ordenada de atrás hacia adelante (algoritmo del pintor). El canvas
/// nativo y el cliente web la consumen igual que la rueda 2D.
pub fn compose_sphere(
    model: &RenderModel,
    view: &SphereView,
    opts: &SphereOpts,
) -> Vec<DrawCommand> {
    let pal = &opts.palette;
    let size = opts.size;
    let center = size * 0.5;
    let rad = size * 0.36;
    let proj = Projector::new(view, center, center, rad);
    let eps = opts.obliquity_deg.to_radians();
    // El cénit del observador — disponible cuando se pide el horizonte.
    // Lo usan tanto la sección del horizonte como el día/noche de los
    // cuerpos.
    let zenith = if opts.show_horizon {
        Some(zenith_ecliptic(model.geo_latitude_deg, model.midheaven_deg, eps))
    } else {
        None
    };

    // (profundidad, comando) — se ordena al final.
    let mut items: Vec<(f32, DrawCommand)> = Vec::new();

    // --- Cuerpo de la esfera: sombreado con volumen ---
    add_sphere_shading(&mut items, pal, center, rad);

    // --- Cielo de fondo: Vía Láctea + estrellas (solo tema oscuro) ---
    if opts.show_sky && pal.is_dark {
        add_milky_way_glow(&mut items, &proj, eps, size, zenith);
        add_starfield(&mut items, &proj, size, eps);
    }

    // --- Figuras de las constelaciones ---
    if opts.show_constellations {
        add_constellations(&mut items, &proj, eps, size, pal);
    }

    // --- Rejilla: meridianos + paralelos de la eclíptica ---
    if opts.show_grid {
        let grid = pal.fg_muted.with_alpha(0.16);
        for k in 0..6 {
            add_loop(&mut items, &proj, &meridian_points((k as f32) * 30.0, 64), grid, 0.5);
        }
        for &beta in &[-60.0_f32, -30.0, 30.0, 60.0] {
            add_loop(&mut items, &proj, &parallel_points(beta, 64), grid, 0.5);
        }
    }

    // --- Ecuador celeste + eje de la Tierra ---
    if opts.show_equator {
        let equator: Vec<Vec3> = ring_points(96).iter().map(|p| rot_x(*p, eps)).collect();
        add_loop(&mut items, &proj, &equator, pal.uranus.with_alpha(0.85), 1.3);
        let n = proj.project(rot_x(Vec3::new(0.0, 0.0, 1.0), eps));
        let s = proj.project(rot_x(Vec3::new(0.0, 0.0, -1.0), eps));
        items.push((
            (n.depth + s.depth) * 0.5,
            DrawCommand::Line {
                x1: s.x,
                y1: s.y,
                x2: n.x,
                y2: n.y,
                color: pal.uranus.with_alpha(0.45),
                width: 0.8,
                dash: Some((4.0, 4.0)),
            },
        ));
    }

    // --- Eclíptica: el camino del zodíaco, el aro prominente ---
    add_loop(&mut items, &proj, &ring_points(96), pal.dial_ring, 2.0);
    {
        // Eje polar de la eclíptica, tenue.
        let n = proj.project(Vec3::new(0.0, 0.0, 1.0));
        let s = proj.project(Vec3::new(0.0, 0.0, -1.0));
        items.push((
            (n.depth + s.depth) * 0.5,
            DrawCommand::Line {
                x1: s.x,
                y1: s.y,
                x2: n.x,
                y2: n.y,
                color: pal.fg_muted.with_alpha(0.30),
                width: 0.6,
                dash: None,
            },
        ));
    }

    // --- Polos: eclípticos (punto dorado) y celestes (anillo + cruz) ---
    for z in [1.0_f32, -1.0] {
        let p = proj.project(Vec3::new(0.0, 0.0, z));
        items.push((
            p.depth + 0.001,
            DrawCommand::Circle {
                cx: p.x,
                cy: p.y,
                r: size * 0.009,
                stroke: None,
                fill: Some(dim(pal.dial_ring, p.depth)),
                stroke_w: 0.0,
            },
        ));
    }
    for (z, label) in [(1.0_f32, "PN"), (-1.0, "PS")] {
        let pole = rot_x(Vec3::new(0.0, 0.0, z), eps);
        let p = proj.project(pole);
        let col = dim(pal.uranus, p.depth);
        let arm = size * 0.013;
        items.push((
            p.depth + 0.001,
            DrawCommand::Circle {
                cx: p.x,
                cy: p.y,
                r: size * 0.012,
                stroke: Some(col),
                fill: None,
                stroke_w: 1.2,
            },
        ));
        items.push((
            p.depth + 0.001,
            DrawCommand::Line {
                x1: p.x - arm,
                y1: p.y,
                x2: p.x + arm,
                y2: p.y,
                color: col,
                width: 1.0,
                dash: None,
            },
        ));
        items.push((
            p.depth + 0.001,
            DrawCommand::Line {
                x1: p.x,
                y1: p.y - arm,
                x2: p.x,
                y2: p.y + arm,
                color: col,
                width: 1.0,
                dash: None,
            },
        ));
        let lp = proj.project(pole.scale(1.13));
        items.push((
            lp.depth + 0.002,
            DrawCommand::Text {
                x: lp.x,
                y: lp.y,
                content: label.into(),
                color: dim(pal.uranus, lp.depth),
                size: size * 0.018,
                anchor: TextAnchor::Middle,
            },
        ));
    }

    // --- Horizonte local, cénit del observador y meridiano ---
    if let Some(z) = zenith {
        let horiz_color = if pal.is_dark {
            Rgba::opaque(0.90, 0.58, 0.32)
        } else {
            Rgba::opaque(0.66, 0.38, 0.14)
        };
        add_loop(
            &mut items,
            &proj,
            &great_circle_perp(z, 96),
            horiz_color.with_alpha(0.90),
            1.7,
        );
        // El meridiano local: círculo máximo por el cénit y el polo
        // celeste — su normal es `z × NCP`.
        let ncp = rot_x(Vec3::new(0.0, 0.0, 1.0), eps);
        add_loop(
            &mut items,
            &proj,
            &great_circle_perp(z.cross(ncp), 96),
            pal.fg_muted.with_alpha(0.28),
            0.7,
        );
        // Cénit — el punto geográfico del observador — y nadir.
        add_point_marker(&mut items, &proj, z, pal.sun, size, "Cénit", true);
        add_point_marker(
            &mut items,
            &proj,
            z.scale(-1.0),
            pal.fg_muted,
            size,
            "Nadir",
            false,
        );
    }

    // --- Signos: espolón en cada borde + glifo en el centro ---
    if opts.show_signs {
        for i in 0..12 {
            let boundary = (i as f32) * 30.0;
            let a = proj.project(eclip(boundary));
            let b = proj.project(eclip(boundary).scale(1.09));
            let d = (a.depth + b.depth) * 0.5;
            items.push((
                d,
                DrawCommand::Line {
                    x1: a.x,
                    y1: a.y,
                    x2: b.x,
                    y2: b.y,
                    color: dim(pal.dial_ring, d),
                    width: 1.0,
                    dash: None,
                },
            ));
            let mid = boundary + 15.0;
            let g = proj.project(eclip(mid).scale(1.17));
            let name = SIGN_NAMES[i];
            items.push((
                g.depth + 0.002,
                DrawCommand::Text {
                    x: g.x,
                    y: g.y,
                    content: sign_unicode(name).into(),
                    color: dim(pal.sign(name), g.depth),
                    size: size * 0.030,
                    anchor: TextAnchor::Middle,
                },
            ));
        }
    }

    // --- Ángulos ASC / MC / DSC / IC ---
    for (deg, label) in [
        (model.ascendant_deg, "Asc"),
        (model.midheaven_deg, "MC"),
        (model.descendant_deg, "Dsc"),
        (model.imum_coeli_deg, "IC"),
    ] {
        let a = proj.project(eclip(deg));
        let b = proj.project(eclip(deg).scale(1.14));
        let d = (a.depth + b.depth) * 0.5;
        items.push((
            d,
            DrawCommand::Line {
                x1: a.x,
                y1: a.y,
                x2: b.x,
                y2: b.y,
                color: dim(pal.angle_highlight, d),
                width: 1.6,
                dash: None,
            },
        ));
        let lbl = proj.project(eclip(deg).scale(1.30));
        items.push((
            lbl.depth + 0.002,
            DrawCommand::Text {
                x: lbl.x,
                y: lbl.y,
                content: label.into(),
                color: dim(pal.angle_highlight, lbl.depth),
                size: size * 0.021,
                anchor: TextAnchor::Middle,
            },
        ));
    }

    // --- Cuerpos: natales (disco lleno) y topocéntricos (disco hueco
    //     + conector a su par geocéntrico) ---
    if opts.show_bodies {
        let halo = if pal.is_dark {
            pal.bg_panel.with_alpha(0.92)
        } else {
            Rgba::opaque(1.0, 1.0, 1.0).with_alpha(0.92)
        };
        // 1) Cuerpos natales (geocéntricos). Se recuerdan sus posiciones
        //    para poder tender el conector hacia los topocéntricos.
        let mut natal_pos: Vec<(String, Vec3)> = Vec::new();
        for layer in &model.layers {
            if !matches!(layer.kind, LayerKind::Bodies) || layer.module_id != "natal" {
                continue;
            }
            for g in &layer.glyphs {
                let pos = eclip(g.deg);
                natal_pos.push((g.symbol.clone(), pos));
                let p = proj.project(pos);
                let mut color = pal.planet(&g.symbol);
                // Día/noche: un cuerpo bajo el horizonte se atenúa — de
                // un vistazo se ve qué planetas estaban sobre la tierra
                // en el momento de la carta.
                if let Some(z) = zenith {
                    if pos.dot(z) < 0.0 {
                        color = color.with_alpha(color.a * 0.40);
                    }
                }
                items.push((
                    p.depth,
                    DrawCommand::Circle {
                        cx: p.x,
                        cy: p.y,
                        r: size * 0.020,
                        stroke: Some(dim(color, p.depth)),
                        fill: Some(halo),
                        stroke_w: 1.3,
                    },
                ));
                items.push((
                    p.depth + 0.003,
                    DrawCommand::Text {
                        x: p.x,
                        y: p.y,
                        content: planet_unicode_with_retro(&g.symbol, g.retrograde),
                        color: dim(color, p.depth),
                        size: size * 0.026,
                        anchor: TextAnchor::Middle,
                    },
                ));
            }
        }
        // 2) Cuerpos topocéntricos — si la capa está activa. Disco hueco
        //    (sin relleno, lo distingue del natal) + un conector hasta
        //    su par geocéntrico: el LARGO del conector es la paralaje,
        //    así no se miente sobre su magnitud (un cinturón aparte la
        //    exageraría — la diferencia es sub-grado salvo la Luna).
        for layer in &model.layers {
            if !matches!(layer.kind, LayerKind::Bodies) || layer.module_id != "topocentric" {
                continue;
            }
            for g in &layer.glyphs {
                let pos = eclip(g.deg);
                let p = proj.project(pos);
                let color = dim(pal.planet(&g.symbol), p.depth);
                if let Some((_, npos)) = natal_pos.iter().find(|(s, _)| s == &g.symbol) {
                    let np = proj.project(*npos);
                    items.push((
                        p.depth - 0.001,
                        DrawCommand::Line {
                            x1: np.x,
                            y1: np.y,
                            x2: p.x,
                            y2: p.y,
                            color: color.with_alpha(color.a * 0.70),
                            width: 1.0,
                            dash: None,
                        },
                    ));
                }
                items.push((
                    p.depth + 0.002,
                    DrawCommand::Circle {
                        cx: p.x,
                        cy: p.y,
                        r: size * 0.014,
                        stroke: Some(color),
                        fill: None,
                        stroke_w: 1.3,
                    },
                ));
            }
        }
    }

    // --- Estrellas fijas notables (capa del motor, si está activa) ---
    // El motor emite la capa `FixedStars` con la longitud eclíptica ya
    // precesionada; aquí se le suma la latitud para situarla en su
    // lugar real de la esfera, no aplastada sobre la eclíptica.
    for layer in &model.layers {
        if !matches!(layer.kind, LayerKind::FixedStars) {
            continue;
        }
        for g in &layer.glyphs {
            let name = g.annotation.as_deref().unwrap_or("");
            let pos = eclip_latlon(g.deg, fixed_star_latitude(name));
            add_fixed_star(&mut items, &proj, pos, size, name, pal);
        }
    }

    // --- Tierra interior: globo esquemático con el observador ---
    if opts.show_earth {
        add_inner_earth(&mut items, &proj, model, eps, size, center, rad, pal);
    }

    // Algoritmo del pintor: de la profundidad menor (fondo) a la mayor.
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
    items.into_iter().map(|(_, cmd)| cmd).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChartId, ChartKind, Geometry, Glyph, Layer};

    #[test]
    fn vernal_point_y_cuadratura_sobre_la_eclyptica() {
        let v = eclip(0.0);
        assert!((v.x - 1.0).abs() < 1e-5 && v.y.abs() < 1e-5 && v.z.abs() < 1e-5);
        let q = eclip(90.0);
        assert!(q.x.abs() < 1e-5 && (q.y - 1.0).abs() < 1e-5 && q.z.abs() < 1e-5);
    }

    #[test]
    fn la_oblicuidad_inclina_el_polo_celeste() {
        // El polo norte celeste = polo eclíptico rotado por ε. El
        // ángulo entre ambos debe ser exactamente ε.
        let ncp = rot_x(Vec3::new(0.0, 0.0, 1.0), OBLICUIDAD_DEG.to_radians());
        let cos_ang = ncp.z; // producto punto con (0,0,1).
        let ang = cos_ang.acos().to_degrees();
        assert!((ang - OBLICUIDAD_DEG).abs() < 1e-3, "ángulo {ang}");
    }

    #[test]
    fn la_proyeccion_no_se_sale_del_cuadro() {
        let view = SphereView::default();
        let proj = Projector::new(&view, 300.0, 300.0, 108.0);
        for i in 0..360 {
            let p = proj.project(eclip(i as f32));
            assert!(p.x >= 300.0 - 109.0 && p.x <= 300.0 + 109.0);
            assert!(p.y >= 300.0 - 109.0 && p.y <= 300.0 + 109.0);
        }
    }

    fn modelo_demo() -> RenderModel {
        RenderModel {
            chart_id: ChartId::default(),
            chart_kind: ChartKind::Natal,
            title: "demo".into(),
            subtitle: None,
            compute_ms: 0,
            ascendant_deg: 100.0,
            midheaven_deg: 10.0,
            descendant_deg: 280.0,
            imum_coeli_deg: 190.0,
            geo_latitude_deg: -34.6,
            geo_longitude_deg: -58.4,
            layers: vec![Layer {
                module_id: "natal".into(),
                kind: LayerKind::Bodies,
                ring: 0.0,
                z: 0,
                geometry: Geometry::GlyphsOnly,
                glyphs: vec![
                    Glyph { deg: 12.0, symbol: "sun".into(), ..Default::default() },
                    Glyph { deg: 200.0, symbol: "moon".into(), ..Default::default() },
                ],
            }],
            overlays: vec![],
            aspect_summary: vec![],
            uranian_groups: vec![],
            gr_triggers: vec![],
            harmonic: 1,
            harmonic_spectrum: vec![],
        }
    }

    #[test]
    fn compose_sphere_emite_esqueleto_y_cuerpos() {
        // Sin constelaciones, para contar solo el esqueleto base.
        let cmds = compose_sphere(
            &modelo_demo(),
            &SphereView::default(),
            &SphereOpts { show_constellations: false, ..Default::default() },
        );
        assert!(!cmds.is_empty(), "la esfera produce comandos");
        let lineas = cmds.iter().filter(|c| matches!(c, DrawCommand::Line { .. })).count();
        let textos = cmds.iter().filter(|c| matches!(c, DrawCommand::Text { .. })).count();
        assert!(lineas > 100, "círculos máximos como polilíneas: {lineas}");
        // 12 signos + 4 ángulos + 2 polos celestes + cénit + nadir + 2
        // cuerpos = 22 etiquetas de texto.
        assert_eq!(textos, 22, "glifos de signos, ángulos, polos y cuerpos: {textos}");
    }

    #[test]
    fn las_constelaciones_dibujan_sus_figuras() {
        assert!(
            crate::constellations_data::FIGURAS.len() > 80,
            "el catálogo trae las 88 constelaciones"
        );
        let modelo = modelo_demo();
        let lineas = |c: &[DrawCommand]| {
            c.iter().filter(|d| matches!(d, DrawCommand::Line { .. })).count()
        };
        let con = compose_sphere(&modelo, &SphereView::default(), &SphereOpts::default());
        let sin = compose_sphere(
            &modelo,
            &SphereView::default(),
            &SphereOpts { show_constellations: false, ..Default::default() },
        );
        assert!(
            lineas(&con) > lineas(&sin) + 500,
            "las figuras agregan cientos de trazos: {} vs {}",
            lineas(&con),
            lineas(&sin),
        );
    }

    #[test]
    fn el_cenit_esta_a_la_colatitud_del_polo_celeste() {
        let eps = OBLICUIDAD_DEG.to_radians();
        for &(lat, mc) in &[(-34.6_f32, 10.0_f32), (40.0, 200.0), (0.0, 95.0), (60.0, 300.0)] {
            let z = zenith_ecliptic(lat, mc, eps);
            let ncp = rot_x(Vec3::new(0.0, 0.0, 1.0), eps);
            // El ángulo cénit↔polo celeste es la colatitud (90°−φ): su
            // coseno —el producto punto de dos unitarios— es sin φ.
            assert!(
                (z.dot(ncp) - lat.to_radians().sin()).abs() < 1e-4,
                "lat {lat}: z·NCP = {} vs sin φ = {}",
                z.dot(ncp),
                lat.to_radians().sin(),
            );
        }
    }

    #[test]
    fn el_cielo_dibuja_un_campo_de_estrellas() {
        let modelo = modelo_demo();
        let con = compose_sphere(
            &modelo,
            &SphereView::default(),
            &SphereOpts { show_sky: true, ..Default::default() },
        );
        let sin = compose_sphere(
            &modelo,
            &SphereView::default(),
            &SphereOpts { show_sky: false, ..Default::default() },
        );
        let discos = |c: &[DrawCommand]| {
            c.iter().filter(|d| matches!(d, DrawCommand::Circle { .. })).count()
        };
        assert!(
            discos(&con) > discos(&sin) + 300,
            "el cielo agrega cientos de estrellas: {} vs {}",
            discos(&con),
            discos(&sin),
        );
    }

    #[test]
    fn eclip_latlon_respeta_la_latitud() {
        let sobre = eclip_latlon(123.0, 0.0);
        assert!(sobre.z.abs() < 1e-5, "latitud 0 → sobre la eclíptica");
        let polo = eclip_latlon(45.0, 90.0);
        assert!((polo.z - 1.0).abs() < 1e-5, "latitud 90 → polo eclíptico");
        let sirio = eclip_latlon(200.0, -39.61);
        assert!((sirio.z - (-39.61_f32).to_radians().sin()).abs() < 1e-5);
    }

    #[test]
    fn las_latitudes_de_estrellas_fijas_son_coherentes() {
        // Sirio es la más austral; Vega la más boreal; Régulo casi
        // sobre la eclíptica; una desconocida cae a latitud 0.
        assert!(fixed_star_latitude("Sirius") < -30.0);
        assert!(fixed_star_latitude("Vega") > 55.0);
        assert!(fixed_star_latitude("Regulus").abs() < 1.0);
        assert_eq!(fixed_star_latitude("Inexistente"), 0.0);
    }

    #[test]
    fn compose_sphere_dibuja_las_estrellas_fijas_de_la_capa() {
        let mut modelo = modelo_demo();
        modelo.layers.push(Layer {
            module_id: "fixed_stars".into(),
            kind: LayerKind::FixedStars,
            ring: 1.04,
            z: 16,
            geometry: Geometry::GlyphsOnly,
            glyphs: vec![Glyph {
                deg: 104.0,
                symbol: "✦Sir".into(),
                annotation: Some("Sirius".into()),
                ..Default::default()
            }],
        });
        let cmds = compose_sphere(&modelo, &SphereView::default(), &SphereOpts::default());
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                DrawCommand::Text { content, .. } if content == "Sirius"
            )),
            "la estrella fija de la capa aparece etiquetada en la esfera"
        );
    }

    #[test]
    fn el_observador_sobre_la_tierra_coincide_con_el_cenit() {
        let eps = OBLICUIDAD_DEG.to_radians();
        for &(lat, lon, mc) in &[(-34.6_f32, -58.4, 10.0), (40.0, 14.0, 200.0), (51.5, 0.0, 280.0)] {
            let ramc = ramc_deg(mc, eps);
            // El punto geográfico del observador mira exactamente al
            // cénit — eso ancla la orientación de la Tierra interior.
            let obs = geo_to_ecliptic(lat, lon, lon, ramc, eps);
            let zen = zenith_ecliptic(lat, mc, eps);
            assert!(obs.dot(zen) > 0.9999, "obs·cénit = {}", obs.dot(zen));
        }
    }

    #[test]
    fn la_tierra_interior_dibuja_continentes_rellenos() {
        let modelo = modelo_demo();
        let poligonos = |c: &[DrawCommand]| {
            c.iter().filter(|d| matches!(d, DrawCommand::Polygon { .. })).count()
        };
        let con = compose_sphere(&modelo, &SphereView::default(), &SphereOpts::default());
        let sin = compose_sphere(
            &modelo,
            &SphereView::default(),
            &SphereOpts { show_earth: false, ..Default::default() },
        );
        assert_eq!(poligonos(&sin), 0, "sin Tierra no hay continentes");
        assert!(
            poligonos(&con) >= 6,
            "la Tierra interior rellena cada continente como polígono"
        );
    }

    #[test]
    fn el_meridiano_contiene_cenit_polo_y_medio_cielo() {
        let eps = OBLICUIDAD_DEG.to_radians();
        for &(lat, mc) in &[(-34.6_f32, 10.0_f32), (40.0, 200.0), (51.5, 280.0)] {
            let z = zenith_ecliptic(lat, mc, eps);
            let ncp = rot_x(Vec3::new(0.0, 0.0, 1.0), eps);
            // Cénit, polo celeste y MC son coplanares (el plano del
            // meridiano) → su producto mixto se anula. Esto verifica
            // que el RAMC se derivó bien del Medio Cielo.
            let triple = z.cross(ncp).dot(eclip(mc));
            assert!(triple.abs() < 1e-4, "lat {lat}, mc {mc}: triple = {triple}");
        }
    }

    #[test]
    fn el_primer_comando_es_el_limbo_de_fondo() {
        let cmds = compose_sphere(&modelo_demo(), &SphereView::default(), &SphereOpts::default());
        assert!(
            matches!(cmds.first(), Some(DrawCommand::Circle { .. })),
            "el limbo (profundidad −100) se pinta primero"
        );
    }
}
