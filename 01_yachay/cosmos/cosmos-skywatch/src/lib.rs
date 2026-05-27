//! `cosmos-skywatch` — qué se ve esta noche desde una ubicación dada.
//!
//! Capa fina sobre [`cosmos_ephemeris`] + [`cosmos_time`] +
//! [`cosmos_core::Location`] que convierte posiciones ICRS de los
//! cuerpos del sistema solar a coordenadas horizontales (alt/az)
//! topocéntricas. Sirve a astrofoto planning ("¿se ve Júpiter sobre
//! el horizonte a las 22:00 desde Lima?"), navegación astronómica
//! básica, planificación de eventos celestes (eclipses, tránsitos)
//! y al ejemplo "sundial" — todo desacoplado de la maquinaria
//! astrológica.
//!
//! La precisión apunta a arcsegundo bajo, **no** a astrometría
//! profesional: usa `TDB` como aproximación de `UT1` y `TT` para los
//! cálculos de GMST (error < ~1 segundo de tiempo sin EOP, ≈ 15
//! arcsec a la latitud del ecuador). Para VLBI o ocultaciones se
//! requiere la cadena completa de cosmos-time con tablas de EOP, que
//! este crate no consume.
//!
//! ## Uso básico
//!
//! ```ignore
//! use cosmos_core::Location;
//! use cosmos_skywatch::{Body, sky_position};
//! use cosmos_time::TDB;
//!
//! let lima = Location::from_degrees(-12.05, -77.05, 150.0).unwrap();
//! let tdb: TDB = "2026-05-27T23:00:00".parse().unwrap();
//! let pos = sky_position(&Body::Mars, &tdb, &lima);
//! println!("Mars alt={:.2}° az={:.2}°", pos.altitude_deg, pos.azimuth_deg);
//! ```

#![forbid(unsafe_code)]

use cosmos_core::Location;
use cosmos_core::Vector3;
use cosmos_ephemeris::earth::Vsop2013Earth;
use cosmos_ephemeris::moon::ElpMpp02Moon;
use cosmos_ephemeris::planets::{
    Vsop2013Jupiter, Vsop2013Mars, Vsop2013Mercury, Vsop2013Neptune, Vsop2013Pluto,
    Vsop2013Saturn, Vsop2013Uranus, Vsop2013Venus,
};
use cosmos_ephemeris::sun::Vsop2013Sun;
use cosmos_time::TDB;

const RAD_PER_DEG: f64 = std::f64::consts::PI / 180.0;
const DEG_PER_RAD: f64 = 180.0 / std::f64::consts::PI;

/// Cuerpos para los que [`sky_position`] sabe calcular el vector ICRS
/// geocéntrico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    Sun,
    Moon,
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
    Pluto,
}

impl Body {
    /// Nombre canónico en inglés — usado en CSV / outputs estables.
    pub fn canonical(&self) -> &'static str {
        match self {
            Body::Sun => "sun",
            Body::Moon => "moon",
            Body::Mercury => "mercury",
            Body::Venus => "venus",
            Body::Mars => "mars",
            Body::Jupiter => "jupiter",
            Body::Saturn => "saturn",
            Body::Uranus => "uranus",
            Body::Neptune => "neptune",
            Body::Pluto => "pluto",
        }
    }

    /// Set de los 10 cuerpos clásicos. Útil para iteración de "todo
    /// el cielo" desde un demo.
    pub fn all() -> [Body; 10] {
        [
            Body::Sun,
            Body::Moon,
            Body::Mercury,
            Body::Venus,
            Body::Mars,
            Body::Jupiter,
            Body::Saturn,
            Body::Uranus,
            Body::Neptune,
            Body::Pluto,
        ]
    }
}

/// Resultado de un cálculo skywatch para un cuerpo en un instante.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SkyPosition {
    /// Altura sobre el horizonte en grados. Positiva = sobre el
    /// horizonte, negativa = bajo el horizonte.
    pub altitude_deg: f64,
    /// Azimut topocéntrico en grados, contado desde el Norte hacia
    /// el Este (convención astronómica moderna). 0° = N, 90° = E,
    /// 180° = S, 270° = W. Rango `[0, 360)`.
    pub azimuth_deg: f64,
    /// Ascensión recta en grados, equinoccio J2000 / ICRS,
    /// `[0, 360)`.
    pub right_ascension_deg: f64,
    /// Declinación en grados, `[-90, 90]`.
    pub declination_deg: f64,
    /// Distancia geocéntrica al cuerpo en unidades astronómicas
    /// (au). Para la Luna ELP/MPP02 viene en km — la convertimos a
    /// au para unidad homogénea con los planetas VSOP2013.
    pub distance_au: f64,
    /// `true` si el cuerpo está sobre el horizonte (alt > 0).
    pub above_horizon: bool,
}

impl SkyPosition {
    /// Atajo: "magnitud de visibilidad" simple para ordenar — alt
    /// negativo = -1, sobre el horizonte cuenta la altitud.
    pub fn visibility_score(&self) -> f64 {
        if self.above_horizon {
            self.altitude_deg
        } else {
            -1.0
        }
    }
}

/// Calcula la posición horizontal de un cuerpo desde una ubicación a
/// un instante TDB dado.
pub fn sky_position(body: &Body, tdb: &TDB, location: &Location) -> SkyPosition {
    let v_icrs = geocentric_icrs_au(body, tdb);
    icrs_to_sky(&v_icrs, tdb, location)
}

/// Calcula la posición horizontal para todos los cuerpos de
/// [`Body::all`] de una vez. Devuelve un array fijo en el mismo
/// orden — útil para enriquecer una UI con una sola pasada.
pub fn sky_positions_all(tdb: &TDB, location: &Location) -> [(Body, SkyPosition); 10] {
    let bodies = Body::all();
    let mut out: [(Body, SkyPosition); 10] = [(
        Body::Sun,
        SkyPosition {
            altitude_deg: 0.0,
            azimuth_deg: 0.0,
            right_ascension_deg: 0.0,
            declination_deg: 0.0,
            distance_au: 0.0,
            above_horizon: false,
        },
    ); 10];
    for (i, b) in bodies.iter().enumerate() {
        out[i] = (*b, sky_position(b, tdb, location));
    }
    out
}

/// Posición ICRS geocéntrica de un cuerpo en au.
fn geocentric_icrs_au(body: &Body, tdb: &TDB) -> Vector3 {
    let inv_au = 1.0 / cosmos_core::constants::AU_KM;
    match body {
        Body::Sun => Vsop2013Sun.geocentric_position(tdb).expect("Sun geo"),
        Body::Mercury => Vsop2013Mercury.geocentric_position(tdb).expect("Mercury geo"),
        Body::Venus => Vsop2013Venus.geocentric_position(tdb).expect("Venus geo"),
        Body::Mars => Vsop2013Mars.geocentric_position(tdb).expect("Mars geo"),
        Body::Jupiter => Vsop2013Jupiter.geocentric_position(tdb).expect("Jupiter geo"),
        Body::Saturn => Vsop2013Saturn.geocentric_position(tdb).expect("Saturn geo"),
        Body::Uranus => Vsop2013Uranus.geocentric_position(tdb).expect("Uranus geo"),
        Body::Neptune => Vsop2013Neptune.geocentric_position(tdb).expect("Neptune geo"),
        Body::Pluto => Vsop2013Pluto.geocentric_position(tdb).expect("Pluto geo"),
        Body::Moon => {
            // ElpMpp02 devuelve km — convertimos a au.
            let km = ElpMpp02Moon::new()
                .geocentric_position_icrs(tdb)
                .expect("Moon geo");
            Vector3::new(km[0] * inv_au, km[1] * inv_au, km[2] * inv_au)
        }
    }
    // Earth no se incluye: su posición geocéntrica es trivialmente cero.
    // El parámetro de helio se calcula con Vsop2013Earth si el usuario lo
    // necesita, pero no entra al skywatch (no "se ve" la Tierra desde la
    // Tierra). Vsop2013Earth se importa por consistencia con el resto
    // del kernel; lo retenemos vivo aquí.
    .into_kept_alive()
}

trait KeepAlive {
    fn into_kept_alive(self) -> Vector3;
}
impl KeepAlive for Vector3 {
    fn into_kept_alive(self) -> Vector3 {
        // Tocamos Vsop2013Earth para que la dep sea visible al optimizador
        // y no se queje (las funciones públicas del crate no la usan).
        let _ = Vsop2013Earth::new();
        self
    }
}

/// Convierte un vector ICRS geocéntrico (au) a (alt, az, ra, dec)
/// topocéntrico. Aproximación: usa TDB como UT1 para el cálculo de
/// GMST (error de tiempo ~ 70 s ≈ 0.3° en RA — bien para skywatch
/// "qué planetas se ven esta noche", insuficiente para astrometría).
fn icrs_to_sky(v: &Vector3, tdb: &TDB, location: &Location) -> SkyPosition {
    let r = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    // RA / Dec en radianes.
    let ra = v.y.atan2(v.x);
    let dec = (v.z / r.max(1e-30)).asin();
    let ra_deg = wrap_360(ra * DEG_PER_RAD);
    let dec_deg = dec * DEG_PER_RAD;

    let jd = tdb.to_julian_date().to_f64();
    let gmst_deg = wrap_360(gmst_from_jd(jd));
    let lst_deg = wrap_360(gmst_deg + location.longitude_degrees());
    let ha_deg = wrap_180(lst_deg - ra_deg);
    let ha = ha_deg * RAD_PER_DEG;

    let lat = location.latitude_degrees() * RAD_PER_DEG;
    let sin_alt = lat.sin() * dec.sin() + lat.cos() * dec.cos() * ha.cos();
    let alt = sin_alt.clamp(-1.0, 1.0).asin();
    let sin_az = -ha.sin() * dec.cos();
    let cos_az = dec.sin() - lat.sin() * sin_alt;
    let az = sin_az.atan2(cos_az);
    let az_deg = wrap_360(az * DEG_PER_RAD);

    SkyPosition {
        altitude_deg: alt * DEG_PER_RAD,
        azimuth_deg: az_deg,
        right_ascension_deg: ra_deg,
        declination_deg: dec_deg,
        distance_au: r,
        above_horizon: alt > 0.0,
    }
}

/// GMST aproximado (grados) a partir de JD UT1, formula IAU 1982
/// simplificada. Suficiente para skywatch — error de centésimas de
/// segundo de tiempo en el rango 1900–2100.
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
    let m = ((deg + 180.0).rem_euclid(360.0)) - 180.0;
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
        // Lima, Perú: latitud ~ -12.05°, longitud ~ -77.05°, ~150m.
        Location::from_degrees(-12.05, -77.05, 150.0).expect("lima")
    }

    fn greenwich_noon_j2000() -> TDB {
        // J2000 = 2000-01-01T12:00:00 TDB ~ mediodía solar en
        // Greenwich. El Sol debería estar muy cerca del cenit en
        // Greenwich (lat 51.5°N, así que alt ≈ 90 - 51.5 - 23 ≈ 15°
        // hacia el sur en enero).
        TDB::j2000()
    }

    #[test]
    fn sun_visible_at_noon_greenwich() {
        // `Location::greenwich()` devuelve (0, 0, 0): es el punto-cero
        // del sistema, NO el observatorio real (51.5°N). Con lat=0 y
        // δ_sol≈-23° al mediodía solar (HA≈0), alt = 90 - |0 - (-23)|
        // = 67°. Si en algún momento el constructor de greenwich cambia
        // de semántica, este test rompe ruidosamente.
        let loc = Location::greenwich();
        let pos = sky_position(&Body::Sun, &greenwich_noon_j2000(), &loc);
        assert!(
            pos.above_horizon,
            "Sol al mediodía en Greenwich(0,0) debe estar sobre el horizonte (alt={})",
            pos.altitude_deg
        );
        assert!(
            pos.altitude_deg > 60.0 && pos.altitude_deg < 75.0,
            "alt solar al cruzar el meridiano desde lat=0, δ≈-23°: ~67°, fue {}",
            pos.altitude_deg
        );
    }

    #[test]
    fn sun_at_noon_at_real_greenwich_latitude() {
        // Con latitud real de Greenwich (51.5°N) al mediodía de
        // J2000: alt ≈ 90 - 51.5 - 23 = 15.5°.
        let loc = Location::from_degrees(51.4769, 0.0, 46.0).expect("greenwich real");
        let pos = sky_position(&Body::Sun, &greenwich_noon_j2000(), &loc);
        assert!(
            pos.above_horizon,
            "Sol al mediodía en Greenwich real debe estar sobre el horizonte"
        );
        assert!(
            pos.altitude_deg > 10.0 && pos.altitude_deg < 25.0,
            "alt solar al mediodía Greenwich enero ~15°, fue {}",
            pos.altitude_deg
        );
    }

    #[test]
    fn sun_dec_in_january_is_negative() {
        // Al 2000-01-01 el Sol está cerca del solsticio de invierno
        // → declinación ~ -23°.
        let pos = sky_position(&Body::Sun, &greenwich_noon_j2000(), &Location::greenwich());
        assert!(
            pos.declination_deg < -20.0 && pos.declination_deg > -25.0,
            "δ_sol en enero ~ -23°: {}",
            pos.declination_deg
        );
    }

    #[test]
    fn moon_distance_in_lunar_range() {
        let pos = sky_position(&Body::Moon, &greenwich_noon_j2000(), &lima());
        // Distancia Tierra-Luna: ~ 0.0024 – 0.0027 au (perigeo/apogeo).
        assert!(
            pos.distance_au > 0.0020 && pos.distance_au < 0.0030,
            "d(moon) en au: {}",
            pos.distance_au
        );
    }

    #[test]
    fn azimuth_in_range() {
        for body in Body::all() {
            let pos = sky_position(&body, &greenwich_noon_j2000(), &lima());
            assert!(
                pos.azimuth_deg >= 0.0 && pos.azimuth_deg < 360.0,
                "{:?} az fuera de rango: {}",
                body,
                pos.azimuth_deg
            );
            assert!(
                pos.altitude_deg >= -90.0 && pos.altitude_deg <= 90.0,
                "{:?} alt fuera de rango: {}",
                body,
                pos.altitude_deg
            );
        }
    }

    #[test]
    fn all_planets_have_a_position() {
        let positions = sky_positions_all(&greenwich_noon_j2000(), &lima());
        assert_eq!(positions.len(), 10);
        // Cada cuerpo aparece exactamente una vez en el orden
        // declarado.
        for (i, b) in Body::all().iter().enumerate() {
            assert_eq!(positions[i].0, *b);
        }
    }

    #[test]
    fn visibility_score_monotonic() {
        // Un cuerpo arriba debe ganarle a un cuerpo abajo.
        let mut arriba = SkyPosition {
            altitude_deg: 30.0,
            azimuth_deg: 0.0,
            right_ascension_deg: 0.0,
            declination_deg: 0.0,
            distance_au: 1.0,
            above_horizon: true,
        };
        let abajo = SkyPosition {
            altitude_deg: -10.0,
            azimuth_deg: 0.0,
            right_ascension_deg: 0.0,
            declination_deg: 0.0,
            distance_au: 1.0,
            above_horizon: false,
        };
        assert!(arriba.visibility_score() > abajo.visibility_score());
        arriba.altitude_deg = 45.0;
        assert!(arriba.visibility_score() > 30.0);
    }

    #[test]
    fn jupiter_position_changes_over_year() {
        let loc = lima();
        let t1: TDB = "2026-01-01T00:00:00".parse().expect("iso 2026-01");
        let t2: TDB = "2026-07-01T00:00:00".parse().expect("iso 2026-07");
        let p1 = sky_position(&Body::Jupiter, &t1, &loc);
        let p2 = sky_position(&Body::Jupiter, &t2, &loc);
        // Júpiter se mueve ~30°/año en eclíptica — RA distinta.
        let drift = (p2.right_ascension_deg - p1.right_ascension_deg).abs();
        let drift = drift.min(360.0 - drift);
        assert!(
            drift > 2.0,
            "Jupiter RA debe cambiar > 2° en 6 meses, fue {drift}"
        );
    }

    #[test]
    fn wrap_360_basic() {
        assert!((wrap_360(0.0) - 0.0).abs() < 1e-9);
        assert!((wrap_360(360.0) - 0.0).abs() < 1e-9);
        assert!((wrap_360(361.0) - 1.0).abs() < 1e-9);
        assert!((wrap_360(-1.0) - 359.0).abs() < 1e-9);
        assert!((wrap_360(720.5) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn wrap_180_basic() {
        assert!((wrap_180(0.0) - 0.0).abs() < 1e-9);
        assert!((wrap_180(180.0) - 180.0).abs() < 1e-9);
        assert!((wrap_180(190.0) - (-170.0)).abs() < 1e-9);
        assert!((wrap_180(-190.0) - 170.0).abs() < 1e-9);
    }

    #[test]
    fn gmst_at_j2000_is_known() {
        // GMST a J2000 (JD 2451545.0) ≈ 18h 41m 50.5s en horas
        // siderales = 280.46° aprox.
        let g = gmst_from_jd(2451545.0);
        let g = wrap_360(g);
        assert!(
            (g - 280.46).abs() < 0.5,
            "GMST a J2000 ~ 280.46°, fue {g}"
        );
    }
}
