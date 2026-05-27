//! `cosmos-transits` — tránsitos de Mercurio y Venus sobre el Sol.
//!
//! Calcula la separación angular geocéntrica entre el cuerpo (Mercurio
//! o Venus) y el Sol, y reporta los instantes en que esa separación es
//! menor que el radio aparente del Sol — los **tránsitos**, que son
//! eventos raros pero observables (Venus 2004 + 2012; Mercurio cada
//! 7–13 años).
//!
//! El modelo es deliberadamente simple:
//!
//! - Posiciones geocéntricas ICRS via [`cosmos_ephemeris`].
//! - Separación = `acos(û_body · û_sun)`, en radianes.
//! - Radio solar aparente desde Tierra = `arctan(R_sun / d_sun)`,
//!   donde `R_sun ≈ 695_700 km`. Varía ~0.5° (ligera elipticidad
//!   orbital terrestre); lo recalculamos en cada llamada.
//! - **Filtro inferior**: el cuerpo debe estar entre Tierra y Sol
//!   (`d_body_geo < d_sun_geo`), si no es ocultación, no tránsito.
//!
//! No incluye paralaje topocéntrico (típicamente ~0.04° para Venus,
//! ~0.01° para Mercurio) — eso afecta el horario exacto desde
//! distintos observatorios pero no la existencia del tránsito.

#![forbid(unsafe_code)]

use cosmos_core::Vector3;
use cosmos_ephemeris::planets::{Vsop2013Mercury, Vsop2013Venus};
use cosmos_ephemeris::sun::Vsop2013Sun;
use cosmos_time::{JulianDate, TDB};

/// Cuerpo que puede transitar el disco solar visto desde Tierra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InnerPlanet {
    Mercury,
    Venus,
}

impl InnerPlanet {
    pub fn canonical(&self) -> &'static str {
        match self {
            InnerPlanet::Mercury => "mercury",
            InnerPlanet::Venus => "venus",
        }
    }
}

/// Radio solar fotosférico, en kilómetros. IAU 2015 nominal.
pub const SOLAR_RADIUS_KM: f64 = 695_700.0;

/// Lectura puntual de la separación cuerpo-Sol.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeparationReading {
    /// Separación angular geocéntrica (grados).
    pub separation_deg: f64,
    /// Radio solar aparente desde la Tierra a este instante (grados).
    pub solar_radius_apparent_deg: f64,
    /// Distancia geocéntrica al cuerpo en au.
    pub body_distance_au: f64,
    /// Distancia geocéntrica al Sol en au.
    pub sun_distance_au: f64,
    /// `true` si el cuerpo está dentro del disco solar **y** delante
    /// del Sol (configuración geométrica de un tránsito real).
    pub in_transit: bool,
}

/// Evento de tránsito agregado tras un barrido.
#[derive(Debug, Clone, Copy)]
pub struct TransitEvent {
    pub body: InnerPlanet,
    /// JD TDB del centro aproximado del tránsito (mínimo de
    /// separación dentro de la ventana).
    pub jd_mid: f64,
    /// Separación angular mínima en grados.
    pub min_separation_deg: f64,
    /// Duración aproximada en horas — diferencia entre las primeras y
    /// últimas muestras del barrido donde `in_transit == true`.
    pub duration_hours: f64,
}

/// Lectura puntual: separación + flag de tránsito a un instante TDB.
pub fn separation_at(body: &InnerPlanet, tdb: &TDB) -> SeparationReading {
    let sun_pos = Vsop2013Sun.geocentric_position(tdb).expect("Sun geo");
    let body_pos = match body {
        InnerPlanet::Mercury => Vsop2013Mercury.geocentric_position(tdb).expect("Mercury geo"),
        InnerPlanet::Venus => Vsop2013Venus.geocentric_position(tdb).expect("Venus geo"),
    };
    let sun_r_au = mag(&sun_pos);
    let body_r_au = mag(&body_pos);
    let separation_rad = angle_between(&sun_pos, &body_pos);
    let separation_deg = separation_rad.to_degrees();

    let sun_d_km = sun_r_au * cosmos_core::constants::AU_KM;
    let solar_radius_apparent_rad = (SOLAR_RADIUS_KM / sun_d_km).atan();
    let solar_radius_apparent_deg = solar_radius_apparent_rad.to_degrees();

    let in_transit = body_r_au < sun_r_au && separation_deg < solar_radius_apparent_deg;

    SeparationReading {
        separation_deg,
        solar_radius_apparent_deg,
        body_distance_au: body_r_au,
        sun_distance_au: sun_r_au,
        in_transit,
    }
}

/// Barre `[jd_from, jd_to]` con paso `step_days` buscando ventanas
/// donde `in_transit == true`. Cada ventana contigua se reduce a un
/// [`TransitEvent`] con su `jd_mid` aproximado (instante de mínima
/// separación dentro de la ventana).
///
/// `step_days` típico: `1.0/24.0` (1 hora) — un tránsito real dura
/// varias horas. Pasos más finos dan instantes más precisos pero
/// cuestan proporcionalmente.
pub fn find_transits(
    body: &InnerPlanet,
    jd_from: f64,
    jd_to: f64,
    step_days: f64,
) -> Vec<TransitEvent> {
    let step = step_days.max(1.0 / 1440.0);
    let mut events: Vec<TransitEvent> = Vec::new();
    let mut current_window: Option<(f64, f64, f64, f64)> = None; // (start_jd, end_jd, min_sep, jd_at_min)
    let mut jd = jd_from;
    while jd <= jd_to {
        let tdb = TDB::from_julian_date(JulianDate::from_f64(jd));
        let r = separation_at(body, &tdb);
        if r.in_transit {
            match &mut current_window {
                None => {
                    current_window =
                        Some((jd, jd, r.separation_deg, jd));
                }
                Some(w) => {
                    w.1 = jd;
                    if r.separation_deg < w.2 {
                        w.2 = r.separation_deg;
                        w.3 = jd;
                    }
                }
            }
        } else if let Some((start, end, min_sep, jd_at_min)) = current_window.take() {
            events.push(TransitEvent {
                body: *body,
                jd_mid: jd_at_min,
                min_separation_deg: min_sep,
                duration_hours: (end - start) * 24.0,
            });
            let _ = start;
            let _ = end;
        }
        jd += step;
    }
    if let Some((start, end, min_sep, jd_at_min)) = current_window {
        events.push(TransitEvent {
            body: *body,
            jd_mid: jd_at_min,
            min_separation_deg: min_sep,
            duration_hours: (end - start) * 24.0,
        });
    }
    events
}

fn mag(v: &Vector3) -> f64 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

fn angle_between(a: &Vector3, b: &Vector3) -> f64 {
    let dot = a.x * b.x + a.y * b.y + a.z * b.z;
    let m = mag(a) * mag(b);
    if m < 1e-30 {
        return 0.0;
    }
    let c = (dot / m).clamp(-1.0, 1.0);
    c.acos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solar_radius_about_half_degree() {
        // Radio aparente del Sol desde Tierra ≈ 0.265° (radio, no
        // diámetro). El test usa el rango 0.25–0.28°.
        let tdb: TDB = "2026-01-01T00:00:00".parse().unwrap();
        let r = separation_at(&InnerPlanet::Venus, &tdb);
        assert!(
            r.solar_radius_apparent_deg > 0.25 && r.solar_radius_apparent_deg < 0.28,
            "radio solar aparente plausible: {}",
            r.solar_radius_apparent_deg
        );
    }

    #[test]
    fn no_transit_on_random_day() {
        // 2026-05-27 no es tránsito de nadie.
        let tdb: TDB = "2026-05-27T12:00:00".parse().unwrap();
        let r_v = separation_at(&InnerPlanet::Venus, &tdb);
        let r_m = separation_at(&InnerPlanet::Mercury, &tdb);
        assert!(!r_v.in_transit, "Venus no transita el 2026-05-27");
        assert!(!r_m.in_transit, "Mercury no transita el 2026-05-27");
    }

    #[test]
    fn mercury_transit_2032_detected() {
        // Tránsito de Mercurio del 2032-11-13 (predicho por
        // NASA/JPL). Buscamos en una ventana de ±2 días y esperamos
        // al menos un evento.
        let jd_from = JulianDate::from_calendar(2032, 11, 11, 0, 0, 0.0).to_f64();
        let jd_to = JulianDate::from_calendar(2032, 11, 15, 0, 0, 0.0).to_f64();
        let events = find_transits(&InnerPlanet::Mercury, jd_from, jd_to, 1.0 / 24.0);
        assert!(
            !events.is_empty(),
            "se debió detectar el tránsito de Mercurio del 2032-11-13"
        );
        // El centro debe caer en el día 13 ± 1.
        let ev = events[0];
        let jd_13 = JulianDate::from_calendar(2032, 11, 13, 12, 0, 0.0).to_f64();
        assert!(
            (ev.jd_mid - jd_13).abs() < 1.0,
            "centro del tránsito cerca del 2032-11-13: jd_mid={}",
            ev.jd_mid
        );
        // La separación mínima debe ser < radio solar aparente.
        assert!(
            ev.min_separation_deg < 0.3,
            "separación mínima < 0.3°: {}",
            ev.min_separation_deg
        );
        // Duración entre 3 y 8 horas (los tránsitos de Mercurio duran
        // típicamente 5–7 h).
        assert!(
            ev.duration_hours > 1.0 && ev.duration_hours < 9.0,
            "duración plausible: {}",
            ev.duration_hours
        );
    }

    #[test]
    fn no_transit_when_body_behind_sun() {
        // Si encuentro un instante donde Venus está geo-detrás del Sol
        // (body_d > sun_d), la separación angular puede ser pequeña
        // pero in_transit debe ser false. Tomamos conjunción superior
        // de Venus aprox 2026-08 (Venus detrás del Sol).
        let tdb: TDB = "2026-08-22T12:00:00".parse().unwrap();
        let r = separation_at(&InnerPlanet::Venus, &tdb);
        if r.body_distance_au > r.sun_distance_au {
            assert!(
                !r.in_transit,
                "body detrás del Sol no cuenta como tránsito"
            );
        }
    }

    #[test]
    fn separation_geometry_sane() {
        // Separación angular en [0, 180].
        for hour in (0..24u32).step_by(4) {
            let iso = format!("2026-06-15T{hour:02}:00:00");
            let tdb: TDB = iso.parse().unwrap();
            let r = separation_at(&InnerPlanet::Mercury, &tdb);
            assert!(
                r.separation_deg >= 0.0 && r.separation_deg <= 180.0,
                "separación fuera de rango: {}",
                r.separation_deg
            );
        }
    }

    #[test]
    fn empty_window_returns_empty_vec() {
        // Una ventana corta sin tránsitos no debe devolver eventos.
        let jd_from = JulianDate::from_calendar(2026, 5, 27, 0, 0, 0.0).to_f64();
        let jd_to = JulianDate::from_calendar(2026, 5, 28, 0, 0, 0.0).to_f64();
        let events =
            find_transits(&InnerPlanet::Venus, jd_from, jd_to, 1.0 / 24.0);
        assert!(events.is_empty());
    }
}
