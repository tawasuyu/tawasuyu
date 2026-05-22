//! `sphere3d` — la esfera celeste en 3D, proyectada a primitivas 2D.
//!
//! GPUI no es un motor 3D y empotrar wgpu sería frágil. La estrategia
//! es otra: la esfera celeste es un objeto de **alambre** —círculos
//! máximos y puntos—, y eso se proyecta a software con trigonometría
//! pura. Cada superficie (canvas gpui nativo, SVG del cliente web) ya
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
                fill: Some(glow.with_alpha(0.04)),
                stroke_w: 0.0,
            },
        ));
    }
    // Brillo especular desplazado hacia la luz.
    let hx = center - rad * 0.34;
    let hy = center - rad * 0.34;
    const HALO: usize = 7;
    for i in 0..HALO {
        let t = i as f32 / (HALO - 1) as f32;
        items.push((
            -95.0 + t * 0.5,
            DrawCommand::Circle {
                cx: hx,
                cy: hy,
                r: rad * 0.5 * (1.0 - t),
                stroke: None,
                fill: Some(highlight.with_alpha(0.05)),
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

/// El cénit del observador en el marco eclíptico — el punto del cielo
/// justo sobre su cabeza. Se deriva de la latitud geográfica `φ` y de
/// la ascensión recta del Medio Cielo (RAMC): el cénit tiene
/// declinación `φ` y AR `RAMC`, y eso se lleva del marco ecuatorial al
/// eclíptico rotando por la oblicuidad.
fn zenith_ecliptic(lat_deg: f32, mc_deg: f32, eps_rad: f32) -> Vec3 {
    let phi = lat_deg.to_radians();
    let lmc = mc_deg.to_radians();
    // RAMC: AR del punto eclíptico del MC (latitud eclíptica 0).
    let ramc = (lmc.sin() * eps_rad.cos()).atan2(lmc.cos());
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

    // --- Cielo de fondo: estrellas + Vía Láctea (solo tema oscuro) ---
    if opts.show_sky && pal.is_dark {
        add_starfield(&mut items, &proj, size, eps);
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

    // --- Cuerpos natales sobre la eclíptica ---
    if opts.show_bodies {
        for layer in &model.layers {
            if !matches!(layer.kind, LayerKind::Bodies) || layer.module_id != "natal" {
                continue;
            }
            let halo = if pal.is_dark {
                pal.bg_panel.with_alpha(0.92)
            } else {
                Rgba::opaque(1.0, 1.0, 1.0).with_alpha(0.92)
            };
            for g in &layer.glyphs {
                let pos = eclip(g.deg);
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
        let cmds = compose_sphere(&modelo_demo(), &SphereView::default(), &SphereOpts::default());
        assert!(!cmds.is_empty(), "la esfera produce comandos");
        let lineas = cmds.iter().filter(|c| matches!(c, DrawCommand::Line { .. })).count();
        let textos = cmds.iter().filter(|c| matches!(c, DrawCommand::Text { .. })).count();
        assert!(lineas > 100, "círculos máximos como polilíneas: {lineas}");
        // 12 signos + 4 ángulos + 2 polos celestes + cénit + nadir + 2
        // cuerpos = 22 etiquetas de texto.
        assert_eq!(textos, 22, "glifos de signos, ángulos, polos y cuerpos: {textos}");
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
