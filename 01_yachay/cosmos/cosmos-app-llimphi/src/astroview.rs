//! Gráficas astronómicas (no astrológicas) sobre el mismo motor de
//! efemérides: cielo (alt/az), orto/ocaso, reloj de sol, mareas,
//! eclipses y efemérides. El cómputo es puntual para el instante de la
//! carta (o "ahora", según `CosmosConfig::use_now`) y la ubicación del
//! lugar de nacimiento.
//!
//! `AstroState` cachea las lecturas: se recalcula sólo al cambiar carta
//! o instante (ver `main::recompute_astro`), nunca por frame.

use cosmos_core::Location;
use cosmos_eclipses::{
    lunar_reading_at, solar_reading_at, LunarEclipseKind, LunarEclipseReading, SolarEclipseKind,
    SolarEclipseReading,
};
use cosmos_model::Chart;
use cosmos_rise_set::{rise_transit_set, Horizon, RiseTransitSet};
use cosmos_skywatch::{sky_positions_all, Body, SkyPosition};
use cosmos_sundial::{sundial_reading, SundialReading};
use cosmos_tides::{tide_reading, TideReading};
use cosmos_time::{utc_from_calendar, JulianDate, UTC, TDB};

use llimphi_theme::Theme;
use llimphi_ui::View;

use crate::format::simbolo_cuerpo;
use crate::model::Msg;
use crate::view::{line, section_label, tile_container};

/// Lecturas astronómicas cacheadas para el instante/lugar vigente.
#[derive(Clone)]
pub(crate) struct AstroState {
    pub(crate) instant_iso: String,
    pub(crate) place_label: String,
    /// Tiempo sidéreo local (grados) y latitud del observador — para
    /// proyectar las constelaciones al cielo del observador (alt/az).
    pub(crate) lst_deg: f64,
    pub(crate) lat_deg: f64,
    pub(crate) sky: Vec<(Body, SkyPosition)>,
    pub(crate) sundial: SundialReading,
    pub(crate) tide: TideReading,
    /// Orto/tránsito/ocaso por cuerpo, con el horizonte usado.
    pub(crate) riseset: Vec<(Body, RiseTransitSet)>,
    pub(crate) solar: SolarEclipseReading,
    pub(crate) lunar: LunarEclipseReading,
}

fn build_instant(chart: &Chart, use_now: bool) -> (TDB, String, f64) {
    let utc = if use_now {
        UTC::now()
    } else {
        let bd = &chart.birth_data;
        // El calendario guardado es local; restamos el offset para
        // obtener UTC real antes de pasar a TDB.
        utc_from_calendar(
            bd.year,
            bd.month as u8,
            bd.day as u8,
            bd.hour as u8,
            bd.minute as u8,
            bd.second,
        )
        .add_seconds(-(bd.tz_offset_minutes as f64) * 60.0)
    };
    let jd_ut = utc.to_julian_date().to_f64();
    let tdb = TDB::from(utc.to_julian_date());
    (tdb, utc.to_iso8601(), jd_ut)
}

/// GMST en grados (fórmula IAU 1982, suficiente para ubicar figuras).
fn gmst_deg(jd_ut: f64) -> f64 {
    let t = (jd_ut - 2451545.0) / 36525.0;
    let g = 280.46061837 + 360.98564736629 * (jd_ut - 2451545.0)
        + 0.000387933 * t * t
        - t * t * t / 38710000.0;
    g.rem_euclid(360.0)
}

fn build_location(chart: &Chart) -> Location {
    let bd = &chart.birth_data;
    Location::from_degrees(bd.latitude_deg, bd.longitude_deg, bd.altitude_m)
        .unwrap_or_else(|_| Location::from_degrees(0.0, 0.0, 0.0).expect("loc 0,0"))
}

/// Horizonte estándar por cuerpo (refracción + semidiámetro).
fn horizon_for(body: Body) -> Horizon {
    match body {
        Body::Sun => Horizon::SunStandard,
        Body::Moon => Horizon::MoonStandard,
        _ => Horizon::Geometric,
    }
}

pub(crate) fn compute_astro(chart: &Chart, use_now: bool) -> AstroState {
    let (tdb, instant_iso, jd_ut) = build_instant(chart, use_now);
    let loc = build_location(chart);
    let lst_deg = (gmst_deg(jd_ut) + chart.birth_data.longitude_deg).rem_euclid(360.0);
    let lat_deg = chart.birth_data.latitude_deg;

    let sky: Vec<(Body, SkyPosition)> = sky_positions_all(&tdb, &loc).to_vec();
    let sundial = sundial_reading(&tdb, &loc);
    let tide = tide_reading(&tdb, &loc);
    let riseset: Vec<(Body, RiseTransitSet)> = Body::all()
        .iter()
        .map(|b| (*b, rise_transit_set(b, &tdb, &loc, horizon_for(*b))))
        .collect();
    let solar = solar_reading_at(&tdb);
    let lunar = lunar_reading_at(&tdb);

    let place_label = chart
        .birth_data
        .birthplace_label
        .clone()
        .unwrap_or_else(|| {
            format!(
                "{:.3}°, {:.3}°",
                chart.birth_data.latitude_deg, chart.birth_data.longitude_deg
            )
        });

    AstroState {
        instant_iso,
        place_label,
        lst_deg,
        lat_deg,
        sky,
        sundial,
        tide,
        riseset,
        solar,
        lunar,
    }
}

// =====================================================================
// Renderers
// =====================================================================

/// Placeholder mientras el cómputo astronómico (orto/ocaso/efemérides, la
/// parte cara: 144 muestras × 10 cuerpos) corre en un worker. La UI nunca se
/// bloquea esperándolo; se reemplaza por las lecturas reales al reentrar el
/// `Msg::AstroComputed`.
pub(crate) fn calculando(theme: &Theme) -> View<Msg> {
    tile_container(
        vec![line("calculando…".to_string(), 12.0, theme.fg_muted)],
        theme,
    )
}

/// Cabecera común: instante + lugar.
fn astro_header(a: &AstroState, theme: &Theme) -> View<Msg> {
    line(
        format!("{}  ·  {}", a.instant_iso, a.place_label),
        10.0,
        theme.fg_muted,
    )
}

/// `HH:MM` UTC de un instante TDB (aprox: ignora TDB−UTC ~ms).
fn hhmm(tdb: &TDB) -> String {
    let iso = UTC::from_julian_date(jd_of(tdb)).to_iso8601();
    // ISO: YYYY-MM-DDTHH:MM:SS… → tomamos HH:MM.
    iso.get(11..16).unwrap_or("--:--").to_string()
}

fn jd_of(tdb: &TDB) -> JulianDate {
    tdb.to_julian_date()
}

/// Cielo: tabla alt/az de los 10 cuerpos, ordenada por altitud.
pub(crate) fn view_cielo(a: &AstroState, theme: &Theme) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = vec![astro_header(a, theme)];
    rows.push(line(
        format!("{:<5}{:>8}{:>8}{:>10}", "cuer", "alt", "az", "dist(au)"),
        10.0,
        theme.fg_muted,
    ));
    let mut sky = a.sky.clone();
    sky.sort_by(|x, y| {
        y.1.visibility_score()
            .partial_cmp(&x.1.visibility_score())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (b, p) in &sky {
        let color = if p.above_horizon {
            theme.fg_text
        } else {
            theme.fg_muted
        };
        let txt = format!(
            "{:<5}{:>7.1}°{:>7.1}°{:>10.3}",
            simbolo_cuerpo(b.canonical()),
            p.altitude_deg,
            p.azimuth_deg,
            p.distance_au
        );
        rows.push(line(txt, 11.0, color));
    }
    tile_container(rows, theme)
}

/// Orto/tránsito/ocaso por cuerpo (horarios en UTC).
pub(crate) fn view_ortoocaso(a: &AstroState, theme: &Theme) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = vec![astro_header(a, theme)];
    rows.push(line(
        format!(
            "{:<5}{:>8}{:>8}{:>8}{:>8}",
            "cuer", "orto", "culm", "alt°", "ocaso"
        ),
        10.0,
        theme.fg_muted,
    ));
    for (b, r) in &a.riseset {
        let orto = if r.never_rises {
            "----".to_string()
        } else if r.never_sets {
            "circ".to_string()
        } else {
            r.rise.as_ref().map(hhmm).unwrap_or_else(|| "----".into())
        };
        let culm = hhmm(&r.transit);
        let ocaso = r.set.as_ref().map(hhmm).unwrap_or_else(|| "----".into());
        let txt = format!(
            "{:<5}{:>8}{:>8}{:>7.1}{:>8}",
            simbolo_cuerpo(b.canonical()),
            orto,
            culm,
            r.transit_altitude_deg,
            ocaso
        );
        rows.push(line(txt, 11.0, theme.fg_text));
    }
    tile_container(rows, theme)
}

/// Reloj de sol: azimut y largo de sombra del gnomon.
pub(crate) fn view_sundial(a: &AstroState, theme: &Theme) -> View<Msg> {
    let s = &a.sundial;
    let mut rows: Vec<View<Msg>> = vec![astro_header(a, theme)];
    rows.push(section_label("Sol".to_string(), theme));
    rows.push(line(
        format!(
            "altitud {:.2}°   azimut {:.2}°",
            s.sun.altitude_deg, s.sun.azimuth_deg
        ),
        11.0,
        theme.fg_text,
    ));
    rows.push(line(
        format!("ángulo horario {:.2}°", s.hour_angle_deg),
        11.0,
        theme.fg_muted,
    ));
    rows.push(section_label("Sombra del gnomon".to_string(), theme));
    match (s.shadow_azimuth_deg, s.shadow_length_ratio) {
        (Some(az), Some(ratio)) => {
            rows.push(line(
                format!("azimut de sombra {az:.2}°"),
                11.0,
                theme.fg_text,
            ));
            rows.push(line(
                format!("largo / altura del gnomon = {ratio:.2}"),
                11.0,
                theme.fg_text,
            ));
            rows.push(line(
                format!("(gnomon de 1 m -> sombra de {:.2} m)", ratio),
                10.0,
                theme.fg_muted,
            ));
        }
        _ => {
            rows.push(line(
                "Sol bajo el horizonte — sin sombra".to_string(),
                11.0,
                theme.fg_muted,
            ));
        }
    }
    tile_container(rows, theme)
}

/// Mareas de equilibrio (Sol + Luna).
pub(crate) fn view_mareas(a: &AstroState, theme: &Theme) -> View<Msg> {
    let t = &a.tide;
    let mut rows: Vec<View<Msg>> = vec![astro_header(a, theme)];
    rows.push(section_label("Marea de equilibrio".to_string(), theme));
    rows.push(line(
        format!("total {:+.3} m", t.total_height_m),
        13.0,
        theme.fg_text,
    ));
    let comp = |label: &str, c: &cosmos_tides::ComponentReading| -> String {
        format!(
            "{label:<6} {:+.3} m   cenital {:.1}°   alt {:.1}°",
            c.height_m, c.zenith_deg, c.sky.altitude_deg
        )
    };
    rows.push(section_label("Componentes".to_string(), theme));
    rows.push(line(comp("Luna", &t.lunar), 11.0, theme.fg_text));
    rows.push(line(comp("Sol", &t.solar), 11.0, theme.fg_text));
    rows.push(line(
        "MVP: fuerza generadora, sin respuesta hidrodinámica de la cuenca."
            .to_string(),
        9.0,
        theme.fg_muted,
    ));
    tile_container(rows, theme)
}

/// Eclipses: lectura puntual solar y lunar para el instante.
pub(crate) fn view_eclipses(a: &AstroState, theme: &Theme) -> View<Msg> {
    let s = &a.solar;
    let l = &a.lunar;
    let solar_kind = match s.kind {
        SolarEclipseKind::None => "ninguno",
        SolarEclipseKind::Partial => "parcial",
        SolarEclipseKind::Annular => "anular",
        SolarEclipseKind::Total => "total",
    };
    let lunar_kind = match l.kind {
        LunarEclipseKind::None => "ninguno",
        LunarEclipseKind::Penumbral => "penumbral",
        LunarEclipseKind::Partial => "parcial",
        LunarEclipseKind::Total => "total",
    };
    let mut rows: Vec<View<Msg>> = vec![astro_header(a, theme)];
    rows.push(section_label("Eclipse solar".to_string(), theme));
    rows.push(line(
        format!("tipo: {solar_kind}   magnitud {:.3}", s.magnitude),
        11.0,
        theme.fg_text,
    ));
    rows.push(line(
        format!(
            "separación Sol·Luna {:.3}°   (r Sol {:.3}°, r Luna {:.3}°)",
            s.separation_deg, s.sun_apparent_radius_deg, s.moon_apparent_radius_deg
        ),
        11.0,
        theme.fg_muted,
    ));
    rows.push(section_label("Eclipse lunar".to_string(), theme));
    rows.push(line(
        format!("tipo: {lunar_kind}   magnitud umbral {:.3}", l.umbral_magnitude),
        11.0,
        theme.fg_text,
    ));
    rows.push(line(
        format!(
            "γ {:.0} km   umbra {:.0} km   penumbra {:.0} km",
            l.gamma_km, l.umbra_radius_km, l.penumbra_radius_km
        ),
        11.0,
        theme.fg_muted,
    ));
    tile_container(rows, theme)
}

/// Efemérides: RA/dec/distancia de los cuerpos para el instante.
pub(crate) fn view_efemerides(a: &AstroState, theme: &Theme) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = vec![astro_header(a, theme)];
    rows.push(line(
        format!("{:<5}{:>10}{:>9}{:>11}", "cuer", "AR(h)", "dec°", "dist(au)"),
        10.0,
        theme.fg_muted,
    ));
    for (b, p) in &a.sky {
        let ra_h = p.right_ascension_deg / 15.0;
        let txt = format!(
            "{:<5}{:>9.3}h{:>8.2}°{:>11.4}",
            simbolo_cuerpo(b.canonical()),
            ra_h,
            p.declination_deg,
            p.distance_au
        );
        rows.push(line(txt, 11.0, theme.fg_text));
    }
    tile_container(rows, theme)
}
