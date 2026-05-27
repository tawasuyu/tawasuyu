//! `cosmos-rise-set` — instantes de salida, paso meridiano y puesta de
//! Sol, Luna y planetas para una ubicación geográfica dada.
//!
//! Capa fina sobre [`cosmos_skywatch`]: barre la altitud topocéntrica de
//! un cuerpo a lo largo de un día y reporta:
//!
//! - **rise** — la altura del cuerpo cruza el horizonte de abajo hacia
//!   arriba (salida).
//! - **transit** — el cuerpo alcanza su máxima altura del día (paso
//!   meridiano; HA ≈ 0 si está sobre el meridiano superior).
//! - **set** — la altura del cuerpo cruza el horizonte de arriba hacia
//!   abajo (puesta).
//!
//! El horizonte por defecto es **geométrico** (`alt = 0`). Para el Sol
//! conviene usar [`Horizon::SunStandard`] (`-0.833°`, que compensa
//! refracción atmosférica media + radio del disco solar) y los
//! crepúsculos clásicos:
//!
//! - [`Horizon::CivilTwilight`] = `-6°`
//! - [`Horizon::NauticalTwilight`] = `-12°`
//! - [`Horizon::AstronomicalTwilight`] = `-18°`
//!
//! ## Algoritmo
//!
//! 1. Muestreo grueso del día con paso de 10 minutos: para cada paso se
//!    evalúa la altura del cuerpo.
//! 2. Cualquier cambio de signo `alt - horizon` indica un cruce; se
//!    refina con bisección hasta resolución de ~1 s.
//! 3. El paso meridiano es el muestreo con `alt` máxima dentro del día,
//!    refinado por interpolación parabólica entre los 3 puntos vecinos.
//!
//! Cuando el cuerpo no cruza el horizonte (circumpolar o nunca sale),
//! `rise` y `set` son `None`. Los flags `never_rises` / `never_sets`
//! permiten distinguir los dos casos.
//!
//! ## Precisión
//!
//! Heredada de [`cosmos_skywatch`]: aproximación TDB ≈ UT1 — error de
//! tiempo `~70 s` ≈ `~1°` de movimiento de la Luna por hora, < arcmin
//! para los planetas exteriores. Suficiente para apps de astrofoto,
//! status bar de sistema, alarmas de amanecer. Para efemérides
//! navales hay que añadir EOP + refracción local.

#![forbid(unsafe_code)]

use cosmos_core::Location;
use cosmos_skywatch::{sky_position, Body};
use cosmos_time::{JulianDate, TDB};

/// Horizonte de referencia para definir "rise" y "set".
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Horizon {
    /// `alt = 0°` — horizonte geométrico, ignora refracción y radio.
    Geometric,
    /// `alt = -0.833°` — convención estándar para el Sol (radio solar
    /// aparente ~0.267° + refracción al horizonte ~0.566°).
    SunStandard,
    /// `alt = -0.567°` — convención estándar para la Luna (refracción
    /// pero **sin** corregir paralaje topocéntrica — para eso hace
    /// falta una cadena de cosmos-time más completa).
    MoonStandard,
    /// `alt = -6°` — crepúsculo civil.
    CivilTwilight,
    /// `alt = -12°` — crepúsculo náutico.
    NauticalTwilight,
    /// `alt = -18°` — crepúsculo astronómico.
    AstronomicalTwilight,
    /// Valor personalizado, en grados.
    Custom(f64),
}

impl Horizon {
    /// Altitud del horizonte en grados.
    pub fn altitude_deg(&self) -> f64 {
        match self {
            Horizon::Geometric => 0.0,
            Horizon::SunStandard => -0.833,
            Horizon::MoonStandard => -0.567,
            Horizon::CivilTwilight => -6.0,
            Horizon::NauticalTwilight => -12.0,
            Horizon::AstronomicalTwilight => -18.0,
            Horizon::Custom(deg) => *deg,
        }
    }
}

/// Resultado de un cálculo rise/transit/set para un día.
#[derive(Debug, Clone, Copy)]
pub struct RiseTransitSet {
    /// Instante TDB de la salida del cuerpo. `None` si el cuerpo no
    /// cruza el horizonte de abajo hacia arriba durante la ventana.
    pub rise: Option<TDB>,
    /// Instante TDB del paso meridiano (máxima altura del día). Siempre
    /// existe — es el muestreo con altitud máxima dentro de la ventana,
    /// refinado por interpolación parabólica.
    pub transit: TDB,
    /// Altitud (grados) en el paso meridiano. Si es menor que el
    /// horizonte, el cuerpo nunca sale.
    pub transit_altitude_deg: f64,
    /// Instante TDB de la puesta del cuerpo. `None` si el cuerpo no
    /// cruza el horizonte de arriba hacia abajo durante la ventana.
    pub set: Option<TDB>,
    /// `true` si el cuerpo nunca alcanza el horizonte durante la
    /// ventana (transit_altitude < horizon).
    pub never_rises: bool,
    /// `true` si el cuerpo permanece todo el día sobre el horizonte
    /// (circumpolar).
    pub never_sets: bool,
}

/// Calcula rise/transit/set para un cuerpo desde una ubicación durante
/// las 24 h que comienzan en `tdb_start`.
///
/// `tdb_start` típicamente debe ser medianoche local en TDB. Para una
/// ventana de 12 h centrada en una fecha (p. ej. amanecer + atardecer
/// del día), pasar tdb_start = noche anterior.
pub fn rise_transit_set(body: &Body, tdb_start: &TDB, location: &Location, horizon: Horizon) -> RiseTransitSet {
    rise_transit_set_window(body, tdb_start, 1.0, location, horizon)
}

/// Variante con ventana arbitraria en días desde `tdb_start`. Útil para
/// buscar el próximo amanecer en una ventana de 48 h.
pub fn rise_transit_set_window(
    body: &Body,
    tdb_start: &TDB,
    duration_days: f64,
    location: &Location,
    horizon: Horizon,
) -> RiseTransitSet {
    let h_deg = horizon.altitude_deg();
    let jd_start = tdb_start.to_julian_date().to_f64();
    let jd_end = jd_start + duration_days;
    let step_days = SAMPLING_STEP_DAYS;
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity((duration_days / step_days) as usize + 2);

    let mut jd = jd_start;
    while jd <= jd_end + step_days * 0.5 {
        let tdb = TDB::from_julian_date(JulianDate::from_f64(jd));
        let alt = sky_position(body, &tdb, location).altitude_deg;
        samples.push((jd, alt));
        jd += step_days;
    }

    // Cruces de horizonte.
    let mut rise: Option<TDB> = None;
    let mut set: Option<TDB> = None;
    for w in samples.windows(2) {
        let (jd_a, alt_a) = w[0];
        let (jd_b, alt_b) = w[1];
        let da = alt_a - h_deg;
        let db = alt_b - h_deg;
        if da == 0.0 && db == 0.0 {
            continue;
        }
        if da.signum() != db.signum() {
            let jd_cross = bisect_horizon(body, location, h_deg, jd_a, jd_b);
            let t_cross = TDB::from_julian_date(JulianDate::from_f64(jd_cross));
            if da < 0.0 && db > 0.0 {
                if rise.is_none() {
                    rise = Some(t_cross);
                }
            } else if da > 0.0 && db < 0.0 && set.is_none() {
                set = Some(t_cross);
            }
            if rise.is_some() && set.is_some() {
                break;
            }
        }
    }

    // Transit = máxima altura. Búsqueda + refinamiento parabólico.
    let mut max_i: usize = 0;
    let mut max_alt = f64::NEG_INFINITY;
    for (i, (_, alt)) in samples.iter().enumerate() {
        if *alt > max_alt {
            max_alt = *alt;
            max_i = i;
        }
    }
    let (transit_jd, transit_alt) = if max_i > 0 && max_i + 1 < samples.len() {
        parabolic_peak(samples[max_i - 1], samples[max_i], samples[max_i + 1])
    } else {
        samples[max_i]
    };

    let never_rises = transit_alt < h_deg;
    let never_sets = !never_rises && rise.is_none() && set.is_none();

    RiseTransitSet {
        rise,
        transit: TDB::from_julian_date(JulianDate::from_f64(transit_jd)),
        transit_altitude_deg: transit_alt,
        set,
        never_rises,
        never_sets,
    }
}

/// Paso de muestreo grueso para localizar cruces. 10 minutos = compromiso
/// entre robustez (pasos cortos no se saltan eventos cercanos al horizonte
/// del Sol) y costo. La Luna se mueve ~0.5°/h, así que 10 min ≈ 0.08° —
/// suficientemente fino para detectar todos los cruces sin perderlos.
const SAMPLING_STEP_DAYS: f64 = 10.0 / 1440.0;

/// Refina el cruce del horizonte por bisección. `jd_a` y `jd_b` tienen
/// signos opuestos en `alt - horizon`.
fn bisect_horizon(body: &Body, location: &Location, h_deg: f64, jd_a: f64, jd_b: f64) -> f64 {
    let mut lo = jd_a;
    let mut hi = jd_b;
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        let tdb = TDB::from_julian_date(JulianDate::from_f64(mid));
        let alt = sky_position(body, &tdb, location).altitude_deg;
        let f_lo = {
            let tdb_lo = TDB::from_julian_date(JulianDate::from_f64(lo));
            sky_position(body, &tdb_lo, location).altitude_deg - h_deg
        };
        let f_mid = alt - h_deg;
        if (hi - lo).abs() * 86400.0 < 1.0 {
            return mid;
        }
        if f_lo.signum() == f_mid.signum() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Ajusta una parábola por 3 puntos `(jd, alt)` y devuelve el vértice
/// `(jd_peak, alt_peak)`. Usada para refinar el paso meridiano.
///
/// Para evitar la pérdida de precisión cuando `jd ~ 2.46e6` (productos
/// `x²` del orden de `6e12`), centramos en `x1` antes de ajustar.
fn parabolic_peak(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> (f64, f64) {
    let (x0, y0) = a;
    let (x1, y1) = b;
    let (x2, y2) = c;
    let u0 = x0 - x1;
    let u2 = x2 - x1;
    // Sistema: y = α·u² + β·u + y1, eval en u0 y u2:
    //   α·u0² + β·u0 = y0 - y1
    //   α·u2² + β·u2 = y2 - y1
    let det = u0 * u2 * (u0 - u2);
    if det.abs() < 1e-30 {
        return b;
    }
    let alpha = (u2 * (y0 - y1) - u0 * (y2 - y1)) / det;
    let beta = (u0 * u0 * (y2 - y1) - u2 * u2 * (y0 - y1)) / det;
    if alpha >= 0.0 || alpha.abs() < 1e-30 {
        // Sin concavidad negativa, no hay máximo local — devolvemos b.
        return b;
    }
    let u_peak = -beta / (2.0 * alpha);
    if u_peak < u0 || u_peak > u2 {
        // Vértice fuera del bracketing — extrapolación inútil.
        return b;
    }
    let y_peak = alpha * u_peak * u_peak + beta * u_peak + y1;
    (x1 + u_peak, y_peak)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lima() -> Location {
        Location::from_degrees(-12.05, -77.05, 150.0).unwrap()
    }

    fn quito() -> Location {
        Location::from_degrees(0.0, -78.5, 2850.0).unwrap()
    }

    fn longyearbyen() -> Location {
        // Lat 78.2°N — Svalbard. Sol de medianoche entre abril y agosto.
        Location::from_degrees(78.2, 15.6, 50.0).unwrap()
    }

    fn midnight_tdb(year: i32, month: u8, day: u8) -> TDB {
        TDB::from_julian_date(JulianDate::from_calendar(year, month, day, 0, 0, 0.0))
    }

    #[test]
    fn horizon_values_match_convention() {
        assert!((Horizon::Geometric.altitude_deg() - 0.0).abs() < 1e-9);
        assert!((Horizon::SunStandard.altitude_deg() - (-0.833)).abs() < 1e-9);
        assert!((Horizon::CivilTwilight.altitude_deg() - (-6.0)).abs() < 1e-9);
        assert!((Horizon::NauticalTwilight.altitude_deg() - (-12.0)).abs() < 1e-9);
        assert!((Horizon::AstronomicalTwilight.altitude_deg() - (-18.0)).abs() < 1e-9);
        assert!((Horizon::Custom(2.5).altitude_deg() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn sun_rises_and_sets_in_lima_today() {
        let t0 = midnight_tdb(2026, 5, 27);
        let r = rise_transit_set(&Body::Sun, &t0, &lima(), Horizon::SunStandard);
        assert!(r.rise.is_some(), "Sol debe salir hoy en Lima");
        assert!(r.set.is_some(), "Sol debe ponerse hoy en Lima");
        assert!(
            !r.never_rises && !r.never_sets,
            "Lima nunca tiene Sol circumpolar ni 24h de noche"
        );
        // Transit positivo y plausible (Lima en mayo: Sol al mediodía ~50°).
        assert!(
            r.transit_altitude_deg > 30.0 && r.transit_altitude_deg < 80.0,
            "transit alt plausible para Lima mayo: {}",
            r.transit_altitude_deg
        );
    }

    #[test]
    fn rise_before_transit_before_set() {
        let t0 = midnight_tdb(2026, 5, 27);
        let r = rise_transit_set(&Body::Sun, &t0, &lima(), Horizon::SunStandard);
        let rise = r.rise.unwrap().to_julian_date().to_f64();
        let transit = r.transit.to_julian_date().to_f64();
        let set = r.set.unwrap().to_julian_date().to_f64();
        assert!(rise < transit, "rise antes que transit");
        assert!(transit < set, "transit antes que set");
    }

    #[test]
    fn solar_day_length_in_lima_about_12h() {
        // Lima cerca del ecuador: el día solar dura ~ 11h45m (en mayo
        // un poco menos que 12 h por estar en hemisferio sur, otoño).
        let t0 = midnight_tdb(2026, 5, 27);
        let r = rise_transit_set(&Body::Sun, &t0, &lima(), Horizon::SunStandard);
        let dt_h =
            (r.set.unwrap().to_julian_date().to_f64() - r.rise.unwrap().to_julian_date().to_f64())
                * 24.0;
        assert!(
            dt_h > 10.0 && dt_h < 13.0,
            "día solar Lima en mayo entre 10 y 13 h: {dt_h}"
        );
    }

    #[test]
    fn quito_equinox_day_is_exactly_12h() {
        // Quito en equinoccio (lat ≈ 0°): día y noche iguales = 12 h.
        let t0 = midnight_tdb(2026, 3, 20);
        let r = rise_transit_set(&Body::Sun, &t0, &quito(), Horizon::SunStandard);
        let dt_h =
            (r.set.unwrap().to_julian_date().to_f64() - r.rise.unwrap().to_julian_date().to_f64())
                * 24.0;
        // Aceptamos 12 h ± 15 min (refracción + alt -0.833 inflan ligeramente
        // el día visible vs el geométrico).
        assert!(
            (dt_h - 12.0).abs() < 0.25,
            "día solar Quito equinoccio ~12h, fue {dt_h}"
        );
    }

    #[test]
    fn longyearbyen_midnight_sun_in_june() {
        // Svalbard: del 19 abr al 23 ago, Sol de medianoche. El 21 jun
        // el Sol nunca se pone → rise = None, set = None, never_sets =
        // true.
        let t0 = midnight_tdb(2026, 6, 21);
        let r = rise_transit_set(&Body::Sun, &t0, &longyearbyen(), Horizon::SunStandard);
        assert!(
            r.never_sets,
            "Svalbard 21 jun: never_sets debe ser true. transit_alt={}",
            r.transit_altitude_deg
        );
        assert!(r.rise.is_none(), "rise debe ser None en Sol de medianoche");
        assert!(r.set.is_none(), "set debe ser None en Sol de medianoche");
        assert!(
            r.transit_altitude_deg > 0.0,
            "transit alt > 0: {}",
            r.transit_altitude_deg
        );
    }

    #[test]
    fn longyearbyen_polar_night_in_december() {
        // Polar night: del 11 nov al 30 ene en Longyearbyen. El 21 dic
        // el Sol nunca sale → never_rises = true.
        let t0 = midnight_tdb(2026, 12, 21);
        let r = rise_transit_set(&Body::Sun, &t0, &longyearbyen(), Horizon::SunStandard);
        assert!(
            r.never_rises,
            "Svalbard 21 dic: never_rises debe ser true. transit_alt={}",
            r.transit_altitude_deg
        );
        assert!(r.rise.is_none());
        assert!(r.set.is_none());
    }

    #[test]
    fn civil_twilight_starts_before_sunrise() {
        // El crepúsculo civil empieza antes (sol más bajo). Por tanto
        // el "rise" con horizonte -6° llega antes que con -0.833°.
        let t0 = midnight_tdb(2026, 5, 27);
        let sun = rise_transit_set(&Body::Sun, &t0, &lima(), Horizon::SunStandard);
        let civil = rise_transit_set(&Body::Sun, &t0, &lima(), Horizon::CivilTwilight);
        let sun_rise = sun.rise.unwrap().to_julian_date().to_f64();
        let civil_rise = civil.rise.unwrap().to_julian_date().to_f64();
        assert!(
            civil_rise < sun_rise,
            "crepúsculo civil empieza antes del amanecer: civil={civil_rise}, sun={sun_rise}"
        );
        // La diferencia debe ser del orden de 20-40 min.
        let dt_min = (sun_rise - civil_rise) * 1440.0;
        assert!(
            dt_min > 10.0 && dt_min < 60.0,
            "delta crepúsculo civil → amanecer típico 20-40 min, fue {dt_min}"
        );
    }

    #[test]
    fn nautical_before_civil_before_astronomical() {
        let t0 = midnight_tdb(2026, 5, 27);
        let civil = rise_transit_set(&Body::Sun, &t0, &lima(), Horizon::CivilTwilight)
            .rise
            .unwrap()
            .to_julian_date()
            .to_f64();
        let nautical = rise_transit_set(&Body::Sun, &t0, &lima(), Horizon::NauticalTwilight)
            .rise
            .unwrap()
            .to_julian_date()
            .to_f64();
        let astro = rise_transit_set(&Body::Sun, &t0, &lima(), Horizon::AstronomicalTwilight)
            .rise
            .unwrap()
            .to_julian_date()
            .to_f64();
        // El astronómico es el más temprano (Sol más bajo bajo el horizonte).
        assert!(
            astro < nautical && nautical < civil,
            "orden de crepúsculos por la mañana: astro < nautical < civil. \
             astro={astro} naut={nautical} civil={civil}"
        );
    }

    #[test]
    fn moon_rise_set_present_in_lima() {
        // La Luna se mueve ~12°/día — algún día debe salir/ponerse en
        // Lima. Probamos una ventana de 48 h para asegurar al menos un
        // par rise+set.
        let t0 = midnight_tdb(2026, 5, 27);
        let r = rise_transit_set_window(
            &Body::Moon,
            &t0,
            2.0,
            &lima(),
            Horizon::MoonStandard,
        );
        assert!(
            r.rise.is_some() || r.set.is_some(),
            "en 48h la Luna salió o se puso en Lima"
        );
    }

    #[test]
    fn jupiter_transit_altitude_in_range() {
        // Júpiter desde Quito (lat 0): transit alt ~ 90 - |δ|. Como
        // δ_jupiter < 25°, la transit alt debe estar entre 65° y 90°.
        let t0 = midnight_tdb(2026, 5, 27);
        let r = rise_transit_set_window(
            &Body::Jupiter,
            &t0,
            1.0,
            &quito(),
            Horizon::Geometric,
        );
        if !r.never_rises {
            assert!(
                r.transit_altitude_deg > 60.0,
                "Júpiter desde Quito: transit > 60°, fue {}",
                r.transit_altitude_deg
            );
        }
    }

    #[test]
    fn parabolic_peak_centered_triangle() {
        // (-1, 0), (0, 1), (1, 0): vértice en (0, 1).
        let (xp, yp) = parabolic_peak((-1.0, 0.0), (0.0, 1.0), (1.0, 0.0));
        assert!(xp.abs() < 1e-9, "vertice x={xp}");
        assert!((yp - 1.0).abs() < 1e-9, "vertice y={yp}");
    }

    #[test]
    fn parabolic_peak_offcenter() {
        // y = -(x-2)² + 5 → vertice en (2, 5).
        // En x=0: -(0-2)²+5 = 1. En x=1: -(1-2)²+5 = 4. En x=3: -(3-2)²+5=4.
        let (xp, yp) = parabolic_peak((0.0, 1.0), (1.0, 4.0), (3.0, 4.0));
        assert!((xp - 2.0).abs() < 1e-6, "vertice x debería ser 2, fue {xp}");
        assert!((yp - 5.0).abs() < 1e-6, "vertice y debería ser 5, fue {yp}");
    }
}
