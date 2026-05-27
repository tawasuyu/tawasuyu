//! `cosmos-tides` — modelo de mareas instantáneas Sol+Luna.
//!
//! Calcula el **potencial generador de marea** (la pieza astronómica)
//! sobre una ubicación de la Tierra a un instante TDB. No resuelve la
//! respuesta hidrodinámica de la cuenca local (esa requiere
//! modelado FEM tipo OSU/FES, fuera del alcance de gioser-suite).
//! Sirve para curvas indicativas, comparación cualitativa
//! entre días/lugares, animaciones educativas, y como input al motor
//! ECS de dominium si alguien quiere simular fluidos a alto nivel.
//!
//! ## Modelo
//!
//! Para cada cuerpo perturbador, el potencial de marea evaluado en la
//! superficie terrestre tiene como primer término no trivial el del
//! segundo polinomio de Legendre:
//!
//! ```text
//!   V_2(z) = (G * M / d) * (R/d)^2 * (3 cos²(z) − 1) / 2
//! ```
//!
//! donde `M` es la masa del cuerpo, `d` la distancia geocéntrica al
//! cuerpo, `R` el radio terrestre y `z` el ángulo cenital del cuerpo
//! desde la ubicación (`cos(z) = sin(alt)`). La fuerza vertical local
//! (que sube/baja la altura del agua) es proporcional a `(3cos²z−1)/2`.
//!
//! La altura equilibrada (Equilibrium Tide) se obtiene dividiendo
//! por la gravedad superficial `g`. En unidades de metros se
//! sustituyen los `GM` de cuerpos y `R, g` constantes.
//!
//! ## Salida
//!
//! [`TideReading`] expone:
//! - `lunar_height_m` y `solar_height_m`: altura de equilibrio por
//!   cuerpo, en metros, signo conservado (positivo = "abulta",
//!   negativo = "se hunde");
//! - `total_height_m`: suma de los dos;
//! - `lunar_zenith_deg`, `solar_zenith_deg`: ángulo cenital de cada
//!   cuerpo, útil para diagnóstico.
//!
//! Magnitudes esperadas en el ecuador: pico lunar ≈ 0.36 m, solar
//! ≈ 0.16 m. Marea viva (Sol y Luna alineados): ≈ 0.52 m. Marea
//! muerta (cuadratura): ≈ 0.20 m. La marea **real** observada en
//! costa es 1–10× mayor por la amplificación de resonancia.

#![forbid(unsafe_code)]

use cosmos_core::Location;
use cosmos_skywatch::{sky_position, Body, SkyPosition};
use cosmos_time::TDB;

/// Radio terrestre medio en metros (IAU 2015 / IERS).
pub const EARTH_RADIUS_M: f64 = 6_378_137.0;
/// Gravedad estándar en m/s².
pub const STANDARD_GRAVITY: f64 = 9.80665;
/// 1 AU en metros — para convertir distancias de cosmos-ephemeris.
pub const AU_M: f64 = 149_597_870_700.0;

/// `GM` (m³/s²) de los dos cuerpos relevantes para la marea
/// equilibrada terrestre. Sol y Luna. Otros cuerpos contribuyen <0.01 m
/// — los omitimos del MVP.
pub mod gm {
    /// Heliocentric gravitational parameter (m³/s²).
    pub const SUN: f64 = 1.32712440018e20;
    /// Lunar gravitational parameter (m³/s²) — Lunar Laser Ranging.
    pub const MOON: f64 = 4.9028000661e12;
}

/// Lectura de marea para una ubicación en un instante.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TideReading {
    pub lunar: ComponentReading,
    pub solar: ComponentReading,
    /// Altura total = lunar + solar, en metros.
    pub total_height_m: f64,
}

/// Aporte de un cuerpo a la marea instantánea.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComponentReading {
    /// Altura de equilibrio (metros). Positivo = abulta hacia el
    /// cenit local; negativo = hacia el horizonte (banda lateral
    /// del bulge).
    pub height_m: f64,
    /// Ángulo cenital del cuerpo en grados (0° = cenit, 90° =
    /// horizonte). Negativo significa que el cuerpo está más alto que
    /// el cenit, lo cual no ocurre — el rango es `[0, 180]`.
    pub zenith_deg: f64,
    /// Posición topocéntrica completa del cuerpo — útil para que un
    /// caller que ya pide skywatch no recalcule.
    pub sky: SkyPosition,
}

/// Calcula la lectura de marea para una ubicación a un instante TDB.
pub fn tide_reading(tdb: &TDB, location: &Location) -> TideReading {
    let lunar = component(Body::Moon, gm::MOON, tdb, location);
    let solar = component(Body::Sun, gm::SUN, tdb, location);
    TideReading {
        total_height_m: lunar.height_m + solar.height_m,
        lunar,
        solar,
    }
}

fn component(
    body: Body,
    gm: f64,
    tdb: &TDB,
    location: &Location,
) -> ComponentReading {
    let sky = sky_position(&body, tdb, location);
    // alt en grados → cenital en grados. Sol bajo horizonte sigue
    // contribuyendo (la marea no se apaga de noche; el bulge tiene
    // simetría con el lado opuesto, por eso `3cos²z − 1` toma valores
    // negativos para z cerca de 90°).
    let z_deg = 90.0 - sky.altitude_deg;
    let z = z_deg.to_radians();
    let cos_z = z.cos();
    let factor = (3.0 * cos_z * cos_z - 1.0) * 0.5;
    let d_m = sky.distance_au * AU_M;
    // V_2 / g = (GM/d) * (R/d)^2 * factor / g
    // → metros equivalentes de marea de equilibrio.
    let height = (gm / d_m) * (EARTH_RADIUS_M / d_m).powi(2) * factor / STANDARD_GRAVITY;
    ComponentReading {
        height_m: height,
        zenith_deg: z_deg,
        sky,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn callao() -> Location {
        // Callao (puerto de Lima): lat ≈ -12.07°, lon ≈ -77.13°.
        Location::from_degrees(-12.07, -77.13, 0.0).unwrap()
    }

    #[test]
    fn equator_lunar_peak_in_expected_range() {
        // El pico de marea lunar de equilibrio es ≈ 0.36 m en el
        // ecuador con la Luna al cenit. Buscamos un instante donde la
        // Luna esté alta sobre el Callao y medimos su componente
        // lunar — debe ser positiva y < ~0.6 m.
        let loc = callao();
        // Barremos 24h a paso de 1h y nos quedamos con el pico.
        let mut max_lunar = f64::MIN;
        let mut max_z = 0.0_f64;
        for h in 0..24u32 {
            let iso = format!("2026-06-15T{:02}:00:00", h);
            let r = tide_reading(&iso.parse().unwrap(), &loc);
            if r.lunar.height_m > max_lunar {
                max_lunar = r.lunar.height_m;
                max_z = r.lunar.zenith_deg;
            }
        }
        assert!(
            max_lunar > 0.05 && max_lunar < 0.6,
            "pico lunar plausible ~ 0.05..0.6 m, fue {max_lunar} (zenith en pico = {max_z})"
        );
    }

    #[test]
    fn total_equals_lunar_plus_solar() {
        let r = tide_reading(
            &"2026-06-15T12:00:00".parse().unwrap(),
            &callao(),
        );
        assert!((r.total_height_m - (r.lunar.height_m + r.solar.height_m)).abs() < 1e-9);
    }

    #[test]
    fn solar_smaller_than_lunar_at_peak() {
        // La regla cualitativa: lunar > solar (≈ 2.2×). Comparamos el
        // pico de cada componente.
        let loc = callao();
        let mut max_l = f64::MIN;
        let mut max_s = f64::MIN;
        for h in 0..24u32 {
            let iso = format!("2026-06-15T{:02}:00:00", h);
            let r = tide_reading(&iso.parse().unwrap(), &loc);
            if r.lunar.height_m > max_l {
                max_l = r.lunar.height_m;
            }
            if r.solar.height_m > max_s {
                max_s = r.solar.height_m;
            }
        }
        assert!(max_l > max_s, "lunar > solar: lunar={max_l} solar={max_s}");
    }

    #[test]
    fn syzygy_higher_than_quadrature() {
        // La amplitud diaria de marea total debe ser mayor en sicigia
        // (Luna nueva/llena, Sol y Luna alineados) que en cuadratura
        // (cuarto creciente/menguante). Aproximamos sicigia tomando
        // un día donde la Luna está cerca del Sol — la fase exacta
        // requeriría un cálculo extra.
        let loc = callao();
        let amplitude_at = |day: &str| -> f64 {
            let mut mx = f64::MIN;
            let mut mn = f64::MAX;
            for h in 0..24u32 {
                let iso = format!("{day}T{:02}:00:00", h);
                let r = tide_reading(&iso.parse().unwrap(), &loc);
                if r.total_height_m > mx {
                    mx = r.total_height_m;
                }
                if r.total_height_m < mn {
                    mn = r.total_height_m;
                }
            }
            mx - mn
        };
        // Luna nueva aprox: 2026-01-18; cuarto: 2026-01-26.
        // (Las fechas exactas las verifica cosmos-skywatch.)
        let amp_syz = amplitude_at("2026-01-18");
        let amp_quad = amplitude_at("2026-01-26");
        assert!(
            amp_syz > amp_quad,
            "amplitud sicigia > cuadratura: syz={amp_syz} quad={amp_quad}"
        );
    }

    #[test]
    fn zenith_in_valid_range() {
        let r = tide_reading(
            &"2026-06-15T12:00:00".parse().unwrap(),
            &callao(),
        );
        assert!(
            r.lunar.zenith_deg >= 0.0 && r.lunar.zenith_deg <= 180.0,
            "z_lunar fuera de rango: {}",
            r.lunar.zenith_deg
        );
        assert!(
            r.solar.zenith_deg >= 0.0 && r.solar.zenith_deg <= 180.0,
            "z_solar fuera de rango: {}",
            r.solar.zenith_deg
        );
    }

    #[test]
    fn body_below_horizon_still_contributes() {
        // Cuando un cuerpo está al horizonte (z = 90°), su factor de
        // marea es (3·0 − 1)/2 = -0.5 → altura negativa (banda
        // lateral del bulge). Tomamos un caso donde la Luna acaba de
        // ponerse y verificamos que su componente lunar es negativa.
        let loc = callao();
        // Barremos 24h y buscamos el caso de Luna cerca del horizonte.
        let mut closest_to_90 = (180.0_f64, 0.0_f64);
        for h in 0..24u32 {
            let iso = format!("2026-06-15T{:02}:00:00", h);
            let r = tide_reading(&iso.parse().unwrap(), &loc);
            let dist_to_90 = (r.lunar.zenith_deg - 90.0).abs();
            if dist_to_90 < (closest_to_90.0 - 90.0_f64).abs() {
                closest_to_90 = (r.lunar.zenith_deg, r.lunar.height_m);
            }
        }
        // Cerca del horizonte, factor negativo → height < 0 (en al
        // menos algún punto del barrido).
        let (z, h) = closest_to_90;
        if (z - 90.0).abs() < 10.0 {
            assert!(h < 0.0, "Luna cerca del horizonte da height negativa: z={z}, h={h}");
        }
    }
}
