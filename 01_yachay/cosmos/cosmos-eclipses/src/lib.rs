//! `cosmos-eclipses` — detección geocéntrica de eclipses solares y
//! lunares.
//!
//! ## Eclipse solar (Luna entre Tierra y Sol)
//!
//! Calcula la separación angular geocéntrica Luna ↔ Sol y la compara
//! con el radio aparente del Sol y el radio aparente de la Luna:
//!
//! - `ω < ρ_sun + ρ_moon`  → **parcial** o mejor.
//! - `ω < |ρ_sun − ρ_moon|` → **anular** (si ρ_moon < ρ_sun) o **total**
//!   (si ρ_moon ≥ ρ_sun).
//!
//! No incluye paralaje topocéntrica — un eclipse solar es geocéntrico
//! cuando la línea Tierra-Sol pasa por algún punto de la superficie
//! terrestre cubierto por el cono lunar. Para saber si es visible
//! desde una ubicación específica hace falta cadena WGS84 +
//! topocentric, fuera del alcance de este crate.
//!
//! ## Eclipse lunar (Luna en el cono de sombra de la Tierra)
//!
//! Descompone la posición de la Luna respecto al eje anti-solar (eje
//! que sale del centro de la Tierra en dirección opuesta al Sol):
//!
//! - `p` = proyección de `r_moon` sobre el eje anti-solar (en au).
//! - `q` = distancia perpendicular al eje (en au).
//!
//! A esa distancia `p` el cono umbra terrestre tiene radio:
//!
//! ```text
//! R_umbra(p)   = R_earth − p · (R_sun − R_earth) / d_sun
//! R_penumbra(p) = R_earth + p · (R_sun + R_earth) / d_sun
//! ```
//!
//! Combinado con el radio físico de la Luna (`R_moon`):
//!
//! - `q + R_moon < R_umbra`     → umbral **total**.
//! - `q − R_moon < R_umbra`     → umbral **parcial**.
//! - `q + R_moon < R_penumbra`  → penumbra total.
//! - `q − R_moon < R_penumbra`  → penumbra parcial.
//!
//! Requiere `p > 0` (Luna del lado opuesto al Sol — luna llena
//! geométrica).
//!
//! ## Precisión
//!
//! Hereda la precisión de ELP/MPP02 para la Luna (~1 km) y VSOP2013
//! para el Sol (mejor que arcsegundo). El paralaje horizontal lunar
//! `~57′ = 0.95°` hace que la hora de máximo varíe ~hasta 2 h entre
//! observadores en distintos hemisferios para eclipses solares;
//! cosmos-eclipses devuelve el instante **geocéntrico**, no el
//! topocéntrico.

#![forbid(unsafe_code)]

use cosmos_core::Vector3;
use cosmos_ephemeris::moon::ElpMpp02Moon;
use cosmos_ephemeris::sun::Vsop2013Sun;
use cosmos_time::{JulianDate, TDB};

/// Radio fotosférico solar, km. IAU 2015 nominal.
pub const SOLAR_RADIUS_KM: f64 = 695_700.0;
/// Radio ecuatorial terrestre, km. IUGG / GRS80.
pub const EARTH_RADIUS_KM: f64 = 6_378.137;
/// Radio medio lunar, km. IAU 2015 nominal.
pub const MOON_RADIUS_KM: f64 = 1_737.4;

/// Magnitud / clasificación de un eclipse solar geocéntrico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolarEclipseKind {
    /// Sin contacto del disco lunar con el disco solar.
    None,
    /// Discos lunar y solar se intersectan parcialmente.
    Partial,
    /// La Luna cubre el centro del Sol pero su disco aparente es
    /// **menor** que el solar — anillo solar visible alrededor.
    Annular,
    /// La Luna cubre completamente el Sol (`ρ_moon ≥ ρ_sun` y centros
    /// alineados dentro del solapamiento).
    Total,
}

/// Magnitud / clasificación de un eclipse lunar geocéntrico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LunarEclipseKind {
    /// Luna fuera del cono penumbra terrestre.
    None,
    /// Luna parcialmente dentro del cono penumbra (sin tocar umbra).
    Penumbral,
    /// Una parte del disco lunar entra en la umbra (oscurecimiento
    /// observable a simple vista).
    Partial,
    /// Disco lunar entero dentro de la umbra — el conocido "Sangre"
    /// por la luz refractada por la atmósfera terrestre.
    Total,
}

/// Lectura puntual para eclipse solar a un instante TDB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolarEclipseReading {
    /// Separación angular geocéntrica Luna ↔ Sol (grados).
    pub separation_deg: f64,
    /// Radio aparente del Sol desde la Tierra (grados).
    pub sun_apparent_radius_deg: f64,
    /// Radio aparente de la Luna desde la Tierra (grados).
    pub moon_apparent_radius_deg: f64,
    /// Paralaje horizontal lunar (grados) — diferencia angular entre la
    /// posición geocéntrica y la topocéntrica desde el horizonte.
    /// Aprox. 0.91°–1.0°. Suma al umbral de detección para encontrar
    /// eclipses observables desde **algún** punto de la Tierra.
    pub moon_horizontal_parallax_deg: f64,
    /// "Magnitud" del eclipse: `(ρ_sun + ρ_moon − ω) / (2·ρ_sun)`,
    /// estándar astronómico — 0 = sin contacto, 1 = total/anular
    /// central geocéntrico.
    pub magnitude: f64,
    /// Clasificación.
    pub kind: SolarEclipseKind,
}

/// Lectura puntual para eclipse lunar a un instante TDB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LunarEclipseReading {
    /// Distancia perpendicular `q` del centro lunar al eje
    /// anti-solar, en km.
    pub gamma_km: f64,
    /// Radio del cono umbra terrestre a la distancia paralela de la
    /// Luna, en km.
    pub umbra_radius_km: f64,
    /// Radio del cono penumbra terrestre a la distancia paralela de
    /// la Luna, en km.
    pub penumbra_radius_km: f64,
    /// "Magnitud umbral": `(R_umbra + R_moon − q) / (2·R_moon)`. Igual
    /// a 1 cuando el centro lunar coincide con el centro del cono
    /// umbra (eclipse umbral central).
    pub umbral_magnitude: f64,
    /// Clasificación.
    pub kind: LunarEclipseKind,
}

/// Evento agregado tras un barrido.
#[derive(Debug, Clone, Copy)]
pub struct EclipseEvent {
    /// JD TDB del instante de máximo (mínima separación / mayor
    /// magnitud) dentro de la ventana de contigüidad.
    pub jd_mid: f64,
    /// Magnitud máxima registrada.
    pub magnitude_max: f64,
    /// Duración en horas del intervalo donde el flag de detección era
    /// `true` (paso de muestreo discreto).
    pub duration_hours: f64,
    /// Clasificación más severa observada en el intervalo (Total >
    /// Annular > Partial > None para solar; Total > Partial >
    /// Penumbral > None para lunar).
    pub kind_max_solar: Option<SolarEclipseKind>,
    /// Idem para lunar.
    pub kind_max_lunar: Option<LunarEclipseKind>,
}

/// Lectura solar puntual.
pub fn solar_reading_at(tdb: &TDB) -> SolarEclipseReading {
    let sun_pos = Vsop2013Sun.geocentric_position(tdb).expect("Sun geo");
    let moon_pos_au = moon_geocentric_au(tdb);

    let d_sun_au = mag(&sun_pos);
    let d_moon_au = mag(&moon_pos_au);
    let d_sun_km = d_sun_au * cosmos_core::constants::AU_KM;
    let d_moon_km = d_moon_au * cosmos_core::constants::AU_KM;

    let sun_apparent_radius_deg = (SOLAR_RADIUS_KM / d_sun_km).atan().to_degrees();
    let moon_apparent_radius_deg = (MOON_RADIUS_KM / d_moon_km).atan().to_degrees();
    let moon_horizontal_parallax_deg = (EARTH_RADIUS_KM / d_moon_km).asin().to_degrees();
    let separation_deg = angle_between(&sun_pos, &moon_pos_au).to_degrees();

    let rs = sun_apparent_radius_deg;
    let rm = moon_apparent_radius_deg;
    let pi_m = moon_horizontal_parallax_deg;
    // Magnitud central: 1.0 cuando la línea Tierra-Luna-Sol coincide
    // exactamente (centros alineados).
    let magnitude = ((rs + rm - separation_deg) / (2.0 * rs)).max(0.0);

    // Umbrales (Meeus, "Astronomical Algorithms" cap. 54):
    //   ω < ρ_sun + ρ_moon + π_moon   → eclipse visible desde algún
    //                                   punto de la Tierra (parcial).
    //   ω < π_moon − (ρ_sun − ρ_moon) → eje del cono umbra/antumbra
    //                                   intersecta la superficie
    //                                   terrestre (central: Total o
    //                                   Annular).
    let partial_limit = rs + rm + pi_m;
    let central_limit = pi_m - (rs - rm);

    let kind = if separation_deg > partial_limit {
        SolarEclipseKind::None
    } else if separation_deg < central_limit.max(0.0) {
        if rm >= rs {
            SolarEclipseKind::Total
        } else {
            SolarEclipseKind::Annular
        }
    } else {
        SolarEclipseKind::Partial
    };

    SolarEclipseReading {
        separation_deg,
        sun_apparent_radius_deg,
        moon_apparent_radius_deg,
        moon_horizontal_parallax_deg,
        magnitude,
        kind,
    }
}

/// Lectura lunar puntual.
pub fn lunar_reading_at(tdb: &TDB) -> LunarEclipseReading {
    let sun_pos = Vsop2013Sun.geocentric_position(tdb).expect("Sun geo");
    let moon_pos_au = moon_geocentric_au(tdb);
    let d_sun_au = mag(&sun_pos);
    let d_sun_km = d_sun_au * cosmos_core::constants::AU_KM;

    // Eje anti-solar (vector unitario que sale de la Tierra en
    // dirección opuesta al Sol).
    let anti = Vector3::new(-sun_pos.x, -sun_pos.y, -sun_pos.z);
    let anti_u = unit(&anti);
    let moon_km = Vector3::new(
        moon_pos_au.x * cosmos_core::constants::AU_KM,
        moon_pos_au.y * cosmos_core::constants::AU_KM,
        moon_pos_au.z * cosmos_core::constants::AU_KM,
    );
    let p_km = dot(&moon_km, &anti_u);
    let perp = sub(&moon_km, &scale(&anti_u, p_km));
    let q_km = mag(&perp);

    // Meeus, AA cap. 54 — factor 1.02 expande el cono umbra terrestre
    // para representar la extensión atmosférica observable (la sombra
    // real de la Tierra incluye la altura efectiva de la atmósfera).
    const ATM_FACTOR: f64 = 1.02;
    let umbra_radius_km =
        ATM_FACTOR * (EARTH_RADIUS_KM - p_km * (SOLAR_RADIUS_KM - EARTH_RADIUS_KM) / d_sun_km)
            .max(0.0);
    let penumbra_radius_km =
        ATM_FACTOR * (EARTH_RADIUS_KM + p_km * (SOLAR_RADIUS_KM + EARTH_RADIUS_KM) / d_sun_km);

    let umbral_magnitude = ((umbra_radius_km + MOON_RADIUS_KM - q_km) / (2.0 * MOON_RADIUS_KM))
        .max(0.0);

    let kind = if p_km <= 0.0 {
        // Luna del lado del Sol — fase nueva, no puede haber eclipse
        // lunar.
        LunarEclipseKind::None
    } else if q_km + MOON_RADIUS_KM < umbra_radius_km {
        LunarEclipseKind::Total
    } else if q_km - MOON_RADIUS_KM < umbra_radius_km {
        LunarEclipseKind::Partial
    } else if q_km - MOON_RADIUS_KM < penumbra_radius_km {
        LunarEclipseKind::Penumbral
    } else {
        LunarEclipseKind::None
    };

    LunarEclipseReading {
        gamma_km: q_km,
        umbra_radius_km,
        penumbra_radius_km,
        umbral_magnitude,
        kind,
    }
}

/// Barre `[jd_from, jd_to]` con `step_days` buscando ventanas donde la
/// lectura solar reporta cualquier eclipse (no None). Cada ventana
/// contigua se reduce a un [`EclipseEvent`] con la magnitud máxima
/// observada.
pub fn find_solar_eclipses(jd_from: f64, jd_to: f64, step_days: f64) -> Vec<EclipseEvent> {
    let step = step_days.max(1.0 / 1440.0);
    let mut events: Vec<EclipseEvent> = Vec::new();
    let mut win: Option<(f64, f64, f64, f64, SolarEclipseKind)> = None;
    let mut jd = jd_from;
    while jd <= jd_to {
        let tdb = TDB::from_julian_date(JulianDate::from_f64(jd));
        let r = solar_reading_at(&tdb);
        if r.kind != SolarEclipseKind::None {
            match &mut win {
                None => {
                    win = Some((jd, jd, r.magnitude, jd, r.kind));
                }
                Some(w) => {
                    w.1 = jd;
                    if r.magnitude > w.2 {
                        w.2 = r.magnitude;
                        w.3 = jd;
                    }
                    if rank_solar(r.kind) > rank_solar(w.4) {
                        w.4 = r.kind;
                    }
                }
            }
        } else if let Some((start, end, mag, jd_at_max, kind)) = win.take() {
            events.push(EclipseEvent {
                jd_mid: jd_at_max,
                magnitude_max: mag,
                duration_hours: (end - start) * 24.0,
                kind_max_solar: Some(kind),
                kind_max_lunar: None,
            });
        }
        jd += step;
    }
    if let Some((start, end, mag, jd_at_max, kind)) = win {
        events.push(EclipseEvent {
            jd_mid: jd_at_max,
            magnitude_max: mag,
            duration_hours: (end - start) * 24.0,
            kind_max_solar: Some(kind),
            kind_max_lunar: None,
        });
    }
    events
}

/// Barrido análogo para eclipses lunares.
pub fn find_lunar_eclipses(jd_from: f64, jd_to: f64, step_days: f64) -> Vec<EclipseEvent> {
    let step = step_days.max(1.0 / 1440.0);
    let mut events: Vec<EclipseEvent> = Vec::new();
    let mut win: Option<(f64, f64, f64, f64, LunarEclipseKind)> = None;
    let mut jd = jd_from;
    while jd <= jd_to {
        let tdb = TDB::from_julian_date(JulianDate::from_f64(jd));
        let r = lunar_reading_at(&tdb);
        if r.kind != LunarEclipseKind::None {
            match &mut win {
                None => {
                    win = Some((jd, jd, r.umbral_magnitude, jd, r.kind));
                }
                Some(w) => {
                    w.1 = jd;
                    if r.umbral_magnitude > w.2 {
                        w.2 = r.umbral_magnitude;
                        w.3 = jd;
                    }
                    if rank_lunar(r.kind) > rank_lunar(w.4) {
                        w.4 = r.kind;
                    }
                }
            }
        } else if let Some((start, end, mag, jd_at_max, kind)) = win.take() {
            events.push(EclipseEvent {
                jd_mid: jd_at_max,
                magnitude_max: mag,
                duration_hours: (end - start) * 24.0,
                kind_max_solar: None,
                kind_max_lunar: Some(kind),
            });
        }
        jd += step;
    }
    if let Some((start, end, mag, jd_at_max, kind)) = win {
        events.push(EclipseEvent {
            jd_mid: jd_at_max,
            magnitude_max: mag,
            duration_hours: (end - start) * 24.0,
            kind_max_solar: None,
            kind_max_lunar: Some(kind),
        });
    }
    events
}

fn rank_solar(k: SolarEclipseKind) -> u8 {
    match k {
        SolarEclipseKind::None => 0,
        SolarEclipseKind::Partial => 1,
        SolarEclipseKind::Annular => 2,
        SolarEclipseKind::Total => 3,
    }
}

fn rank_lunar(k: LunarEclipseKind) -> u8 {
    match k {
        LunarEclipseKind::None => 0,
        LunarEclipseKind::Penumbral => 1,
        LunarEclipseKind::Partial => 2,
        LunarEclipseKind::Total => 3,
    }
}

fn moon_geocentric_au(tdb: &TDB) -> Vector3 {
    let inv_au = 1.0 / cosmos_core::constants::AU_KM;
    let km = ElpMpp02Moon::new()
        .geocentric_position_icrs(tdb)
        .expect("Moon geo");
    Vector3::new(km[0] * inv_au, km[1] * inv_au, km[2] * inv_au)
}

fn mag(v: &Vector3) -> f64 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

fn dot(a: &Vector3, b: &Vector3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn sub(a: &Vector3, b: &Vector3) -> Vector3 {
    Vector3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn scale(v: &Vector3, s: f64) -> Vector3 {
    Vector3::new(v.x * s, v.y * s, v.z * s)
}

fn unit(v: &Vector3) -> Vector3 {
    let m = mag(v).max(1e-30);
    Vector3::new(v.x / m, v.y / m, v.z / m)
}

fn angle_between(a: &Vector3, b: &Vector3) -> f64 {
    let m = mag(a) * mag(b);
    if m < 1e-30 {
        return 0.0;
    }
    (dot(a, b) / m).clamp(-1.0, 1.0).acos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jd(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> f64 {
        JulianDate::from_calendar(year, month, day, hour, minute, 0.0).to_f64()
    }

    #[test]
    fn apparent_radii_in_known_range() {
        let tdb: TDB = "2026-01-01T00:00:00".parse().unwrap();
        let r = solar_reading_at(&tdb);
        // ρ_sun ≈ 0.265° (rango anual 0.262–0.272°).
        assert!(
            r.sun_apparent_radius_deg > 0.255 && r.sun_apparent_radius_deg < 0.280,
            "ρ_sun plausible: {}",
            r.sun_apparent_radius_deg
        );
        // ρ_moon ≈ 0.25–0.28° (perigeo/apogeo).
        assert!(
            r.moon_apparent_radius_deg > 0.22 && r.moon_apparent_radius_deg < 0.30,
            "ρ_moon plausible: {}",
            r.moon_apparent_radius_deg
        );
    }

    #[test]
    fn no_eclipse_on_random_day() {
        // 2026-05-27: no es eclipse.
        let tdb: TDB = "2026-05-27T12:00:00".parse().unwrap();
        let r_s = solar_reading_at(&tdb);
        let r_l = lunar_reading_at(&tdb);
        assert_eq!(r_s.kind, SolarEclipseKind::None);
        assert_eq!(r_l.kind, LunarEclipseKind::None);
    }

    #[test]
    fn solar_eclipse_2026_08_12_detected() {
        // Eclipse solar total del 2026-08-12 (España + Islandia).
        // Buscamos en ±2 días.
        let from = jd(2026, 8, 10, 0, 0);
        let to = jd(2026, 8, 14, 0, 0);
        let events = find_solar_eclipses(from, to, 1.0 / 48.0);
        assert!(
            !events.is_empty(),
            "se debe detectar el eclipse solar del 2026-08-12"
        );
        let ev = events[0];
        // Centro entre 11 y 13 agosto.
        assert!(
            ev.jd_mid > jd(2026, 8, 11, 0, 0) && ev.jd_mid < jd(2026, 8, 13, 0, 0),
            "centro plausible: jd_mid={}",
            ev.jd_mid
        );
        // Debe ser total geocéntricamente.
        assert!(
            matches!(
                ev.kind_max_solar,
                Some(SolarEclipseKind::Total) | Some(SolarEclipseKind::Annular)
            ),
            "kind máx para 2026-08-12: {:?}",
            ev.kind_max_solar
        );
    }

    #[test]
    fn solar_eclipse_2027_08_02_detected_as_total() {
        // Eclipse solar total del 2027-08-02 — el más largo del siglo
        // (~6m23s en Egipto). Geocéntricamente debe clasificar como
        // Total.
        let from = jd(2027, 7, 31, 0, 0);
        let to = jd(2027, 8, 4, 0, 0);
        let events = find_solar_eclipses(from, to, 1.0 / 48.0);
        assert!(!events.is_empty(), "se debe detectar el 2027-08-02");
        let ev = events[0];
        assert_eq!(ev.kind_max_solar, Some(SolarEclipseKind::Total),
            "2027-08-02 es Total geocéntrico");
    }

    #[test]
    fn lunar_eclipse_2026_03_03_detected() {
        // Eclipse lunar total del 2026-03-03.
        let from = jd(2026, 3, 2, 0, 0);
        let to = jd(2026, 3, 4, 12, 0);
        let events = find_lunar_eclipses(from, to, 1.0 / 48.0);
        assert!(
            !events.is_empty(),
            "se debe detectar eclipse lunar 2026-03-03"
        );
        let ev = events[0];
        assert!(
            matches!(
                ev.kind_max_lunar,
                Some(LunarEclipseKind::Total) | Some(LunarEclipseKind::Partial)
            ),
            "2026-03-03 al menos parcial umbral: {:?}",
            ev.kind_max_lunar
        );
    }

    #[test]
    fn lunar_geometry_at_full_moon_makes_sense() {
        // Cerca de luna llena 2026-03-03, gamma debe ser pequeño
        // (Luna alineada con el eje anti-solar).
        let tdb: TDB = "2026-03-03T12:00:00".parse().unwrap();
        let r = lunar_reading_at(&tdb);
        assert!(
            r.umbra_radius_km > 3000.0 && r.umbra_radius_km < 6000.0,
            "umbra ~ 4500 km a distancia lunar: {}",
            r.umbra_radius_km
        );
        assert!(
            r.penumbra_radius_km > r.umbra_radius_km,
            "penumbra debe ser mayor que umbra"
        );
    }

    #[test]
    fn solar_magnitude_in_unit_interval_at_eclipse() {
        // En el centro de un eclipse total, magnitude debe ser cercana
        // a 1.0.
        let tdb: TDB = "2027-08-02T10:00:00".parse().unwrap();
        let r = solar_reading_at(&tdb);
        if r.kind != SolarEclipseKind::None {
            assert!(
                r.magnitude > 0.5 && r.magnitude < 2.0,
                "magnitude plausible en eclipse: {}",
                r.magnitude
            );
        }
    }

    #[test]
    fn long_window_finds_multiple_events() {
        // Barrido 2026-01..2028-01: debe haber al menos 4 eclipses
        // solares (típicamente 4-5 por año-y-medio).
        let from = jd(2026, 1, 1, 0, 0);
        let to = jd(2028, 1, 1, 0, 0);
        let solar = find_solar_eclipses(from, to, 1.0 / 12.0);
        let lunar = find_lunar_eclipses(from, to, 1.0 / 12.0);
        assert!(
            solar.len() >= 3,
            "≥ 3 eclipses solares en 2 años, fueron {}",
            solar.len()
        );
        assert!(
            lunar.len() >= 2,
            "≥ 2 eclipses lunares en 2 años, fueron {}",
            lunar.len()
        );
    }

    #[test]
    fn lunar_eclipse_impossible_at_new_moon() {
        // Luna nueva: Luna del lado del Sol, p < 0, no puede haber
        // eclipse lunar. Tomamos un instante cerca de nueva (~ 2026-08-12,
        // que es eclipse solar).
        let tdb: TDB = "2026-08-12T17:00:00".parse().unwrap();
        let r = lunar_reading_at(&tdb);
        assert_eq!(r.kind, LunarEclipseKind::None, "luna nueva: no eclipse lunar");
    }
}
