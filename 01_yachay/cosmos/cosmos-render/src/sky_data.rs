//! Datos celestes reales para las vistas de cielo: el catálogo de
//! estrellas brillantes y el plano galáctico (la Vía Láctea).
//!
//! Las **magnitudes** son reales (subconjunto de `sefstars.txt`, las 25
//! más brillantes con V ≤ 1.65, más Castor y Mimosa), así la intensidad
//! de cada estrella en pantalla refleja su brillo verdadero y no un
//! valor decorativo. La **Vía Láctea** se sitúa con el polo galáctico
//! estándar IAU (J2000), igual que en la esfera 3D.

/// Una estrella brillante del catálogo: nombre, posición ecuatorial
/// J2000 y magnitud visual aparente.
#[derive(Debug, Clone, Copy)]
pub struct BrightStar {
    pub name: &'static str,
    pub ra_deg: f32,
    pub dec_deg: f32,
    /// Magnitud visual aparente (más chica = más brillante).
    pub mag: f32,
}

/// Las estrellas más brillantes del cielo (V ≤ 1.65), ordenadas por
/// magnitud ascendente. Valores de `sefstars.txt` (Hipparcos/Swiss).
pub const BRIGHT_STARS: &[BrightStar] = &[
    BrightStar { name: "Sirius", ra_deg: 101.287, dec_deg: -16.716, mag: -1.46 },
    BrightStar { name: "Canopus", ra_deg: 95.988, dec_deg: -52.696, mag: -0.74 },
    BrightStar { name: "Rigil Kent.", ra_deg: 219.901, dec_deg: -60.836, mag: -0.10 },
    BrightStar { name: "Arcturus", ra_deg: 213.915, dec_deg: 19.182, mag: -0.05 },
    BrightStar { name: "Vega", ra_deg: 279.235, dec_deg: 38.784, mag: 0.03 },
    BrightStar { name: "Capella", ra_deg: 79.172, dec_deg: 45.998, mag: 0.08 },
    BrightStar { name: "Rigel", ra_deg: 78.634, dec_deg: -8.202, mag: 0.13 },
    BrightStar { name: "Procyon", ra_deg: 114.825, dec_deg: 5.225, mag: 0.37 },
    BrightStar { name: "Betelgeuse", ra_deg: 88.793, dec_deg: 7.407, mag: 0.42 },
    BrightStar { name: "Achernar", ra_deg: 24.429, dec_deg: -57.237, mag: 0.46 },
    BrightStar { name: "Hadar", ra_deg: 210.956, dec_deg: -60.373, mag: 0.60 },
    BrightStar { name: "Altair", ra_deg: 297.696, dec_deg: 8.868, mag: 0.76 },
    BrightStar { name: "Acrux", ra_deg: 186.650, dec_deg: -63.099, mag: 0.81 },
    BrightStar { name: "Aldebaran", ra_deg: 68.980, dec_deg: 16.509, mag: 0.86 },
    BrightStar { name: "Antares", ra_deg: 247.352, dec_deg: -26.432, mag: 0.91 },
    BrightStar { name: "Spica", ra_deg: 201.298, dec_deg: -11.161, mag: 0.97 },
    BrightStar { name: "Pollux", ra_deg: 116.329, dec_deg: 28.026, mag: 1.14 },
    BrightStar { name: "Fomalhaut", ra_deg: 344.413, dec_deg: -29.622, mag: 1.16 },
    BrightStar { name: "Deneb", ra_deg: 310.358, dec_deg: 45.280, mag: 1.25 },
    BrightStar { name: "Mimosa", ra_deg: 191.930, dec_deg: -59.689, mag: 1.25 },
    BrightStar { name: "Regulus", ra_deg: 152.093, dec_deg: 11.967, mag: 1.40 },
    BrightStar { name: "Adhara", ra_deg: 104.656, dec_deg: -28.972, mag: 1.50 },
    BrightStar { name: "Castor", ra_deg: 113.649, dec_deg: 31.888, mag: 1.58 },
    BrightStar { name: "Shaula", ra_deg: 263.402, dec_deg: -37.104, mag: 1.62 },
    BrightStar { name: "Bellatrix", ra_deg: 81.283, dec_deg: 6.350, mag: 1.64 },
    BrightStar { name: "Elnath", ra_deg: 81.573, dec_deg: 28.607, mag: 1.65 },
];

/// Polo norte galáctico (J2000), constante IAU que fija el plano de la
/// Vía Láctea.
pub const GAL_POLE_RA: f32 = 192.859;
pub const GAL_POLE_DEC: f32 = 27.128;
/// Centro galáctico (Sgr A*, J2000): hacia ahí la Vía Láctea brilla más.
pub const GAL_CENTER_RA: f32 = 266.405;
pub const GAL_CENTER_DEC: f32 = -28.936;

/// Vector unitario de una dirección ecuatorial (AR, Dec en grados).
fn dir(ra_deg: f64, dec_deg: f64) -> [f64; 3] {
    let (sr, cr) = ra_deg.to_radians().sin_cos();
    let (sd, cd) = dec_deg.to_radians().sin_cos();
    [cd * cr, cd * sr, sd]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f64; 3]) -> [f64; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if l < 1e-12 {
        a
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}

/// Una muestra del ecuador galáctico (la línea media de la Vía Láctea):
/// su posición ecuatorial (AR°, Dec°) y `toward_center` ∈ [0,1], cuánto
/// apunta hacia el centro galáctico — para modular el brillo de la banda.
#[derive(Debug, Clone, Copy)]
pub struct GalSample {
    pub ra_deg: f32,
    pub dec_deg: f32,
    pub toward_center: f32,
}

/// `n` muestras a lo largo del ecuador galáctico, en coordenadas
/// ecuatoriales. El círculo máximo perpendicular al polo galáctico.
pub fn galactic_equator(n: usize) -> Vec<GalSample> {
    let pole = norm(dir(GAL_POLE_RA as f64, GAL_POLE_DEC as f64));
    let center = norm(dir(GAL_CENTER_RA as f64, GAL_CENTER_DEC as f64));
    // Base ortonormal del plano perpendicular al polo.
    let r = if pole[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let u = norm(cross(pole, r));
    let v = cross(pole, u);
    (0..n)
        .map(|i| {
            let t = (i as f64) / (n as f64) * std::f64::consts::TAU;
            let (s, c) = t.sin_cos();
            let p = [
                u[0] * c + v[0] * s,
                u[1] * c + v[1] * s,
                u[2] * c + v[2] * s,
            ];
            let ra = p[1].atan2(p[0]).to_degrees().rem_euclid(360.0);
            let dec = p[2].clamp(-1.0, 1.0).asin().to_degrees();
            let dotc = p[0] * center[0] + p[1] * center[1] + p[2] * center[2];
            GalSample {
                ra_deg: ra as f32,
                dec_deg: dec as f32,
                toward_center: ((dotc * 0.5 + 0.5).clamp(0.0, 1.0)) as f32,
            }
        })
        .collect()
}
