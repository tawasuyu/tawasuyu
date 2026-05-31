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
pub(crate) struct Vec3 {
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
pub(crate) struct Projected {
    x: f32,
    y: f32,
    /// `+` hacia el observador (frente), `−` lejos (fondo).
    depth: f32,
}

/// Proyector ortográfico: gira un punto por la cámara (`yaw` alrededor
/// del eje polar, `pitch` alrededor del eje horizontal de pantalla) y
/// lo aplana a coordenadas de pantalla.
pub(crate) struct Projector {
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

// =====================================================================
// Submódulos: capas de la escena (alambre, estrellas, Tierra) y el
// compositor que las orquesta. Tipos y math primitiva viven arriba.
// =====================================================================
mod compose;
mod earth;
mod layers;
mod starfield;
#[cfg(test)]
mod tests;

pub use compose::*;
pub(crate) use earth::*;
pub(crate) use layers::*;
pub(crate) use starfield::*;
