//! `cosmos-astrocartography` — primitivas del mapa AstroCarto.
//!
//! Día juliano, GMST, conversión eclíptica→ecuatorial y la proyección
//! equirectangular del lienzo. Es astronomía + geometría pura: cero UI, cero
//! Llimphi. Vivía recalculada dentro del tile `cosmos-app-llimphi::astrocarto`
//! (frontend) — y duplicaba JD/GMST/oblicuidad de `cosmos-time`/`cosmos-coords`;
//! baja acá para que cualquier frontend (web/CLI) la reuse y se pueda testear
//! sin pintar (Regla 2). El ensamblado de las líneas MC/IC/Asc/Desc y su trazo
//! siguen en el frontend, construidos con estas piezas.
//!
//! La aproximación supone latitud eclíptica β=0 para todos los cuerpos (válido
//! para AstroCarto a este zoom) y obliquidad ε₂₀₀₀ = 23.4393° fija (error a 100
//! años < 0.01°).

#![forbid(unsafe_code)]

/// Obliquidad media ε₂₀₀₀ en grados.
pub const ASTROCARTO_OBLIQUITY: f64 = 23.4393;
/// Ancho del lienzo equirectangular de referencia (coordenadas de proyección).
pub const ASTROCARTO_W: f32 = 320.0;
/// Alto del lienzo equirectangular de referencia.
pub const ASTROCARTO_H: f32 = 160.0;

/// Día juliano UTC a partir de una fecha/hora civil.
pub fn julian_day_utc(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: f64) -> f64 {
    let (y, m) = if month <= 2 {
        (year - 1, (month + 12) as i32)
    } else {
        (year, month as i32)
    };
    let a = (y as f64 / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    let jd0 = (365.25 * (y as f64 + 4716.0)).floor()
        + (30.6001 * (m as f64 + 1.0)).floor()
        + day as f64
        + b
        - 1524.5;
    let frac = (hour as f64 + minute as f64 / 60.0 + second / 3600.0) / 24.0;
    jd0 + frac
}

/// GMST en grados [0, 360) — Meeus 12.4.
pub fn gmst_deg(jd_ut: f64) -> f64 {
    let t = (jd_ut - 2451545.0) / 36525.0;
    let g = 280.46061837
        + 360.98564736629 * (jd_ut - 2451545.0)
        + 0.000387933 * t * t
        - t * t * t / 38710000.0;
    g.rem_euclid(360.0)
}

/// Conversión eclíptica → ecuatorial con β=0 fijo. Retorna (RA°, Dec°).
pub fn ecliptic_to_equatorial(lon_deg: f64) -> (f64, f64) {
    let l = lon_deg.to_radians();
    let e = ASTROCARTO_OBLIQUITY.to_radians();
    let ra = (l.sin() * e.cos()).atan2(l.cos()).to_degrees().rem_euclid(360.0);
    let dec = (e.sin() * l.sin()).asin().to_degrees();
    (ra, dec)
}

/// Normaliza una longitud a `[-180, 180]`.
pub fn wrap_lon(lon: f64) -> f64 {
    let l = lon.rem_euclid(360.0);
    if l > 180.0 {
        l - 360.0
    } else {
        l
    }
}

/// Proyección equirectangular a coordenadas del lienzo de referencia.
pub fn project_lon_lat(lon_deg: f64, lat_deg: f64) -> (f32, f32) {
    let x = ((lon_deg + 180.0) / 360.0) as f32 * ASTROCARTO_W;
    let y = ((90.0 - lat_deg) / 180.0) as f32 * ASTROCARTO_H;
    (x.clamp(0.0, ASTROCARTO_W), y.clamp(0.0, ASTROCARTO_H))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmst_en_rango() {
        let jd = julian_day_utc(2000, 1, 1, 12, 0, 0.0);
        let g = gmst_deg(jd);
        assert!((0.0..360.0).contains(&g));
    }

    #[test]
    fn ecliptic_equinoccio() {
        // λ=0 (punto vernal) → RA≈0, Dec≈0.
        let (ra, dec) = ecliptic_to_equatorial(0.0);
        assert!(ra.abs() < 1e-6 || (360.0 - ra).abs() < 1e-6);
        assert!(dec.abs() < 1e-6);
    }

    #[test]
    fn proyeccion_centro_y_bordes() {
        // (0,0) cae al centro del lienzo.
        let (x, y) = project_lon_lat(0.0, 0.0);
        assert!((x - ASTROCARTO_W / 2.0).abs() < 1e-3);
        assert!((y - ASTROCARTO_H / 2.0).abs() < 1e-3);
    }

    #[test]
    fn wrap_lon_normaliza() {
        assert_eq!(wrap_lon(190.0), -170.0);
        assert_eq!(wrap_lon(-10.0), -10.0);
    }
}
