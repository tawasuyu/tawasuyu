//! `cosmos-notebook-kernel` — kernel de notebook que envuelve
//! [`cosmos_time`] + [`cosmos_ephemeris`] para servir efemérides puras
//! desde el DAG.
//!
//! No construye `Chart`s ni interpreta astrología — esto es el ejes
//! "cosmos-ephem puro" del refactor sugerido (separar la base
//! astrométrica de la interpretación). Sirve a skywatch / sundial /
//! mareas / navegación astronómica / cualquier dominio que necesite
//! posiciones de cuerpos del sistema solar en un instante dado.
//!
//! ## Lenguajes reconocidos
//!
//! Base (efemérides puras):
//!
//! | `language`                  | Source                          | Efecto                                                        |
//! |-----------------------------|---------------------------------|---------------------------------------------------------------|
//! | `cosmos-tdb`                | ISO 8601 (ej. `2026-05-27T00:00:00`) o `j2000` | Fija el instante TDB compartido del kernel.    |
//! | `cosmos-location`           | `"LAT LON ALT_M"` (deg, deg, m)  | Fija la ubicación compartida (default = Greenwich 0,0,0).     |
//! | `cosmos-positions`          | (vacío) o lista de cuerpos       | Tabla de posiciones geocéntricas ICRS (x,y,z en au) al TDB.   |
//! | `cosmos-helio`              | (vacío) o lista de cuerpos       | Tabla de posiciones heliocéntricas (incluye Tierra).          |
//! | `cosmos-distance`           | `"BODY"`                         | Distancia geocéntrica al cuerpo en au, output Scalar.         |
//!
//! Extractos (encima de cosmos-skywatch/sundial/tides/rise-set/eclipses/transits):
//!
//! | `language`                  | Source                          | Efecto                                                        |
//! |-----------------------------|---------------------------------|---------------------------------------------------------------|
//! | `cosmos-skywatch`           | (vacío) o lista de cuerpos       | Tabla alt/az/RA/dec/dist desde la Location al TDB.            |
//! | `cosmos-sundial`            | (vacío)                          | Lectura del cuadrante solar: HA, sombra azimut + ratio.       |
//! | `cosmos-tides`              | (vacío)                          | Altura de marea equilibrio Sol+Luna al TDB+Location, en m.    |
//! | `cosmos-rise-set`           | (vacío) o lista de cuerpos       | Tabla rise/transit/set para el día del TDB.                   |
//! | `cosmos-eclipses`           | `"YEARS [solar|lunar]"`          | Eclipses geocéntricos en ventana (default `4 solar`).         |
//! | `cosmos-transits`           | `"YEARS"`                        | Tránsitos de Mercurio/Venus sobre el Sol en ventana.          |
//!
//! Cuerpos reconocidos: `sun`, `moon`, `mercury`, `venus`, `earth`,
//! `mars`, `jupiter`, `saturn`, `uranus`, `neptune`, `pluto`. Sin
//! source en `positions`/`helio` se devuelven todos los cuerpos
//! geocéntricamente válidos.
//!
//! ## Encaje con el DAG
//!
//! - Una celda `cosmos-tdb "2026-05-27T00:00:00"` fija el reloj.
//! - Celdas dependientes `cosmos-positions`, `cosmos-helio`,
//!   `cosmos-distance "mars"` leen ese reloj y producen tablas.
//! - Editar la primera y `run_from` re-cocina toda la cadena con el
//!   nuevo instante. Mismo patrón que el kernel-dominium.

#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cosmos_core::{Location, Vector3};
use cosmos_eclipses::{find_lunar_eclipses, find_solar_eclipses};
use cosmos_ephemeris::moon::ElpMpp02Moon;
use cosmos_ephemeris::planets::{
    Vsop2013Jupiter, Vsop2013Mars, Vsop2013Mercury, Vsop2013Neptune, Vsop2013Pluto,
    Vsop2013Saturn, Vsop2013Uranus, Vsop2013Venus,
};
use cosmos_ephemeris::sun::Vsop2013Sun;
use cosmos_ephemeris::earth::Vsop2013Earth;
use cosmos_rise_set::{rise_transit_set_window, Horizon};
use cosmos_skywatch::{sky_position as sky_pos, Body as SkyBody};
use cosmos_sundial::sundial_reading;
use cosmos_tides::tide_reading;
use cosmos_time::TDB;
use cosmos_transits::{find_transits, InnerPlanet};
use pluma_notebook_core::{CellOutput, OutputPayload};
use pluma_notebook_exec::{Kernel, KernelError, KernelOutput};

/// Estado vivo del kernel cosmos: el instante TDB compartido + la
/// ubicación de observación. Default: J2000 en Greenwich (0,0,0).
#[derive(Debug, Clone, Copy)]
pub struct CosmosState {
    pub tdb: TDB,
    pub location: Location,
}

impl Default for CosmosState {
    fn default() -> Self {
        Self {
            tdb: TDB::j2000(),
            location: Location::greenwich(),
        }
    }
}

pub struct CosmosKernel {
    state: Arc<Mutex<CosmosState>>,
}

impl Default for CosmosKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl CosmosKernel {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(CosmosState::default())),
        }
    }

    pub fn state_handle(&self) -> Arc<Mutex<CosmosState>> {
        Arc::clone(&self.state)
    }

    pub fn snapshot(&self) -> CosmosState {
        *self.state.lock().expect("kernel state envenenado")
    }
}

#[async_trait]
impl Kernel for CosmosKernel {
    async fn execute(
        &self,
        source: &str,
        language: &str,
    ) -> Result<KernelOutput, KernelError> {
        match language {
            "cosmos-tdb" => exec_tdb(source, &self.state),
            "cosmos-location" => exec_location(source, &self.state),
            "cosmos-positions" => exec_positions(source, &self.state, Frame::Geocentric),
            "cosmos-helio" => exec_positions(source, &self.state, Frame::Heliocentric),
            "cosmos-distance" => exec_distance(source, &self.state),
            "cosmos-skywatch" => exec_skywatch(source, &self.state),
            "cosmos-sundial" => exec_sundial(&self.state),
            "cosmos-tides" => exec_tides(&self.state),
            "cosmos-rise-set" => exec_rise_set(source, &self.state),
            "cosmos-eclipses" => exec_eclipses(source, &self.state),
            "cosmos-transits" => exec_transits(source, &self.state),
            other => Err(KernelError::Runtime(format!(
                "lenguaje no reconocido por el kernel cosmos: '{other}' \
                 (esperaba: cosmos-tdb | cosmos-location | cosmos-positions | \
                 cosmos-helio | cosmos-distance | cosmos-skywatch | \
                 cosmos-sundial | cosmos-tides | cosmos-rise-set | \
                 cosmos-eclipses | cosmos-transits)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    Geocentric,
    Heliocentric,
}

fn exec_tdb(
    source: &str,
    state: &Arc<Mutex<CosmosState>>,
) -> Result<KernelOutput, KernelError> {
    let raw = source.trim();
    let tdb = if raw.is_empty() || raw.eq_ignore_ascii_case("j2000") {
        TDB::j2000()
    } else {
        raw.parse::<TDB>().map_err(|e| {
            KernelError::Runtime(format!(
                "fecha TDB inválida '{raw}': {e:?} (esperaba ISO 8601 ej. 2026-05-27T00:00:00 o 'j2000')"
            ))
        })?
    };
    let mut s = lock(state)?;
    s.tdb = tdb;
    let jd = tdb.to_julian_date().to_f64();
    Ok(text_output(format!("TDB fijado a {raw:?} (JD={jd:.6})")))
}

fn exec_location(
    source: &str,
    state: &Arc<Mutex<CosmosState>>,
) -> Result<KernelOutput, KernelError> {
    let raw = source.trim();
    let parts: Vec<&str> = raw.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(KernelError::Runtime(
            "cosmos-location requiere tres valores: LAT_DEG LON_DEG ALT_M".into(),
        ));
    }
    let lat: f64 = parts[0]
        .parse()
        .map_err(|e| KernelError::Runtime(format!("LAT inválida: {e}")))?;
    let lon: f64 = parts[1]
        .parse()
        .map_err(|e| KernelError::Runtime(format!("LON inválida: {e}")))?;
    let alt: f64 = parts[2]
        .parse()
        .map_err(|e| KernelError::Runtime(format!("ALT_M inválida: {e}")))?;
    let loc = Location::from_degrees(lat, lon, alt)
        .map_err(|e| KernelError::Runtime(format!("Location inválida: {e:?}")))?;
    let mut s = lock(state)?;
    s.location = loc;
    Ok(text_output(format!(
        "Location fijada a ({lat:.4}°, {lon:.4}°, {alt:.0} m)"
    )))
}

fn exec_skywatch(
    source: &str,
    state: &Arc<Mutex<CosmosState>>,
) -> Result<KernelOutput, KernelError> {
    let snap = *lock(state)?;
    let bodies = parse_sky_bodies(source)?;
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(bodies.len());
    for b in &bodies {
        let p = sky_pos(b, &snap.tdb, &snap.location);
        rows.push(vec![
            b.canonical().to_string(),
            format!("{:.3}", p.altitude_deg),
            format!("{:.3}", p.azimuth_deg),
            format!("{:.4}", p.right_ascension_deg),
            format!("{:.4}", p.declination_deg),
            format!("{:.6}", p.distance_au),
            if p.above_horizon { "sí" } else { "no" }.to_string(),
        ]);
    }
    let columns = vec![
        "body".into(),
        "alt_deg".into(),
        "az_deg".into(),
        "ra_deg".into(),
        "dec_deg".into(),
        "r_au".into(),
        "visible".into(),
    ];
    let stdout = format_table(&columns, &rows);
    Ok(CellOutput {
        stdout,
        value: Some(rows.len().to_string()),
        payload: OutputPayload::Table { columns, rows },
    })
}

fn exec_sundial(state: &Arc<Mutex<CosmosState>>) -> Result<KernelOutput, KernelError> {
    let snap = *lock(state)?;
    let r = sundial_reading(&snap.tdb, &snap.location);
    let columns = vec![
        "sun_alt_deg".into(),
        "sun_az_deg".into(),
        "hour_angle_deg".into(),
        "shadow_az_deg".into(),
        "shadow_length_ratio".into(),
    ];
    let row = vec![
        format!("{:.3}", r.sun.altitude_deg),
        format!("{:.3}", r.sun.azimuth_deg),
        format!("{:.3}", r.hour_angle_deg),
        r.shadow_azimuth_deg
            .map(|v| format!("{v:.3}"))
            .unwrap_or_else(|| "—".into()),
        r.shadow_length_ratio
            .map(|v| format!("{v:.4}"))
            .unwrap_or_else(|| "—".into()),
    ];
    let stdout = format_table(&columns, std::slice::from_ref(&row));
    Ok(CellOutput {
        stdout,
        value: r.shadow_length_ratio.map(|v| format!("{v:.4}")),
        payload: OutputPayload::Table { columns, rows: vec![row] },
    })
}

fn exec_tides(state: &Arc<Mutex<CosmosState>>) -> Result<KernelOutput, KernelError> {
    let snap = *lock(state)?;
    let r = tide_reading(&snap.tdb, &snap.location);
    let columns = vec![
        "componente".into(),
        "height_m".into(),
        "zenith_deg".into(),
    ];
    let rows = vec![
        vec![
            "lunar".into(),
            format!("{:.4}", r.lunar.height_m),
            format!("{:.2}", r.lunar.zenith_deg),
        ],
        vec![
            "solar".into(),
            format!("{:.4}", r.solar.height_m),
            format!("{:.2}", r.solar.zenith_deg),
        ],
        vec![
            "total".into(),
            format!("{:.4}", r.total_height_m),
            "—".into(),
        ],
    ];
    let stdout = format_table(&columns, &rows);
    Ok(CellOutput {
        stdout,
        value: Some(format!("{:.4}", r.total_height_m)),
        payload: OutputPayload::Table { columns, rows },
    })
}

fn exec_rise_set(
    source: &str,
    state: &Arc<Mutex<CosmosState>>,
) -> Result<KernelOutput, KernelError> {
    let snap = *lock(state)?;
    let bodies = parse_sky_bodies(source)?;
    let columns = vec![
        "body".into(),
        "rise_jd".into(),
        "transit_jd".into(),
        "set_jd".into(),
        "transit_alt_deg".into(),
        "estado".into(),
    ];
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(bodies.len());
    for b in &bodies {
        // Horizonte estándar: SunStandard para Sol, MoonStandard para
        // Luna, Geometric para el resto.
        let horizon = match b {
            SkyBody::Sun => Horizon::SunStandard,
            SkyBody::Moon => Horizon::MoonStandard,
            _ => Horizon::Geometric,
        };
        let r = rise_transit_set_window(b, &snap.tdb, 1.0, &snap.location, horizon);
        let estado = if r.never_rises {
            "no sale"
        } else if r.never_sets {
            "circumpolar"
        } else {
            "sale"
        };
        rows.push(vec![
            b.canonical().to_string(),
            r.rise
                .map(|t| format!("{:.6}", t.to_julian_date().to_f64()))
                .unwrap_or_else(|| "—".into()),
            format!("{:.6}", r.transit.to_julian_date().to_f64()),
            r.set
                .map(|t| format!("{:.6}", t.to_julian_date().to_f64()))
                .unwrap_or_else(|| "—".into()),
            format!("{:.2}", r.transit_altitude_deg),
            estado.into(),
        ]);
    }
    let stdout = format_table(&columns, &rows);
    Ok(CellOutput {
        stdout,
        value: Some(rows.len().to_string()),
        payload: OutputPayload::Table { columns, rows },
    })
}

fn exec_eclipses(
    source: &str,
    state: &Arc<Mutex<CosmosState>>,
) -> Result<KernelOutput, KernelError> {
    let snap = *lock(state)?;
    let (years, kind) = parse_eclipses_args(source)?;
    let jd_from = snap.tdb.to_julian_date().to_f64();
    let jd_to = jd_from + years as f64 * 365.25;
    let step = 1.0 / 24.0; // 1h
    let events = match kind {
        EclipseKind::Solar => find_solar_eclipses(jd_from, jd_to, step),
        EclipseKind::Lunar => find_lunar_eclipses(jd_from, jd_to, step),
    };
    let columns = vec![
        "tipo".into(),
        "jd_mid".into(),
        "magnitud".into(),
        "duracion_h".into(),
    ];
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(events.len());
    for ev in &events {
        let label = match kind {
            EclipseKind::Solar => format!("{:?}", ev.kind_max_solar.unwrap()),
            EclipseKind::Lunar => format!("{:?}", ev.kind_max_lunar.unwrap()),
        };
        rows.push(vec![
            label,
            format!("{:.4}", ev.jd_mid),
            format!("{:.3}", ev.magnitude_max),
            format!("{:.1}", ev.duration_hours),
        ]);
    }
    let stdout = format_table(&columns, &rows);
    Ok(CellOutput {
        stdout,
        value: Some(events.len().to_string()),
        payload: OutputPayload::Table { columns, rows },
    })
}

fn exec_transits(
    source: &str,
    state: &Arc<Mutex<CosmosState>>,
) -> Result<KernelOutput, KernelError> {
    let snap = *lock(state)?;
    let years: u32 = source
        .trim()
        .parse()
        .unwrap_or(15);
    let jd_from = snap.tdb.to_julian_date().to_f64();
    let jd_to = jd_from + years as f64 * 365.25;
    let step = 1.0 / 24.0;
    let mercury = find_transits(&InnerPlanet::Mercury, jd_from, jd_to, step);
    let venus = find_transits(&InnerPlanet::Venus, jd_from, jd_to, step);
    let columns = vec![
        "body".into(),
        "jd_mid".into(),
        "sep_min_deg".into(),
        "duracion_h".into(),
    ];
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(mercury.len() + venus.len());
    for ev in mercury.iter().chain(venus.iter()) {
        rows.push(vec![
            ev.body.canonical().to_string(),
            format!("{:.4}", ev.jd_mid),
            format!("{:.6}", ev.min_separation_deg),
            format!("{:.2}", ev.duration_hours),
        ]);
    }
    let stdout = format_table(&columns, &rows);
    Ok(CellOutput {
        stdout,
        value: Some(rows.len().to_string()),
        payload: OutputPayload::Table { columns, rows },
    })
}

#[derive(Debug, Clone, Copy)]
enum EclipseKind {
    Solar,
    Lunar,
}

fn parse_eclipses_args(source: &str) -> Result<(u32, EclipseKind), KernelError> {
    let raw = source.trim();
    if raw.is_empty() {
        return Ok((4, EclipseKind::Solar));
    }
    let parts: Vec<&str> = raw.split_whitespace().collect();
    let years: u32 = parts[0]
        .parse()
        .map_err(|e| KernelError::Runtime(format!("años inválidos '{}': {e}", parts[0])))?;
    let kind = if parts.len() < 2 {
        EclipseKind::Solar
    } else {
        match parts[1].to_ascii_lowercase().as_str() {
            "solar" | "sun" | "sol" => EclipseKind::Solar,
            "lunar" | "moon" | "luna" => EclipseKind::Lunar,
            other => {
                return Err(KernelError::Runtime(format!(
                    "tipo de eclipse no reconocido: '{other}' (esperaba 'solar' o 'lunar')"
                )));
            }
        }
    };
    Ok((years, kind))
}

fn parse_sky_bodies(source: &str) -> Result<Vec<SkyBody>, KernelError> {
    let raw = source.trim();
    if raw.is_empty() {
        return Ok(SkyBody::all().to_vec());
    }
    raw.split_whitespace()
        .map(|s| match s.to_ascii_lowercase().as_str() {
            "sun" | "sol" => Ok(SkyBody::Sun),
            "moon" | "luna" => Ok(SkyBody::Moon),
            "mercury" | "mercurio" => Ok(SkyBody::Mercury),
            "venus" => Ok(SkyBody::Venus),
            "mars" | "marte" => Ok(SkyBody::Mars),
            "jupiter" | "júpiter" => Ok(SkyBody::Jupiter),
            "saturn" | "saturno" => Ok(SkyBody::Saturn),
            "uranus" | "urano" => Ok(SkyBody::Uranus),
            "neptune" | "neptuno" => Ok(SkyBody::Neptune),
            "pluto" | "plutón" | "pluton" => Ok(SkyBody::Pluto),
            other => Err(KernelError::Runtime(format!(
                "cuerpo skywatch no reconocido: '{other}'"
            ))),
        })
        .collect()
}

fn exec_positions(
    source: &str,
    state: &Arc<Mutex<CosmosState>>,
    frame: Frame,
) -> Result<KernelOutput, KernelError> {
    let tdb = lock(state)?.tdb;
    let bodies = parse_bodies(source, frame)?;
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(bodies.len());
    for body in &bodies {
        let v = position_of(body, frame, &tdb).map_err(|e| {
            KernelError::Runtime(format!(
                "fallo calculando {body:?} ({:?}): {e}",
                frame
            ))
        })?;
        rows.push(vec![
            body.canonical().to_string(),
            format!("{:.10}", v.x),
            format!("{:.10}", v.y),
            format!("{:.10}", v.z),
            format!("{:.10}", (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()),
        ]);
    }
    let columns = vec![
        "body".to_string(),
        "x_au".to_string(),
        "y_au".to_string(),
        "z_au".to_string(),
        "r_au".to_string(),
    ];
    let stdout = format_table(&columns, &rows);
    Ok(CellOutput {
        stdout,
        value: Some(rows.len().to_string()),
        payload: OutputPayload::Table { columns, rows },
    })
}

fn exec_distance(
    source: &str,
    state: &Arc<Mutex<CosmosState>>,
) -> Result<KernelOutput, KernelError> {
    let name = source.trim();
    if name.is_empty() {
        return Err(KernelError::Runtime(
            "cosmos-distance requiere el nombre del cuerpo (ej. 'mars')".into(),
        ));
    }
    let body = Body::parse(name)?;
    let tdb = lock(state)?.tdb;
    let v = position_of(&body, Frame::Geocentric, &tdb).map_err(|e| {
        KernelError::Runtime(format!("fallo calculando {body:?}: {e}"))
    })?;
    let r = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    Ok(CellOutput {
        stdout: format!("d_geo({}) = {:.10} au", body.canonical(), r),
        value: Some(format!("{r:.10}")),
        payload: OutputPayload::Scalar(r),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body {
    Sun,
    Moon,
    Mercury,
    Venus,
    Earth,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
    Pluto,
}

impl Body {
    fn canonical(&self) -> &'static str {
        match self {
            Body::Sun => "sun",
            Body::Moon => "moon",
            Body::Mercury => "mercury",
            Body::Venus => "venus",
            Body::Earth => "earth",
            Body::Mars => "mars",
            Body::Jupiter => "jupiter",
            Body::Saturn => "saturn",
            Body::Uranus => "uranus",
            Body::Neptune => "neptune",
            Body::Pluto => "pluto",
        }
    }

    fn parse(s: &str) -> Result<Self, KernelError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "sun" | "sol" => Ok(Body::Sun),
            "moon" | "luna" => Ok(Body::Moon),
            "mercury" | "mercurio" => Ok(Body::Mercury),
            "venus" => Ok(Body::Venus),
            "earth" | "tierra" => Ok(Body::Earth),
            "mars" | "marte" => Ok(Body::Mars),
            "jupiter" | "júpiter" => Ok(Body::Jupiter),
            "saturn" | "saturno" => Ok(Body::Saturn),
            "uranus" | "urano" => Ok(Body::Uranus),
            "neptune" | "neptuno" => Ok(Body::Neptune),
            "pluto" | "plutón" | "pluton" => Ok(Body::Pluto),
            other => Err(KernelError::Runtime(format!(
                "cuerpo no reconocido: '{other}' (válidos: sun, moon, mercury, venus, earth, mars, jupiter, saturn, uranus, neptune, pluto)"
            ))),
        }
    }
}

fn parse_bodies(source: &str, frame: Frame) -> Result<Vec<Body>, KernelError> {
    let raw = source.trim();
    if raw.is_empty() {
        // Default: todos. En heliocentric incluye Tierra; en geocentric
        // la omitimos (su posición geocéntrica es trivialmente cero, no
        // aporta).
        let mut all = vec![Body::Sun, Body::Moon, Body::Mercury, Body::Venus];
        if frame == Frame::Heliocentric {
            all.push(Body::Earth);
        }
        all.extend([
            Body::Mars,
            Body::Jupiter,
            Body::Saturn,
            Body::Uranus,
            Body::Neptune,
            Body::Pluto,
        ]);
        return Ok(all);
    }
    raw.split_whitespace().map(Body::parse).collect()
}

fn position_of(body: &Body, frame: Frame, tdb: &TDB) -> Result<Vector3, String> {
    let err = |e: cosmos_core::errors::AstroError| format!("{e:?}");
    match (frame, body) {
        // === Heliocentric ===
        (Frame::Heliocentric, Body::Sun) => Ok(Vector3::zeros()),
        (Frame::Heliocentric, Body::Earth) => {
            Vsop2013Earth::new().heliocentric_position(tdb).map_err(err)
        }
        (Frame::Heliocentric, Body::Moon) => {
            // No hay una helio puro de Moon en este crate (Moon es
            // geocéntrica por construcción). Devolvemos
            // earth_helio + moon_geo en ICRS para mantener una semántica
            // razonable cuando alguien pide "helio + moon". La Moon
            // viene en km; convertimos a au.
            let earth = Vsop2013Earth::new().heliocentric_position(tdb).map_err(err)?;
            let m_geo_km = ElpMpp02Moon::new()
                .geocentric_position_icrs(tdb)
                .map_err(err)?;
            let inv_au = 1.0 / cosmos_core::constants::AU_KM;
            Ok(Vector3::new(
                earth.x + m_geo_km[0] * inv_au,
                earth.y + m_geo_km[1] * inv_au,
                earth.z + m_geo_km[2] * inv_au,
            ))
        }
        (Frame::Heliocentric, Body::Mercury) => {
            Vsop2013Mercury.heliocentric_position(tdb).map_err(err)
        }
        (Frame::Heliocentric, Body::Venus) => {
            Vsop2013Venus.heliocentric_position(tdb).map_err(err)
        }
        (Frame::Heliocentric, Body::Mars) => {
            Vsop2013Mars.heliocentric_position(tdb).map_err(err)
        }
        (Frame::Heliocentric, Body::Jupiter) => {
            Vsop2013Jupiter.heliocentric_position(tdb).map_err(err)
        }
        (Frame::Heliocentric, Body::Saturn) => {
            Vsop2013Saturn.heliocentric_position(tdb).map_err(err)
        }
        (Frame::Heliocentric, Body::Uranus) => {
            Vsop2013Uranus.heliocentric_position(tdb).map_err(err)
        }
        (Frame::Heliocentric, Body::Neptune) => {
            Vsop2013Neptune.heliocentric_position(tdb).map_err(err)
        }
        (Frame::Heliocentric, Body::Pluto) => {
            Vsop2013Pluto.heliocentric_position(tdb).map_err(err)
        }
        // === Geocentric ===
        (Frame::Geocentric, Body::Earth) => Ok(Vector3::zeros()),
        (Frame::Geocentric, Body::Sun) => {
            Vsop2013Sun.geocentric_position(tdb).map_err(err)
        }
        (Frame::Geocentric, Body::Moon) => {
            // ElpMpp02 devuelve km; convertimos a au para unidad
            // homogénea con los planetas VSOP2013.
            let v = ElpMpp02Moon::new()
                .geocentric_position_icrs(tdb)
                .map_err(err)?;
            let inv_au = 1.0 / cosmos_core::constants::AU_KM;
            Ok(Vector3::new(v[0] * inv_au, v[1] * inv_au, v[2] * inv_au))
        }
        (Frame::Geocentric, Body::Mercury) => {
            Vsop2013Mercury.geocentric_position(tdb).map_err(err)
        }
        (Frame::Geocentric, Body::Venus) => {
            Vsop2013Venus.geocentric_position(tdb).map_err(err)
        }
        (Frame::Geocentric, Body::Mars) => {
            Vsop2013Mars.geocentric_position(tdb).map_err(err)
        }
        (Frame::Geocentric, Body::Jupiter) => {
            Vsop2013Jupiter.geocentric_position(tdb).map_err(err)
        }
        (Frame::Geocentric, Body::Saturn) => {
            Vsop2013Saturn.geocentric_position(tdb).map_err(err)
        }
        (Frame::Geocentric, Body::Uranus) => {
            Vsop2013Uranus.geocentric_position(tdb).map_err(err)
        }
        (Frame::Geocentric, Body::Neptune) => {
            Vsop2013Neptune.geocentric_position(tdb).map_err(err)
        }
        (Frame::Geocentric, Body::Pluto) => {
            Vsop2013Pluto.geocentric_position(tdb).map_err(err)
        }
    }
}

fn format_table(columns: &[String], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = columns.iter().map(|c| c.len()).collect();
    for r in rows {
        for (i, cell) in r.iter().enumerate() {
            if i < widths.len() && cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }
    let mut out = String::new();
    for (i, col) in columns.iter().enumerate() {
        if i > 0 {
            out.push_str("  ");
        }
        out.push_str(&format!("{:<w$}", col, w = widths[i]));
    }
    out.push('\n');
    for r in rows {
        for (i, cell) in r.iter().enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            out.push_str(&format!("{:<w$}", cell, w = widths[i]));
        }
        out.push('\n');
    }
    out
}

fn text_output(msg: impl Into<String>) -> KernelOutput {
    let s = msg.into();
    CellOutput {
        stdout: s.clone(),
        value: None,
        payload: OutputPayload::Text(s),
    }
}

fn lock<'a>(
    state: &'a Arc<Mutex<CosmosState>>,
) -> Result<std::sync::MutexGuard<'a, CosmosState>, KernelError> {
    state
        .lock()
        .map_err(|_| KernelError::Runtime("kernel state envenenado".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pluma_notebook_core::{CellKind, Notebook};
    use pluma_notebook_exec::run_all;

    #[tokio::test]
    async fn tdb_default_es_j2000() {
        let k = CosmosKernel::new();
        let s = k.snapshot();
        let expected = TDB::j2000().to_julian_date().to_f64();
        assert!(
            (s.tdb.to_julian_date().to_f64() - expected).abs() < 1e-9,
            "default tdb debe ser J2000"
        );
    }

    #[tokio::test]
    async fn tdb_acepta_iso8601() {
        let k = CosmosKernel::new();
        k.execute("2026-05-27T00:00:00", "cosmos-tdb")
            .await
            .unwrap();
        // Si no panicó y el state cambió, ya está. El JD exacto lo
        // valida cosmos-time en sus tests.
        let j2k = TDB::j2000().to_julian_date().to_f64();
        let cur = k.snapshot().tdb.to_julian_date().to_f64();
        assert!(cur != j2k, "el TDB debe haber cambiado de J2000");
    }

    #[tokio::test]
    async fn tdb_acepta_j2000_literal() {
        let k = CosmosKernel::new();
        k.execute("j2000", "cosmos-tdb").await.unwrap();
        let expected = TDB::j2000().to_julian_date().to_f64();
        let cur = k.snapshot().tdb.to_julian_date().to_f64();
        assert!((cur - expected).abs() < 1e-9);
    }

    #[tokio::test]
    async fn positions_default_devuelve_todos_excepto_earth() {
        let k = CosmosKernel::new();
        let out = k.execute("", "cosmos-positions").await.unwrap();
        if let OutputPayload::Table { rows, .. } = out.payload {
            // 10 cuerpos: sun moon mercury venus mars jupiter saturn
            // uranus neptune pluto (earth omitido en geocentric default).
            assert_eq!(rows.len(), 10);
            let bodies: Vec<&str> = rows.iter().map(|r| r[0].as_str()).collect();
            assert!(bodies.contains(&"sun"));
            assert!(bodies.contains(&"moon"));
            assert!(bodies.contains(&"mars"));
            assert!(!bodies.contains(&"earth"));
        } else {
            panic!("se esperaba Table");
        }
    }

    #[tokio::test]
    async fn positions_acepta_lista_explicita() {
        let k = CosmosKernel::new();
        let out = k.execute("mars venus", "cosmos-positions").await.unwrap();
        if let OutputPayload::Table { rows, .. } = out.payload {
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0][0], "mars");
            assert_eq!(rows[1][0], "venus");
        } else {
            panic!("se esperaba Table");
        }
    }

    #[tokio::test]
    async fn helio_incluye_earth_y_sun_cero() {
        let k = CosmosKernel::new();
        let out = k.execute("sun earth", "cosmos-helio").await.unwrap();
        if let OutputPayload::Table { rows, .. } = out.payload {
            assert_eq!(rows[0][0], "sun");
            // Sun heliocentric = origen.
            let r_sun: f64 = rows[0][4].parse().unwrap();
            assert!(r_sun < 1e-9);
            // Earth heliocentric a J2000 ~ 1 au.
            let r_earth: f64 = rows[1][4].parse().unwrap();
            assert!((r_earth - 1.0).abs() < 0.05, "earth ~ 1 au, fue {r_earth}");
        } else {
            panic!("se esperaba Table");
        }
    }

    #[tokio::test]
    async fn distance_devuelve_scalar() {
        let k = CosmosKernel::new();
        let out = k.execute("mars", "cosmos-distance").await.unwrap();
        match out.payload {
            OutputPayload::Scalar(d) => {
                // Mars desde la Tierra al J2000: entre 0.4 y 2.7 au.
                assert!(d > 0.3 && d < 3.0, "d_geo(mars) en rango fisico: {d}");
            }
            other => panic!("se esperaba Scalar, llegó {other:?}"),
        }
    }

    #[tokio::test]
    async fn distance_cuerpo_invalido_falla() {
        let k = CosmosKernel::new();
        let r = k.execute("estrella-de-la-muerte", "cosmos-distance").await;
        assert!(matches!(r, Err(KernelError::Runtime(_))));
    }

    #[tokio::test]
    async fn cambiar_tdb_cambia_posiciones() {
        let k = CosmosKernel::new();
        let d_j2000: f64 = match k
            .execute("mars", "cosmos-distance")
            .await
            .unwrap()
            .payload
        {
            OutputPayload::Scalar(v) => v,
            _ => unreachable!(),
        };
        k.execute("2010-06-15T00:00:00", "cosmos-tdb")
            .await
            .unwrap();
        let d_2010: f64 = match k
            .execute("mars", "cosmos-distance")
            .await
            .unwrap()
            .payload
        {
            OutputPayload::Scalar(v) => v,
            _ => unreachable!(),
        };
        assert!(
            (d_j2000 - d_2010).abs() > 0.01,
            "cambiar TDB debe cambiar la distancia geocéntrica"
        );
    }

    #[tokio::test]
    async fn moon_en_au_no_en_km() {
        // Regresión: ELP/MPP02 devuelve km; el kernel debe convertir
        // a au antes de meter en la tabla. d_geo(luna) ~ 0.0025 au
        // (~ 384000 km / AU_KM), no ~ 380000.
        let k = CosmosKernel::new();
        let out = k.execute("moon", "cosmos-distance").await.unwrap();
        if let OutputPayload::Scalar(d) = out.payload {
            assert!(
                d > 0.0020 && d < 0.0030,
                "d_geo(moon) en au debe estar ~0.0025, fue {d}"
            );
        } else {
            panic!("se esperaba Scalar");
        }
    }

    #[tokio::test]
    async fn lenguaje_no_cosmos_falla() {
        let k = CosmosKernel::new();
        let r = k.execute("python", "fortran").await;
        assert!(matches!(r, Err(KernelError::Runtime(ref m)) if m.contains("no reconocido")));
    }

    #[tokio::test]
    async fn location_acepta_lat_lon_alt() {
        let k = CosmosKernel::new();
        k.execute("-12.05 -77.05 150", "cosmos-location")
            .await
            .unwrap();
        let s = k.snapshot();
        assert!((s.location.latitude_degrees() - (-12.05)).abs() < 1e-6);
        assert!((s.location.longitude_degrees() - (-77.05)).abs() < 1e-6);
    }

    #[tokio::test]
    async fn location_falla_con_args_incorrectos() {
        let k = CosmosKernel::new();
        let r = k.execute("-12.05 -77.05", "cosmos-location").await;
        assert!(matches!(r, Err(KernelError::Runtime(_))));
    }

    #[tokio::test]
    async fn skywatch_default_devuelve_diez_filas() {
        let k = CosmosKernel::new();
        k.execute("-12.05 -77.05 150", "cosmos-location")
            .await
            .unwrap();
        k.execute("2026-05-27T17:00:00", "cosmos-tdb")
            .await
            .unwrap();
        let out = k.execute("", "cosmos-skywatch").await.unwrap();
        if let OutputPayload::Table { rows, columns } = out.payload {
            assert_eq!(rows.len(), 10, "10 cuerpos");
            assert_eq!(columns[0], "body");
            assert_eq!(columns[1], "alt_deg");
        } else {
            panic!("se esperaba Table");
        }
    }

    #[tokio::test]
    async fn skywatch_acepta_lista_explicita() {
        let k = CosmosKernel::new();
        let out = k.execute("sun mars", "cosmos-skywatch").await.unwrap();
        if let OutputPayload::Table { rows, .. } = out.payload {
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0][0], "sun");
        } else {
            panic!("se esperaba Table");
        }
    }

    #[tokio::test]
    async fn sundial_devuelve_una_fila() {
        let k = CosmosKernel::new();
        k.execute("-12.05 -77.05 150", "cosmos-location")
            .await
            .unwrap();
        k.execute("2026-05-27T17:00:00", "cosmos-tdb")
            .await
            .unwrap();
        let out = k.execute("", "cosmos-sundial").await.unwrap();
        if let OutputPayload::Table { rows, columns } = out.payload {
            assert_eq!(rows.len(), 1);
            assert!(columns.contains(&"hour_angle_deg".into()));
        } else {
            panic!("se esperaba Table");
        }
    }

    #[tokio::test]
    async fn tides_tres_filas_lunar_solar_total() {
        let k = CosmosKernel::new();
        k.execute("-12.05 -77.05 0", "cosmos-location")
            .await
            .unwrap();
        let out = k.execute("", "cosmos-tides").await.unwrap();
        if let OutputPayload::Table { rows, .. } = out.payload {
            assert_eq!(rows.len(), 3, "lunar + solar + total");
            assert_eq!(rows[0][0], "lunar");
            assert_eq!(rows[1][0], "solar");
            assert_eq!(rows[2][0], "total");
        } else {
            panic!("se esperaba Table");
        }
    }

    #[tokio::test]
    async fn rise_set_default_diez_cuerpos() {
        let k = CosmosKernel::new();
        k.execute("-12.05 -77.05 150", "cosmos-location")
            .await
            .unwrap();
        k.execute("2026-05-27T00:00:00", "cosmos-tdb")
            .await
            .unwrap();
        let out = k.execute("", "cosmos-rise-set").await.unwrap();
        if let OutputPayload::Table { rows, columns } = out.payload {
            assert_eq!(rows.len(), 10);
            assert!(columns.contains(&"transit_jd".into()));
            // El Sol debe estar en estado "sale" en Lima en mayo.
            let sun = rows.iter().find(|r| r[0] == "sun").unwrap();
            assert_eq!(sun[5], "sale");
        } else {
            panic!("se esperaba Table");
        }
    }

    #[tokio::test]
    async fn eclipses_solar_default() {
        let k = CosmosKernel::new();
        k.execute("2026-01-01T00:00:00", "cosmos-tdb")
            .await
            .unwrap();
        let out = k.execute("4 solar", "cosmos-eclipses").await.unwrap();
        if let OutputPayload::Table { rows, .. } = out.payload {
            // 2026-2030 → al menos 5 eclipses solares geocéntricos.
            assert!(
                rows.len() >= 5,
                "≥ 5 eclipses solares en 4 años, fueron {}",
                rows.len()
            );
        } else {
            panic!("se esperaba Table");
        }
    }

    #[tokio::test]
    async fn eclipses_lunar_explicito() {
        let k = CosmosKernel::new();
        k.execute("2026-01-01T00:00:00", "cosmos-tdb")
            .await
            .unwrap();
        let out = k.execute("2 lunar", "cosmos-eclipses").await.unwrap();
        if let OutputPayload::Table { rows, .. } = out.payload {
            // 2026-2028 → ≥ 2 eclipses lunares.
            assert!(rows.len() >= 2);
        } else {
            panic!("se esperaba Table");
        }
    }

    #[tokio::test]
    async fn transits_mercurio_2032() {
        let k = CosmosKernel::new();
        // TDB en 2030: en 15 años (default) debe encontrarse el
        // tránsito de Mercurio del 2032-11-13.
        k.execute("2030-01-01T00:00:00", "cosmos-tdb")
            .await
            .unwrap();
        let out = k.execute("", "cosmos-transits").await.unwrap();
        if let OutputPayload::Table { rows, .. } = out.payload {
            let has_mercury = rows.iter().any(|r| r[0] == "mercury");
            assert!(has_mercury, "se esperaba el tránsito de Mercurio 2032");
        } else {
            panic!("se esperaba Table");
        }
    }

    #[tokio::test]
    async fn notebook_completo_topo_order() {
        let mut nb = Notebook::new();
        let t = nb.push(
            CellKind::Code { language: "cosmos-tdb".into() },
            "2026-05-27T00:00:00",
        );
        let p = nb.push(
            CellKind::Code { language: "cosmos-positions".into() },
            "sun mars venus",
        );
        let d = nb.push(
            CellKind::Code { language: "cosmos-distance".into() },
            "mars",
        );
        nb.add_dependency(p, t);
        nb.add_dependency(d, t);

        let k = CosmosKernel::new();
        let report = run_all(&mut nb, &k).await.unwrap();
        assert_eq!(report.executed.len(), 3);
        assert!(report.failed.is_empty());

        // La celda de distancia debe haber guardado un Scalar.
        let d_cell = nb.cell(d).unwrap();
        assert!(matches!(
            d_cell.last_output.as_ref().unwrap().payload,
            OutputPayload::Scalar(_)
        ));
    }
}
