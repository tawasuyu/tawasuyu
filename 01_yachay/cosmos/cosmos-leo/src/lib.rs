//! `cosmos-leo` — propagador SGP4 + parser TLE para satélites
//! artificiales.
//!
//! Sexto extracto del cosmos-ephem puro. Es la única pieza del kernel
//! que mira **basura espacial humana** en vez de cuerpos naturales:
//! Estación Espacial Internacional, Starlink, Hubble, geoestacionarios.
//!
//! ## Capa fina sobre `sgp4`
//!
//! La librería externa [`sgp4`](https://crates.io/crates/sgp4) hace todo
//! el trabajo numérico (SGP4 + SDP4 en Rust puro). Este crate aporta:
//!
//! - Parser TLE robusto a líneas con o sin nombre (2LE / 3LE).
//! - Una API `Satellite::propagate(datetime)` que devuelve un `State`
//!   en el marco TEME (True Equator Mean Equinox).
//! - Conversión TEME → ECEF (por rotación GMST) → topocéntrico
//!   (`altitude_deg`, `azimuth_deg`, `range_km`) usando una
//!   [`cosmos_core::Location`].
//!
//! ## Precisión
//!
//! SGP4 hereda la precisión del TLE: típicamente ~1 km al epoch,
//! degradándose ~1-3 km/día (LEO) por arrastre atmosférico no
//! modelado. Para predicciones de pasos visibles ISS desde una
//! ubicación es más que suficiente; para conjunciones o re-entradas
//! se necesita TLE muy reciente (< 24 h).
//!
//! La conversión TEME → ECEF usa GMST IAU 1982 sin nutación/precession
//! correction (suficiente para 1 km de error a la distancia de un LEO).

#![forbid(unsafe_code)]

use chrono::NaiveDateTime;
use cosmos_core::Location;
use sgp4::{Constants, Elements, MinutesSinceEpoch, Prediction};

const RAD_PER_DEG: f64 = std::f64::consts::PI / 180.0;
const DEG_PER_RAD: f64 = 180.0 / std::f64::consts::PI;

/// Un satélite parseado a partir de un TLE, listo para propagar.
pub struct Satellite {
    name: String,
    catalog_number: u64,
    constants: Constants,
    elements: Elements,
}

impl Satellite {
    /// Nombre del satélite (línea 0 del 3LE, o "NORAD-12345" si era 2LE).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Número de catálogo NORAD.
    pub fn catalog_number(&self) -> u64 {
        self.catalog_number
    }

    /// Periodo orbital en minutos (derivado del mean motion del TLE).
    pub fn period_minutes(&self) -> f64 {
        // mean motion (rev/día) → periodo (min): 1440 / mean_motion
        1440.0 / self.elements.mean_motion
    }

    /// Inclinación orbital en grados.
    pub fn inclination_deg(&self) -> f64 {
        self.elements.inclination
    }

    /// Propaga el estado del satélite al instante `datetime` (UTC).
    ///
    /// Devuelve un [`SatState`] en el marco TEME. Use
    /// [`SatState::to_topocentric`] para convertir a alt/az desde una
    /// ubicación.
    pub fn propagate(&self, datetime: NaiveDateTime) -> Result<SatState, PropagateError> {
        let minutes = self
            .elements
            .datetime_to_minutes_since_epoch(&datetime)
            .map_err(|e| PropagateError::DatetimeOutOfRange(e.to_string()))?;
        let pred = self
            .constants
            .propagate(MinutesSinceEpoch(minutes.0))
            .map_err(|e| PropagateError::Sgp4(e.to_string()))?;
        Ok(SatState {
            datetime,
            teme_position_km: pred.position,
            teme_velocity_km_s: pred.velocity,
        })
    }

    /// Variante interna que expone el Prediction crudo — útil para
    /// integraciones que ya tienen GMST.
    pub fn raw_propagate(
        &self,
        minutes_since_epoch: f64,
    ) -> Result<Prediction, PropagateError> {
        self.constants
            .propagate(MinutesSinceEpoch(minutes_since_epoch))
            .map_err(|e| PropagateError::Sgp4(e.to_string()))
    }
}

/// Estado del satélite tras propagar SGP4. Posición y velocidad en TEME
/// (True Equator, Mean Equinox of epoch).
#[derive(Debug, Clone, Copy)]
pub struct SatState {
    /// Instante en que se calculó la propagación.
    pub datetime: NaiveDateTime,
    /// Posición en km, marco TEME.
    pub teme_position_km: [f64; 3],
    /// Velocidad en km/s, marco TEME.
    pub teme_velocity_km_s: [f64; 3],
}

impl SatState {
    /// Magnitud del vector posición = distancia al centro de la Tierra
    /// en km.
    pub fn geocentric_distance_km(&self) -> f64 {
        let p = self.teme_position_km;
        (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()
    }

    /// Altitud sobre el geoide ecuatorial nominal (R = 6378.137 km).
    pub fn altitude_above_earth_km(&self) -> f64 {
        self.geocentric_distance_km() - 6378.137
    }

    /// Convierte el estado a coordenadas ECEF (Earth-Centered
    /// Earth-Fixed): rota por GMST(jd_ut1).
    pub fn to_ecef(&self) -> [f64; 3] {
        let jd = naive_datetime_to_jd_ut1(&self.datetime);
        let theta_rad = gmst_rad(jd);
        rotate_z(&self.teme_position_km, -theta_rad)
    }

    /// Coordenadas topocéntricas (alt, az, range_km) desde una
    /// `Location` en el momento del estado.
    pub fn to_topocentric(&self, location: &Location) -> TopoState {
        let ecef_sat = self.to_ecef();
        // ECEF del observador.
        let (xo, yo, zo) = location_to_ecef(location);
        let dx = ecef_sat[0] - xo;
        let dy = ecef_sat[1] - yo;
        let dz = ecef_sat[2] - zo;
        // Rotación ECEF → ENU (East, North, Up).
        let lat = location.latitude_degrees() * RAD_PER_DEG;
        let lon = location.longitude_degrees() * RAD_PER_DEG;
        let sin_lat = lat.sin();
        let cos_lat = lat.cos();
        let sin_lon = lon.sin();
        let cos_lon = lon.cos();
        let e = -sin_lon * dx + cos_lon * dy;
        let n = -sin_lat * cos_lon * dx - sin_lat * sin_lon * dy + cos_lat * dz;
        let u = cos_lat * cos_lon * dx + cos_lat * sin_lon * dy + sin_lat * dz;

        let range_km = (e * e + n * n + u * u).sqrt();
        let altitude_deg = (u / range_km.max(1e-30)).asin() * DEG_PER_RAD;
        let mut azimuth_deg = e.atan2(n) * DEG_PER_RAD;
        if azimuth_deg < 0.0 {
            azimuth_deg += 360.0;
        }
        TopoState {
            altitude_deg,
            azimuth_deg,
            range_km,
            above_horizon: altitude_deg > 0.0,
        }
    }
}

/// Estado topocéntrico de un satélite desde una ubicación.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TopoState {
    /// Altura sobre el horizonte local en grados.
    pub altitude_deg: f64,
    /// Azimut topocéntrico (0=N, 90=E, 180=S, 270=W), grados.
    pub azimuth_deg: f64,
    /// Distancia al observador, km.
    pub range_km: f64,
    /// `true` si el satélite está sobre el horizonte.
    pub above_horizon: bool,
}

/// Errores al parsear un TLE.
#[derive(Debug, Clone)]
pub enum ParseError {
    /// El bloque no contiene un TLE válido.
    Invalid(String),
    /// La construcción de constantes SGP4 falló (típicamente por
    /// eccentricidad fuera de rango).
    Sgp4(String),
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::Invalid(s) => write!(f, "TLE inválido: {s}"),
            ParseError::Sgp4(s) => write!(f, "SGP4 init falló: {s}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Errores al propagar SGP4.
#[derive(Debug, Clone)]
pub enum PropagateError {
    /// El datetime está fuera del rango aceptable.
    DatetimeOutOfRange(String),
    /// El integrador SGP4 falló (típicamente decay = satélite ya
    /// re-entró a la atmósfera).
    Sgp4(String),
}

impl core::fmt::Display for PropagateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PropagateError::DatetimeOutOfRange(s) => write!(f, "fuera de rango: {s}"),
            PropagateError::Sgp4(s) => write!(f, "SGP4 propagate falló: {s}"),
        }
    }
}

impl std::error::Error for PropagateError {}

/// Parsea un TLE 2LE (sin nombre) o 3LE (con nombre como línea 0). Si
/// el bloque tiene más de un satélite, sólo se devuelve el primero —
/// use [`parse_all`] para una lista completa.
pub fn parse_tle(text: &str) -> Result<Satellite, ParseError> {
    let sats = parse_all(text)?;
    sats.into_iter()
        .next()
        .ok_or_else(|| ParseError::Invalid("ningún satélite en el bloque".into()))
}

/// Parsea uno o más TLEs de un bloque (típicamente vienen en concatenación
/// 3LE desde Celestrak). Acepta líneas con CRLF o LF, ignora líneas vacías.
pub fn parse_all(text: &str) -> Result<Vec<Satellite>, ParseError> {
    // Normalizamos los saltos de línea y removemos líneas vacías.
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return Err(ParseError::Invalid("texto vacío".into()));
    }

    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].starts_with('1') && i + 1 < lines.len() && lines[i + 1].starts_with('2') {
            // 2LE.
            let elements = Elements::from_tle(None, lines[i].as_bytes(), lines[i + 1].as_bytes())
                .map_err(|e| ParseError::Invalid(e.to_string()))?;
            let cat = parse_catalog_number(lines[i]);
            let name = format!("NORAD-{cat}");
            let constants = Constants::from_elements(&elements)
                .map_err(|e| ParseError::Sgp4(e.to_string()))?;
            out.push(Satellite {
                name,
                catalog_number: cat,
                constants,
                elements,
            });
            i += 2;
        } else if i + 2 < lines.len()
            && lines[i + 1].starts_with('1')
            && lines[i + 2].starts_with('2')
        {
            // 3LE.
            let name = lines[i].trim().trim_start_matches('0').trim().to_string();
            let elements = Elements::from_tle(
                Some(name.clone()),
                lines[i + 1].as_bytes(),
                lines[i + 2].as_bytes(),
            )
            .map_err(|e| ParseError::Invalid(e.to_string()))?;
            let cat = parse_catalog_number(lines[i + 1]);
            let constants = Constants::from_elements(&elements)
                .map_err(|e| ParseError::Sgp4(e.to_string()))?;
            out.push(Satellite {
                name,
                catalog_number: cat,
                constants,
                elements,
            });
            i += 3;
        } else {
            return Err(ParseError::Invalid(format!("no se pudo parsear desde línea {i}")));
        }
    }
    Ok(out)
}

/// Lee los caracteres 3-7 de la línea 1 como número de catálogo NORAD.
fn parse_catalog_number(line1: &str) -> u64 {
    line1
        .get(2..7)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Convierte una `Location` (lat, lon, alt_msl) a coordenadas ECEF
/// usando el modelo WGS84 elipsoidal.
fn location_to_ecef(location: &Location) -> (f64, f64, f64) {
    const A: f64 = 6378.137; // semieje mayor WGS84, km
    const F: f64 = 1.0 / 298.257_223_563;
    let e2 = F * (2.0 - F);
    let lat = location.latitude_degrees() * RAD_PER_DEG;
    let lon = location.longitude_degrees() * RAD_PER_DEG;
    let alt_km = location.height / 1000.0;
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let n = A / (1.0 - e2 * sin_lat * sin_lat).sqrt();
    let x = (n + alt_km) * cos_lat * lon.cos();
    let y = (n + alt_km) * cos_lat * lon.sin();
    let z = (n * (1.0 - e2) + alt_km) * sin_lat;
    (x, y, z)
}

fn naive_datetime_to_jd_ut1(dt: &NaiveDateTime) -> f64 {
    use chrono::{Datelike, Timelike};
    let y = dt.year();
    let m = dt.month() as i32;
    let d = dt.day() as f64;
    let h = dt.hour() as f64;
    let mi = dt.minute() as f64;
    let s = dt.second() as f64;
    let frac_day = (h + (mi + s / 60.0) / 60.0) / 24.0;
    // Gregorian JD (Meeus 7.1).
    let (yy, mm) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
    let a = (yy as f64 / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    let jd_int = (365.25 * (yy as f64 + 4716.0)).floor()
        + (30.6001 * (mm as f64 + 1.0)).floor()
        + d
        + b
        - 1524.5;
    jd_int + frac_day
}

fn gmst_rad(jd_ut1: f64) -> f64 {
    let t = (jd_ut1 - 2451545.0) / 36525.0;
    let secs = 67310.548_41
        + (876_600.0 * 3600.0 + 8_640_184.812_866) * t
        + 0.093_104 * t * t
        - 6.2e-6 * t * t * t;
    let hours = (secs / 3600.0).rem_euclid(24.0);
    hours * 15.0 * RAD_PER_DEG
}

fn rotate_z(v: &[f64; 3], theta_rad: f64) -> [f64; 3] {
    let c = theta_rad.cos();
    let s = theta_rad.sin();
    [c * v[0] - s * v[1], s * v[0] + c * v[1], v[2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    // ISS TLE de epoch 2020-07-12 21:15 UTC (frozen, validado contra
    // la documentación del crate `sgp4`).
    const ISS_TLE: &str = "ISS (ZARYA)
1 25544U 98067A   20194.88612269 -.00002218  00000-0 -31515-4 0  9992
2 25544  51.6461 221.2784 0001413  89.1723 280.4612 15.49507896236008";

    fn iss_epoch_noon() -> NaiveDateTime {
        // Epoch ISS_TLE ≈ 2020-07-12 21:15 UTC. Probamos a las 12:00
        // del mismo día — propagación negativa de < 10 h, todavía
        // dentro del rango usable de SGP4.
        NaiveDate::from_ymd_opt(2020, 7, 12)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
    }

    fn lima() -> Location {
        Location::from_degrees(-12.05, -77.05, 150.0).unwrap()
    }

    fn quito() -> Location {
        Location::from_degrees(0.0, -78.5, 2850.0).unwrap()
    }

    #[test]
    fn parse_3le_iss() {
        let sat = parse_tle(ISS_TLE).expect("ISS parseable");
        assert_eq!(sat.name(), "ISS (ZARYA)");
        assert_eq!(sat.catalog_number(), 25544);
        // ISS inclinación ~ 51.6°.
        assert!(
            (sat.inclination_deg() - 51.64).abs() < 0.1,
            "inclinación ISS ~ 51.64°, fue {}",
            sat.inclination_deg()
        );
        // Periodo ~ 92.9 min.
        assert!(
            (sat.period_minutes() - 92.9).abs() < 1.0,
            "periodo ISS ~ 92.9 min, fue {}",
            sat.period_minutes()
        );
    }

    #[test]
    fn parse_2le_works() {
        // Mismo ISS sin la línea 0.
        let tle_2le =
            "1 25544U 98067A   20194.88612269 -.00002218  00000-0 -31515-4 0  9992
2 25544  51.6461 221.2784 0001413  89.1723 280.4612 15.49507896236008";
        let sat = parse_tle(tle_2le).expect("ISS 2LE parseable");
        assert_eq!(sat.catalog_number(), 25544);
        assert!(sat.name().contains("25544"));
    }

    #[test]
    fn iss_altitude_in_leo_range() {
        // ISS orbita a ~ 400-420 km sobre la superficie.
        let sat = parse_tle(ISS_TLE).unwrap();
        let state = sat.propagate(iss_epoch_noon()).expect("propaga");
        let alt = state.altitude_above_earth_km();
        assert!(
            alt > 380.0 && alt < 450.0,
            "altitud ISS plausible (400 km): {alt}"
        );
    }

    #[test]
    fn iss_topocentric_in_range() {
        let sat = parse_tle(ISS_TLE).unwrap();
        let state = sat.propagate(iss_epoch_noon()).unwrap();
        let topo = state.to_topocentric(&lima());
        assert!(
            topo.altitude_deg >= -90.0 && topo.altitude_deg <= 90.0,
            "alt en [-90,90]: {}",
            topo.altitude_deg
        );
        assert!(
            topo.azimuth_deg >= 0.0 && topo.azimuth_deg < 360.0,
            "az en [0,360): {}",
            topo.azimuth_deg
        );
        // Cota superior: 2R + h ≈ 13.2k km (satélite del otro lado del
        // planeta atravesando el centro). Cota inferior: ISS perigeo
        // h ≈ 380 km, pero como el satélite puede estar 90° del cenit
        // observador, queda al menos a la distancia a la superficie =
        // sqrt(2)·R + ε.
        assert!(
            topo.range_km > 380.0 && topo.range_km < 13_200.0,
            "range LEO en cota geométrica posible: {}",
            topo.range_km
        );
    }

    #[test]
    fn iss_completes_orbit_in_about_93_min() {
        // Al cabo de un periodo, la posición debe estar cerca de la
        // inicial.
        let sat = parse_tle(ISS_TLE).unwrap();
        let t0 = iss_epoch_noon();
        let t1 = t0 + chrono::Duration::seconds((sat.period_minutes() * 60.0) as i64);
        let s0 = sat.propagate(t0).unwrap();
        let s1 = sat.propagate(t1).unwrap();
        let dx = s1.teme_position_km[0] - s0.teme_position_km[0];
        let dy = s1.teme_position_km[1] - s0.teme_position_km[1];
        let dz = s1.teme_position_km[2] - s0.teme_position_km[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        assert!(
            dist < 80.0,
            "Tras 1 periodo la ISS debe volver a ~ misma posición: dist={dist}"
        );
    }

    #[test]
    fn iss_visible_pass_exists_within_24h() {
        // En 24 h la ISS debe pasar al menos una vez sobre Lima con
        // alt > 10° (la ISS orbita ~16 veces/día, Lima está bajo su
        // franja de inclinación 51.6°).
        let sat = parse_tle(ISS_TLE).unwrap();
        let t0 = iss_epoch_noon();
        let mut found = false;
        for m in 0..(24 * 60) {
            let t = t0 + chrono::Duration::minutes(m);
            let topo = sat.propagate(t).unwrap().to_topocentric(&lima());
            if topo.altitude_deg > 10.0 {
                found = true;
                break;
            }
        }
        assert!(found, "al menos un paso visible sobre Lima en 24 h");
    }

    #[test]
    fn rotate_z_basic() {
        // Rotación 90° del eje x debe dar eje y.
        let v = [1.0, 0.0, 0.0];
        let r = rotate_z(&v, std::f64::consts::FRAC_PI_2);
        assert!(r[0].abs() < 1e-10);
        assert!((r[1] - 1.0).abs() < 1e-10);
        assert!(r[2].abs() < 1e-10);
    }

    #[test]
    fn ecef_observer_at_equator_quito() {
        // Quito (0°, -78.5°, 2850 m): ECEF (R+h)·(cos lon, sin lon, 0).
        let (x, y, z) = location_to_ecef(&quito());
        let r = (x * x + y * y + z * z).sqrt();
        // r ≈ 6381 km (radio Tierra + altitud + ligera variación elipsoide).
        assert!(
            r > 6378.0 && r < 6385.0,
            "r Quito desde geocentro: {r} km"
        );
        assert!(z.abs() < 1.0, "z Quito ≈ 0 (lat=0): {z}");
    }

    #[test]
    fn parse_multiple_3les() {
        // Segundo bloque idéntico al ISS pero con nombre "ALIAS" — el
        // parser debe devolver dos satélites con el mismo NORAD.
        let alias = ISS_TLE.replacen("ISS (ZARYA)", "ALIAS", 1);
        let block = format!("{ISS_TLE}\n{alias}");
        let sats = parse_all(&block).unwrap();
        assert_eq!(sats.len(), 2);
        assert_eq!(sats[0].catalog_number(), 25544);
        assert_eq!(sats[1].catalog_number(), 25544);
        assert_eq!(sats[0].name(), "ISS (ZARYA)");
        assert_eq!(sats[1].name(), "ALIAS");
    }
}
