//! `cosmos-sundial` — reloj de sol simulado.
//!
//! Capa fina sobre [`cosmos_skywatch`] que toma la posición horizontal
//! del Sol y la convierte en lo único que un gnomon ve: la dirección y
//! el largo de su sombra.
//!
//! Para un gnomon vertical de altura `h` con su base en el origen:
//!
//! - Si el Sol está sobre el horizonte (`alt > 0`), la sombra cae
//!   sobre el plano horizontal hacia el azimut **opuesto** al del
//!   Sol (`shadow_azimuth = sun_azimuth + 180°`).
//! - El largo de la sombra es `h * cot(alt) = h / tan(alt)`. Cuando
//!   `alt → 0` (Sol al horizonte) la sombra tiende a infinito; el API
//!   reporta `None` en ese caso así el caller decide qué pintar.
//!
//! La altura del gnomon no entra al cálculo del azimut ni del ángulo;
//! sólo escala la sombra. Por eso el API expone primero un cociente
//! `shadow_length_ratio = largo/h` y luego un atajo `shadow_length_for`
//! que multiplica por una altura concreta. Apto para cuadrantes
//! físicos (gnomon real) o didácticos (escala visual en pantalla).

#![forbid(unsafe_code)]

use cosmos_core::Location;
use cosmos_skywatch::{sky_position, Body, SkyPosition};
use cosmos_time::TDB;

/// Lectura de un cuadrante solar a un instante dado.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SundialReading {
    /// Posición horizontal del Sol — útil si el caller quiere también
    /// pintar la trayectoria solar o el HA real.
    pub sun: SkyPosition,
    /// Azimut hacia donde cae la sombra (0° = N, 90° = E, 180° = S,
    /// 270° = W). `None` si el Sol está bajo el horizonte.
    pub shadow_azimuth_deg: Option<f64>,
    /// Largo de la sombra expresado como múltiplo de la altura del
    /// gnomon: `largo = h * ratio`. `None` si el Sol está bajo el
    /// horizonte o demasiado cerca de él (alt < `MIN_ALTITUDE_DEG`).
    pub shadow_length_ratio: Option<f64>,
    /// Ángulo horario del Sol en grados (`LST - RA_sun`), normalizado
    /// a `[-180, 180]`. `0°` = Sol sobre el meridiano (mediodía solar
    /// verdadero). Negativo antes del mediodía, positivo después.
    pub hour_angle_deg: f64,
}

impl SundialReading {
    /// Largo absoluto de la sombra para un gnomon de altura `gnomon_h`.
    /// Mismas unidades que `gnomon_h` (metros, pulgadas, lo que sea).
    /// `None` si el Sol no proyecta sombra utilizable.
    pub fn shadow_length_for(&self, gnomon_h: f64) -> Option<f64> {
        self.shadow_length_ratio.map(|r| r * gnomon_h)
    }
}

/// Por debajo de esta altitud solar la sombra es absurdamente larga
/// (Sol casi al horizonte) y se considera no medible. El cuadrante
/// físico real tampoco resuelve nada útil bajo este umbral —
/// refracción atmosférica + horizonte topográfico dominan.
pub const MIN_ALTITUDE_DEG: f64 = 1.0;

/// Calcula una lectura del cuadrante solar para una ubicación y un
/// instante TDB dados.
pub fn sundial_reading(tdb: &TDB, location: &Location) -> SundialReading {
    let sun = sky_position(&Body::Sun, tdb, location);

    // El HA solar es el complemento de la AR — lo recalculamos aquí
    // a partir del LST/RA en lugar de re-exportar el cálculo interno
    // de skywatch: mantiene cosmos-sundial autónomo si en el futuro
    // skywatch refactoriza su API.
    let jd = tdb.to_julian_date().to_f64();
    let gmst = gmst_from_jd(jd);
    let lst = wrap_360(gmst + location.longitude_degrees());
    let ha = wrap_180(lst - sun.right_ascension_deg);

    let (shadow_az, shadow_ratio) = if sun.altitude_deg > MIN_ALTITUDE_DEG {
        let az = (sun.azimuth_deg + 180.0) % 360.0;
        let alt_rad = sun.altitude_deg.to_radians();
        let ratio = 1.0 / alt_rad.tan();
        (Some(az), Some(ratio))
    } else {
        (None, None)
    };

    SundialReading {
        sun,
        shadow_azimuth_deg: shadow_az,
        shadow_length_ratio: shadow_ratio,
        hour_angle_deg: ha,
    }
}

fn gmst_from_jd(jd_ut1: f64) -> f64 {
    let t = (jd_ut1 - 2451545.0) / 36525.0;
    let secs = 67310.54841
        + (876600.0 * 3600.0 + 8640184.812866) * t
        + 0.093104 * t * t
        - 6.2e-6 * t * t * t;
    let hours = (secs / 3600.0).rem_euclid(24.0);
    hours * 15.0
}

fn wrap_360(deg: f64) -> f64 {
    let m = deg.rem_euclid(360.0);
    if m < 0.0 {
        m + 360.0
    } else {
        m
    }
}

fn wrap_180(deg: f64) -> f64 {
    let m = (deg + 180.0).rem_euclid(360.0) - 180.0;
    if m == -180.0 {
        180.0
    } else {
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lima() -> Location {
        Location::from_degrees(-12.05, -77.05, 150.0).unwrap()
    }

    fn quito() -> Location {
        // Lat ≈ 0° — Sol al cenit en equinoccios, sombra → 0 a mediodía.
        Location::from_degrees(0.0, -78.5, 2850.0).unwrap()
    }

    #[test]
    fn sun_below_horizon_no_shadow() {
        // 23:00 TDB en Lima → Sol ya bajo el horizonte (sale ~17:30
        // local en mayo).
        let r = sundial_reading(&"2026-05-27T23:00:00".parse().unwrap(), &lima());
        assert!(!r.sun.above_horizon, "Sol bajo horizonte");
        assert!(r.shadow_azimuth_deg.is_none());
        assert!(r.shadow_length_ratio.is_none());
        assert!(r.shadow_length_for(1.0).is_none());
    }

    #[test]
    fn noon_shadow_falls_opposite_to_sun() {
        // Sun cerca del cenit a mediodía solar en Lima → sombra muy
        // corta, hacia el sur (S = az 180°) o norte según estación.
        let r = sundial_reading(&"2026-05-27T17:00:00".parse().unwrap(), &lima());
        if r.sun.above_horizon {
            let sun_az = r.sun.azimuth_deg;
            let shadow_az = r.shadow_azimuth_deg.unwrap();
            let diff = (shadow_az - sun_az + 360.0).rem_euclid(360.0);
            // shadow_az = sun_az + 180° (módulo 360).
            assert!(
                (diff - 180.0).abs() < 0.01,
                "sombra opuesta al Sol: diff={diff}"
            );
        }
    }

    #[test]
    fn shadow_shorter_when_sun_higher() {
        // Comparamos sombra del Sol a las 17:00 (alto) vs 21:00 (bajo)
        // TDB en Lima. Mediodía Lima ≈ 17:00 TDB.
        let alta = sundial_reading(&"2026-05-27T17:00:00".parse().unwrap(), &lima());
        let baja = sundial_reading(&"2026-05-27T21:00:00".parse().unwrap(), &lima());
        if let (Some(r_alta), Some(r_baja)) =
            (alta.shadow_length_ratio, baja.shadow_length_ratio)
        {
            assert!(
                r_alta < r_baja,
                "sombra más corta cuando Sol más alto: alta={r_alta} baja={r_baja}"
            );
        }
    }

    #[test]
    fn shadow_length_scales_linearly_with_gnomon() {
        let r = sundial_reading(&"2026-05-27T17:00:00".parse().unwrap(), &lima());
        if r.shadow_length_ratio.is_some() {
            let l1 = r.shadow_length_for(1.0).unwrap();
            let l2 = r.shadow_length_for(2.0).unwrap();
            let l5 = r.shadow_length_for(5.0).unwrap();
            assert!((l2 - 2.0 * l1).abs() < 1e-9);
            assert!((l5 - 5.0 * l1).abs() < 1e-9);
        }
    }

    #[test]
    fn hour_angle_zero_near_solar_noon() {
        // Quito al mediodía equinoccial (aprox 2026-03-20T17:13Z para
        // tener Sol cerca del meridiano de Quito): HA debe ser cerca
        // de 0.
        let r = sundial_reading(
            &"2026-03-20T17:13:00".parse().unwrap(),
            &quito(),
        );
        // Aceptamos 0 ± 5° por aprox UT1≈TDB + ecuación del tiempo.
        assert!(
            r.hour_angle_deg.abs() < 5.0,
            "HA solar cerca de 0 al mediodía: {}",
            r.hour_angle_deg
        );
    }

    #[test]
    fn quito_equinox_noon_short_shadow() {
        // Quito (lat ≈ 0) en equinoccio al mediodía solar: Sol al
        // cenit, sombra → 0 (en la práctica < 0.1 de la altura).
        let r = sundial_reading(
            &"2026-03-20T17:13:00".parse().unwrap(),
            &quito(),
        );
        if r.sun.above_horizon {
            assert!(r.sun.altitude_deg > 80.0, "Sol cerca del cenit");
            if let Some(ratio) = r.shadow_length_ratio {
                assert!(ratio < 0.2, "sombra muy corta cerca del cenit: {ratio}");
            }
        }
    }

    #[test]
    fn shadow_azimuth_in_range() {
        // Cualquier sombra debe estar en [0, 360).
        for hour in (10..22u32).step_by(2) {
            let iso = format!("2026-05-27T{:02}:00:00", hour);
            let r = sundial_reading(&iso.parse().unwrap(), &lima());
            if let Some(az) = r.shadow_azimuth_deg {
                assert!(az >= 0.0 && az < 360.0, "sombra az inválida: {az}");
            }
        }
    }

    #[test]
    fn near_horizon_no_shadow() {
        // Forzamos un caso donde alt es muy bajo: justo antes del ocaso
        // en Lima (~22:30 TDB). Si alt < MIN_ALTITUDE_DEG la sombra
        // debe ser None.
        let r = sundial_reading(&"2026-05-27T22:30:00".parse().unwrap(), &lima());
        if r.sun.altitude_deg <= MIN_ALTITUDE_DEG {
            assert!(r.shadow_length_ratio.is_none());
            assert!(r.shadow_azimuth_deg.is_none());
        }
    }
}
