//! Bridge real: `cosmobiologia_model::Chart` → eternal_astrology → [`RenderModel`].
//!
//! La sesión de efemérides VSOP2013 es **compartida globalmente** vía
//! `OnceLock` — abrirla cuesta unos cuantos ms (carga de las series en
//! memoria), y como es read-only se puede leer en paralelo desde varios
//! cómputos.

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use eternal_astrology::{
    all_lots, composite, directed_longitude, find_aspects, find_synastry_aspects, next_return,
    primary_direction::PrimaryDirection, secondary_progression, solar_arc_true, topocentric_ecliptic,
    Aspect, AspectKind as EAspectKind, BirthData, BodySet, ChartConfig,
    DirectionKey as EDirectionKey, HouseSystem as EHouseSystem, Houses as EHouses, NatalChart,
    OrbTable, Zodiac as EZodiac,
};
use eternal_sky::{Ayanamsha, Body, EphemerisSession, Instant as ESInstant, Observer, SessionConfig};

use cosmobiologia_model::{Chart, HouseSystem, StoredChartConfig, Zodiac};

use crate::dignity::essential_dignity;
use crate::{
    compute_gr_triggers, AspectSummary, EngineError, Geometry, Glyph, GrDirection, Layer,
    LayerKind, LineSeg, OverlayMeta, RenderModel, UranianGroup,
};

// =====================================================================
// Sesión global cacheada
// =====================================================================

static SESSION: OnceLock<EphemerisSession> = OnceLock::new();

fn session() -> Result<&'static EphemerisSession, EngineError> {
    if let Some(s) = SESSION.get() {
        return Ok(s);
    }
    let opened = EphemerisSession::open(SessionConfig::vsop2013())
        .map_err(|e| EngineError::Eternal(format!("EphemerisSession::open: {:?}", e)))?;
    // Si otro thread ya pobló la celda mientras abríamos, el set_once
    // falla silenciosamente — usamos el que quedó dentro.
    let _ = SESSION.set(opened);
    Ok(SESSION.get().expect("session was just set"))
}

// =====================================================================
// Traducciones Stored* → eternal
// =====================================================================

fn map_house_system(h: HouseSystem) -> EHouseSystem {
    match h {
        HouseSystem::Placidus => EHouseSystem::Placidus,
        HouseSystem::Koch => EHouseSystem::Koch,
        HouseSystem::Regiomontanus => EHouseSystem::Regiomontanus,
        HouseSystem::Campanus => EHouseSystem::Campanus,
        HouseSystem::Porphyry => EHouseSystem::Porphyry,
        HouseSystem::Equal => EHouseSystem::Equal,
        HouseSystem::WholeSign => EHouseSystem::WholeSign,
    }
}

fn map_zodiac(z: Zodiac, ayanamsha_hint: Option<&str>) -> EZodiac {
    match z {
        Zodiac::Tropical => EZodiac::Tropical,
        Zodiac::Sidereal => {
            let mode = match ayanamsha_hint.unwrap_or("lahiri").to_ascii_lowercase().as_str() {
                "fagan_bradley" | "fagan-bradley" | "faganbradley" => Ayanamsha::FaganBradley,
                "raman" => Ayanamsha::Raman,
                "krishnamurti" => Ayanamsha::Krishnamurti,
                "de_luce" | "deluce" => Ayanamsha::DeLuce,
                "djwhal_khul" | "djwhalkhul" => Ayanamsha::DjwhalKhul,
                "ushashashi" => Ayanamsha::Ushashashi,
                "yukteshwar" => Ayanamsha::Yukteshwar,
                _ => Ayanamsha::Lahiri,
            };
            EZodiac::Sidereal(mode)
        }
        // Dracónico aún no soportado en eternal — caemos a tropical por
        // ahora; cuando eternal lo agregue, lo cableamos acá.
        Zodiac::Draconic => EZodiac::Tropical,
    }
}

fn map_body_set(cfg: &StoredChartConfig) -> BodySet {
    let mut bodies: Vec<Body> = Vec::new();
    for name in &cfg.bodies {
        if let Some(b) = map_body(name) {
            bodies.push(b);
        }
    }
    if bodies.is_empty() {
        // Default razonable si el config vino vacío.
        return BodySet::classical_modern();
    }
    let mut set = BodySet {
        bodies,
        include_south_node: cfg.include_south_node,
    };
    if cfg.include_lilith {
        set = set.with_lilith();
    }
    if cfg.include_main_belt_asteroids {
        set = set.with_main_belt_asteroids();
    }
    set
}

fn map_body(name: &str) -> Option<Body> {
    Some(match name.to_ascii_lowercase().as_str() {
        "sun" => Body::Sun,
        "moon" => Body::Moon,
        "mercury" => Body::Mercury,
        "venus" => Body::Venus,
        "mars" => Body::Mars,
        "jupiter" => Body::Jupiter,
        "saturn" => Body::Saturn,
        "uranus" => Body::Uranus,
        "neptune" => Body::Neptune,
        "pluto" => Body::Pluto,
        "mean_node" | "meannode" => Body::MeanNode,
        "true_node" | "truenode" => Body::TrueNode,
        "mean_lilith" | "lilith" => Body::MeanLilith,
        "true_lilith" => Body::TrueLilith,
        "ceres" => Body::Ceres,
        "pallas" => Body::Pallas,
        "juno" => Body::Juno,
        "vesta" => Body::Vesta,
        _ => return None,
    })
}

pub(crate) fn body_symbol(b: Body) -> &'static str {
    match b {
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
        Body::MeanNode => "north_node",
        Body::TrueNode => "north_node",
        Body::MeanLilith => "lilith",
        Body::TrueLilith => "lilith",
        Body::Ceres => "ceres",
        Body::Pallas => "pallas",
        Body::Juno => "juno",
        Body::Vesta => "vesta",
        Body::Chiron => "chiron",
        Body::Pholus => "chiron",
        Body::Eris => "chiron",
        Body::Sedna => "chiron",
        // `Body` es `#[non_exhaustive]` — cualquier cuerpo nuevo
        // upstream cae al símbolo de fallback hasta que lo cableemos.
        _ => "custom",
    }
}

fn aspect_kind_id(k: EAspectKind) -> &'static str {
    match k {
        EAspectKind::Conjunction => "conjunction",
        EAspectKind::Opposition => "opposition",
        EAspectKind::Trine => "trine",
        EAspectKind::Square => "square",
        EAspectKind::Sextile => "sextile",
        EAspectKind::Quincunx => "quincunx",
        EAspectKind::SemiSextile => "semi_sextile",
        EAspectKind::SemiSquare => "semi_square",
        EAspectKind::Sesquiquadrate => "sesquiquadrate",
        EAspectKind::Quintile => "quintile",
        EAspectKind::BiQuintile => "biquintile",
        EAspectKind::Septile => "septile",
    }
}

// =====================================================================
// compute()
// =====================================================================

/// Construye los tipos eternales (`BirthData`, `ChartConfig`) desde el
/// `Chart` agnóstico, aplicando el offset temporal. Devuelve también el
/// `Observer` y la `ChartConfig` para reusar en pipelines extendidas
/// (transits, sinastría) sin re-traducir.
fn build_eternal_inputs(
    chart: &Chart,
    offset_seconds: i64,
) -> Result<(BirthData, ChartConfig, Observer), EngineError> {
    chart.validate()?;
    let bd = &chart.birth_data;
    let base_instant = ESInstant::from_civil_local(
        bd.year,
        u8::try_from(bd.month).map_err(|_| {
            EngineError::Eternal(format!("mes fuera de u8: {}", bd.month))
        })?,
        u8::try_from(bd.day).map_err(|_| {
            EngineError::Eternal(format!("día fuera de u8: {}", bd.day))
        })?,
        u8::try_from(bd.hour).map_err(|_| {
            EngineError::Eternal(format!("hora fuera de u8: {}", bd.hour))
        })?,
        u8::try_from(bd.minute).map_err(|_| {
            EngineError::Eternal(format!("minuto fuera de u8: {}", bd.minute))
        })?,
        bd.second,
        bd.tz_offset_minutes,
    )
    .map_err(|e| EngineError::Eternal(format!("Instant::from_civil_local: {:?}", e)))?;

    // Microajuste temporal en SEGUNDOS — el rectificador automático
    // barre la hora candidata con resolución de segundo.
    let instant = if offset_seconds == 0 {
        base_instant
    } else {
        let shifted_utc = base_instant.utc().add_seconds(offset_seconds as f64);
        ESInstant::from_utc(shifted_utc)
    };

    let observer = Observer::from_degrees(bd.latitude_deg, bd.longitude_deg, bd.altitude_m);
    let mut birth_e = BirthData::new(instant, observer);
    if let Some(name) = &bd.subject_name {
        birth_e = birth_e.with_name(name.clone());
    }
    let config_e = ChartConfig {
        house_system: map_house_system(chart.config.house_system),
        zodiac: map_zodiac(chart.config.zodiac, chart.config.ayanamsha.as_deref()),
        bodies: map_body_set(&chart.config),
        include_horizon: false,
    };
    Ok((birth_e, config_e, observer))
}

/// Computa la `NatalChart` consultando primero el LRU cache global.
/// Útil para pipelines compuestas (transits, sinastría, composite) que
/// computan la misma carta natal del partner en cada render — bajo
/// drag de sliders se llama decenas de veces seguidas con inputs
/// idénticos.
///
/// La clave incluye todos los campos de `StoredBirthData` y
/// `StoredChartConfig` que afectan el cómputo; editar la carta invalida
/// automáticamente la entrada.
pub(crate) fn compute_natal_chart(
    chart: &Chart,
    offset_seconds: i64,
) -> Result<(Arc<NatalChart>, ChartConfig, Observer), EngineError> {
    let (birth_e, config_e, observer) = build_eternal_inputs(chart, offset_seconds)?;
    let key = crate::natal_cache::key_for(&chart.birth_data, &chart.config, offset_seconds);
    if let Some(cached) = crate::natal_cache::get(key) {
        return Ok((cached, config_e, observer));
    }
    let session = session()?;
    let natal = NatalChart::compute(&birth_e, &config_e, session)
        .map_err(|e| EngineError::Eternal(format!("NatalChart::compute: {:?}", e)))?;
    let arc = Arc::new(natal);
    crate::natal_cache::insert(key, arc.clone());
    Ok((arc, config_e, observer))
}

/// Composición principal: natal + overlays pedidos. Es la función que
/// `lib::compose` delega cuando el feature `eternal-bridge` está activo.
pub fn compose(
    chart: &Chart,
    offset_minutes: i64,
    requests: &[crate::PipelineRequest],
    natal_options: &crate::NatalOptions,
) -> Result<RenderModel, EngineError> {
    let t0 = Instant::now();
    // `compute_natal_chart` trabaja en segundos; `compose` recibe el
    // offset en minutos (el scrub del jog-dial, la API pública).
    let (natal, config_e, observer) = compute_natal_chart(chart, offset_minutes * 60)?;
    let orb_table = build_orb_table(natal_options.orb_multiplier);
    let all_aspects = find_aspects(&natal, &orb_table);
    let aspects: Vec<Aspect> = all_aspects
        .into_iter()
        .filter(|a| {
            let is_major = EAspectKind::MAJORS.contains(&a.kind);
            (is_major && natal_options.show_majors)
                || (!is_major && natal_options.show_minors)
        })
        .collect();
    let mut render = build_render_model(chart, &natal, &aspects, t0);
    if natal_options.show_dignities {
        annotate_dignities(&natal, &mut render);
    }
    populate_natal_aspect_summary(&aspects, &mut render);

    // Carta armónica: re-renderiza los cuerpos natales en su armónico
    // de orden N y recomputa sus aspectos. Se aplica antes de los
    // overlays — éstos quedan en coordenadas natales (la armónica es
    // un análisis de la carta natal pura).
    crate::apply_harmonic(&mut render, natal_options.harmonic);

    for req in requests {
        match req {
            crate::PipelineRequest::Transit => {
                build_transit_overlay(&natal, &config_e, observer, ESInstant::now(), &mut render)?;
                push_overlay_meta(&mut render, "transit", "Tránsito ahora".into());
            }
            crate::PipelineRequest::SecondaryProgression { target_age_years } => {
                build_progression_overlay(&natal, *target_age_years, &mut render)?;
                push_overlay_meta(
                    &mut render,
                    "progression",
                    format!("Progresión {:.1}a", target_age_years),
                );
            }
            crate::PipelineRequest::SolarArc { target_age_years } => {
                build_solar_arc_overlay(&natal, *target_age_years, &mut render)?;
                push_overlay_meta(
                    &mut render,
                    "solar_arc",
                    format!("Solar Arc {:.1}a", target_age_years),
                );
            }
            crate::PipelineRequest::Synastry { partner_chart } => {
                let partner_label = partner_chart.label.clone();
                build_synastry_overlay(&natal, partner_chart, &mut render)?;
                push_overlay_meta(
                    &mut render,
                    "synastry",
                    format!("Sinastría · {}", partner_label),
                );
            }
            crate::PipelineRequest::Midpoints => {
                build_midpoints_overlay(&natal, &mut render);
                push_overlay_meta(&mut render, "midpoints", "Midpoints ☉/☽".into());
            }
            crate::PipelineRequest::PlanetaryReturn {
                body,
                target_age_years,
                shift_days,
            } => {
                let body_e = map_body(body).ok_or_else(|| {
                    EngineError::Eternal(format!(
                        "body desconocido para planetary return: {}",
                        body
                    ))
                })?;
                build_planetary_return_overlay(
                    &natal,
                    &config_e,
                    observer,
                    body_e,
                    *target_age_years,
                    *shift_days,
                    &mut render,
                )?;
                let shift_label = if *shift_days == 0 {
                    String::new()
                } else {
                    format!(" {:+}d", shift_days)
                };
                push_overlay_meta(
                    &mut render,
                    "planetary_return",
                    format!("{} return {:.0}a{}", body_e.name(), target_age_years, shift_label),
                );
            }
            crate::PipelineRequest::Composite { partner_chart } => {
                let partner_label = partner_chart.label.clone();
                build_composite_overlay(&natal, partner_chart, &mut render)?;
                push_overlay_meta(
                    &mut render,
                    "composite",
                    format!("Composite · {}", partner_label),
                );
            }
            crate::PipelineRequest::Uranian => {
                build_uranian_groups(&natal, &mut render);
                let n = render.uranian_groups.len();
                push_overlay_meta(
                    &mut render,
                    "uranian",
                    if n == 0 {
                        "Uraniano · sin ejes".into()
                    } else {
                        format!("Uraniano · {} ejes", n)
                    },
                );
            }
            crate::PipelineRequest::Lots => {
                let count = build_lots_overlay(&natal, &mut render)?;
                push_overlay_meta(&mut render, "lots", format!("Lots · {}", count));
            }
            crate::PipelineRequest::FixedStars => {
                let count = build_fixed_stars_overlay(chart, &mut render);
                push_overlay_meta(
                    &mut render,
                    "fixed_stars",
                    format!("Estrellas fijas · {}", count),
                );
            }
            crate::PipelineRequest::Topocentric => {
                build_topocentric_overlay(&natal, &mut render)?;
                push_overlay_meta(
                    &mut render,
                    "topocentric",
                    "Topocéntrico (Polich-Page)".into(),
                );
            }
            crate::PipelineRequest::PrimaryDirections {
                target_age_years,
                key,
            } => {
                let dkey = match key.as_str() {
                    "ptolemy" => EDirectionKey::Ptolemy,
                    _ => EDirectionKey::Naibod,
                };
                build_primary_directions_overlay(
                    &natal,
                    *target_age_years,
                    dkey,
                    &mut render,
                );
                push_overlay_meta(
                    &mut render,
                    "primary_directions",
                    format!(
                        "GR Direcciones · {:.1}a · {}",
                        target_age_years,
                        match dkey {
                            EDirectionKey::Naibod => "Naibod",
                            EDirectionKey::Ptolemy => "Ptolomeo",
                        }
                    ),
                );
            }
        }
    }

    render.compute_ms = t0.elapsed().as_millis() as u64;
    Ok(render)
}

/// Helper: agrega al `RenderModel` las dos capas del overlay de
/// tránsitos (Outer + cross Aspects).
fn build_transit_overlay(
    natal: &NatalChart,
    config_e: &ChartConfig,
    observer: Observer,
    transit_at: ESInstant,
    render: &mut RenderModel,
) -> Result<(), EngineError> {
    let transit_birth = BirthData::new(transit_at, observer);
    let session = session()?;
    let transit = NatalChart::compute(&transit_birth, config_e, session).map_err(|e| {
        EngineError::Eternal(format!("NatalChart::compute (transit): {:?}", e))
    })?;

    let outer_glyphs: Vec<Glyph> = transit
        .placements
        .iter()
        .map(|p| Glyph {
            deg: p.longitude.longitude_deg() as f32,
            symbol: body_symbol(p.body).into(),
            annotation: Some(format!("{:.1}°", p.longitude.degree_in_sign_decimal())),
            retrograde: p.longitude_rate_rad_per_day < 0.0,
            house: None,
            dignity_marker: None,
        })
        .collect();
    render.layers.push(Layer {
        module_id: "transit".into(),
        kind: LayerKind::Outer,
        ring: 0.82,
        z: 4,
        geometry: Geometry::GlyphsOnly,
        glyphs: outer_glyphs,
    });

    let cross = find_synastry_aspects(
        natal,
        &transit,
        &OrbTable::modern_western(),
        EAspectKind::MAJORS,
    );
    let cross_lines: Vec<LineSeg> = cross
        .iter()
        .filter_map(|a| {
            let natal_p = natal.placement(a.person_a_body)?;
            let transit_p = transit.placement(a.person_b_body)?;
            let opacity = orb_to_opacity(a.orb_abs_deg(), a.kind);
            Some(LineSeg {
                from_deg: natal_p.longitude.longitude_deg() as f32,
                to_deg: transit_p.longitude.longitude_deg() as f32,
                kind: aspect_kind_id(a.kind).into(),
                opacity: opacity * 0.75,
                from_body: body_symbol(a.person_a_body).into(),
                to_body: body_symbol(a.person_b_body).into(),
                orb_deg: a.orb_abs_deg() as f32,
            })
        })
        .collect();
    render.layers.push(Layer {
        module_id: "transit".into(),
        kind: LayerKind::Aspects,
        ring: 0.0,
        z: 5,
        geometry: Geometry::Lines(cross_lines),
        glyphs: Vec::new(),
    });
    populate_cross_aspect_summary(&cross, "transit", render);
    Ok(())
}

/// Helper: agrega al `RenderModel` las capas del overlay de progresión
/// secundaria. La carta progresada se computa con el mismo observer y
/// config que la natal pero al instante natal+(age_years/period_years)
/// días.
/// Overlay topocéntrico: re-proyecta cada placement natal a longitud
/// topocéntrica (con paralaje horizontal) y recalcula las casas con
/// Polich-Page. Los dos quedan emparentados al mismo `module_id =
/// "topocentric"` para que el canvas los pinte con un visual
/// consistente. La capa convive con la natal geocéntrica — ambas se
/// ven simultáneamente.
fn build_topocentric_overlay(
    natal: &NatalChart,
    render: &mut RenderModel,
) -> Result<(), EngineError> {
    const KM_PER_AU: f64 = 149_597_870.7;
    let lst = natal.local_apparent_sidereal_time_rad;
    let eps = natal.obliquity_rad;
    let obs_lat = natal.birth.observer.lat_rad;

    // 1) Planetas topocéntricos. Para puntos sin distancia (nodos,
    // Lilith calculada) `topocentric_ecliptic` retorna la entrada sin
    // cambios — geocéntrico y topocéntrico coinciden ahí.
    let body_glyphs: Vec<Glyph> = natal
        .placements
        .iter()
        .map(|p| {
            let dist_au = p.distance_km / KM_PER_AU;
            let (lon_topo, _) = topocentric_ecliptic(
                p.longitude.longitude_rad(),
                p.latitude_rad,
                dist_au,
                obs_lat,
                lst,
                eps,
            );
            let lon_topo_deg = lon_topo.to_degrees() as f32;
            Glyph {
                deg: lon_topo_deg,
                symbol: body_symbol(p.body).into(),
                annotation: Some(format!("{:.2}° topo", lon_topo_deg)),
                retrograde: p.longitude_rate_rad_per_day < 0.0,
                house: None,
                dignity_marker: None,
            }
        })
        .collect();

    render.layers.push(Layer {
        module_id: "topocentric".into(),
        kind: LayerKind::Bodies,
        ring: 0.50,
        z: 8,
        geometry: Geometry::GlyphsOnly,
        glyphs: body_glyphs,
    });

    // 2) Casas Polich-Page. Si la latitud cae en el círculo polar el
    // sistema diverge — devolvemos un error parcial pero conservamos
    // la capa de planetas topocéntricos (que sí es válida).
    match EHouses::compute(EHouseSystem::PolichPage, lst, obs_lat, eps) {
        Ok(houses_pp) => {
            let cusps_deg: Vec<f32> =
                houses_pp.cusps.iter().map(|c| c.to_degrees() as f32).collect();
            let house_glyphs: Vec<Glyph> = (0..12)
                .map(|i| Glyph {
                    deg: cusps_deg[i] + 4.0,
                    symbol: format!("h{}", i + 1),
                    annotation: None,
                    retrograde: false,
                    house: Some((i as u8) + 1),
                    dignity_marker: None,
                })
                .collect();
            render.layers.push(Layer {
                module_id: "topocentric".into(),
                kind: LayerKind::Houses,
                ring: 0.78,
                z: 9,
                geometry: Geometry::Ring { cusps_deg },
                glyphs: house_glyphs,
            });
        }
        Err(e) => {
            // Polo: el visual se queda solo con planetas topocéntricos.
            eprintln!("[bridge] PolichPage no disponible en lat polar: {:?}", e);
        }
    }

    Ok(())
}

/// Orbe máximo (grados) para que una proyección primaria entre al HUD
/// de triggers. ~2° ≈ 2 años de vida con el key Naibod.
pub(crate) const GR_HUD_ORB_DEG: f32 = 2.0;
/// Micro-orbe de convergencia GR: 5 minutos de arco. Un punto natal
/// tocado a la vez por un directo y un converso dentro de este orbe
/// es un evento de rectificación.
pub(crate) const GR_EVENT_ORB_DEG: f32 = 5.0 / 60.0;
/// Tope de triggers en el HUD tras ordenar por orbe.
pub(crate) const GR_MAX_TRIGGERS: usize = 60;

/// GR dual-ring de Direcciones Primarias: a la edad pedida, cada
/// cuerpo natal se proyecta dos veces — directa (rotación diurna
/// forward, anillo afuera) y conversa (rotación inversa, anillo
/// dentro). En rectificación, los dos rings se ven simultáneamente
/// y si un evento real cayó cerca de un punto natal, debe aparecer
/// "cruzado" con ambos arcos coincidentes — eso valida la hora.
///
/// Además de los dos rings, computa `render.gr_triggers`: cada
/// proyección que cae cerca de un punto natal (cuerpo o ángulo), y
/// marca las convergencias directo+converso. La UI lo usa para el
/// HUD de rectificación y el resaltado de eventos.
///
/// Usa el key Naibod (0°59'08″/año) como default — convención GR.
fn build_primary_directions_overlay(
    natal: &NatalChart,
    target_age_years: f64,
    key: EDirectionKey,
    render: &mut RenderModel,
) {
    let eps = natal.obliquity_rad;

    let directions = [
        (GrDirection::Direct, PrimaryDirection::Direct),
        (GrDirection::Converse, PrimaryDirection::Converse),
    ];

    // Posiciones dirigidas acumuladas para el emparejamiento posterior:
    // `(promisor, dirección, longitud)`.
    let mut directed: Vec<(String, GrDirection, f32)> = Vec::new();

    for (gr_dir, pd_dir) in directions {
        let glyphs: Vec<Glyph> = natal
            .placements
            .iter()
            .map(|p| {
                let new_lon_rad = directed_longitude(
                    p.right_ascension_rad,
                    p.declination_rad,
                    target_age_years,
                    pd_dir,
                    key,
                    eps,
                );
                let directed_deg = (new_lon_rad.to_degrees() as f32).rem_euclid(360.0);
                let symbol = body_symbol(p.body);
                directed.push((symbol.to_string(), gr_dir, directed_deg));
                Glyph {
                    deg: directed_deg,
                    symbol: symbol.into(),
                    annotation: Some(format!("{:.2}°", directed_deg)),
                    retrograde: p.longitude_rate_rad_per_day < 0.0,
                    house: None,
                    dignity_marker: None,
                }
            })
            .collect();

        let (module_id, z) = match gr_dir {
            GrDirection::Direct => ("pd_direct", 10),
            GrDirection::Converse => ("pd_converse", 11),
        };
        render.layers.push(Layer {
            module_id: module_id.into(),
            kind: LayerKind::Bodies,
            ring: 0.0,
            z,
            geometry: Geometry::GlyphsOnly,
            glyphs,
        });
    }

    // Puntos natales objetivo: los cuerpos + los cuatro ángulos. Los
    // ángulos son los anclajes clave de la rectificación.
    let mut natal_targets: Vec<(String, f32)> = natal
        .placements
        .iter()
        .map(|p| {
            (
                body_symbol(p.body).to_string(),
                p.longitude.longitude_deg() as f32,
            )
        })
        .collect();
    natal_targets.push(("asc".into(), render.ascendant_deg));
    natal_targets.push(("mc".into(), render.midheaven_deg));
    natal_targets.push(("desc".into(), render.descendant_deg));
    natal_targets.push(("ic".into(), render.imum_coeli_deg));

    render.gr_triggers = compute_gr_triggers(
        &directed,
        &natal_targets,
        GR_HUD_ORB_DEG,
        GR_EVENT_ORB_DEG,
        GR_MAX_TRIGGERS,
    );
}

fn build_progression_overlay(
    natal: &NatalChart,
    target_age_years: f64,
    render: &mut RenderModel,
) -> Result<(), EngineError> {
    let session = session()?;
    let prog = secondary_progression(natal, session, target_age_years)
        .map_err(|e| EngineError::Eternal(format!("secondary_progression: {:?}", e)))?;
    let progressed = &prog.progressed;

    // Glifos de los cuerpos progresados — anillo interno (radio 0.48).
    let glyphs: Vec<Glyph> = progressed
        .placements
        .iter()
        .map(|p| Glyph {
            deg: p.longitude.longitude_deg() as f32,
            symbol: body_symbol(p.body).into(),
            annotation: Some(format!("{:.1}°", p.longitude.degree_in_sign_decimal())),
            retrograde: p.longitude_rate_rad_per_day < 0.0,
            house: Some(p.house_number),
            dignity_marker: None,
        })
        .collect();
    render.layers.push(Layer {
        module_id: "progression".into(),
        kind: LayerKind::Bodies,
        ring: 0.48,
        z: 6,
        geometry: Geometry::GlyphsOnly,
        glyphs,
    });

    // Cross aspects natal × progresada (sólo mayores).
    let cross = find_synastry_aspects(
        natal,
        progressed,
        &OrbTable::modern_western(),
        EAspectKind::MAJORS,
    );
    let cross_lines: Vec<LineSeg> = cross
        .iter()
        .filter_map(|a| {
            let natal_p = natal.placement(a.person_a_body)?;
            let prog_p = progressed.placement(a.person_b_body)?;
            let opacity = orb_to_opacity(a.orb_abs_deg(), a.kind);
            Some(LineSeg {
                from_deg: natal_p.longitude.longitude_deg() as f32,
                to_deg: prog_p.longitude.longitude_deg() as f32,
                kind: aspect_kind_id(a.kind).into(),
                opacity: opacity * 0.7,
                from_body: body_symbol(a.person_a_body).into(),
                to_body: body_symbol(a.person_b_body).into(),
                orb_deg: a.orb_abs_deg() as f32,
            })
        })
        .collect();
    render.layers.push(Layer {
        module_id: "progression".into(),
        kind: LayerKind::Aspects,
        ring: 0.0,
        z: 7,
        geometry: Geometry::Lines(cross_lines),
        glyphs: Vec::new(),
    });
    populate_cross_aspect_summary(&cross, "progression", render);
    Ok(())
}

/// Helper: detecta "ejes" del dial uraniano de 90° — cuerpos natales
/// cuya longitud módulo 90 cae dentro de una tolerancia ε (2° por
/// default). Llena `render.uranian_groups` con los grupos detectados.
fn build_uranian_groups(natal: &NatalChart, render: &mut RenderModel) {
    const EPSILON: f64 = 2.0;
    let mut entries: Vec<(String, f64)> = natal
        .placements
        .iter()
        .map(|p| {
            let lon = p.longitude.longitude_deg();
            let mod90 = lon.rem_euclid(90.0);
            (body_symbol(p.body).to_string(), mod90)
        })
        .collect();
    entries.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut groups: Vec<UranianGroup> = Vec::new();
    let mut current: Vec<(String, f64)> = Vec::new();
    let mut anchor_mod90 = 0.0_f64;
    for entry in entries {
        if current.is_empty() {
            anchor_mod90 = entry.1;
            current.push(entry);
            continue;
        }
        // Distancia circular módulo 90 entre el entry y el anchor.
        let mut diff = (entry.1 - anchor_mod90).abs();
        if diff > 45.0 {
            diff = 90.0 - diff;
        }
        if diff <= EPSILON {
            current.push(entry);
        } else {
            if current.len() >= 2 {
                let center = current.iter().map(|(_, m)| *m).sum::<f64>() / current.len() as f64;
                groups.push(UranianGroup {
                    bodies: current.iter().map(|(s, _)| s.clone()).collect(),
                    mod90_deg: center,
                });
            }
            anchor_mod90 = entry.1;
            current = vec![entry];
        }
    }
    if current.len() >= 2 {
        let center = current.iter().map(|(_, m)| *m).sum::<f64>() / current.len() as f64;
        groups.push(UranianGroup {
            bodies: current.iter().map(|(s, _)| s.clone()).collect(),
            mod90_deg: center,
        });
    }
    // Wrap-around check: el primer y último grupo podrían ser el mismo
    // (si span >88° abarcando el wrap en 90/0). Si los anchors están a
    // ≤EPSILON modulo 90, mergeamos.
    if groups.len() >= 2 {
        let first_mod = groups[0].mod90_deg;
        let last_mod = groups[groups.len() - 1].mod90_deg;
        let mut diff = (first_mod - last_mod).abs();
        if diff > 45.0 {
            diff = 90.0 - diff;
        }
        if diff <= EPSILON {
            let last = groups.pop().unwrap();
            groups[0].bodies.extend(last.bodies);
        }
    }
    render.uranian_groups = groups;
}

/// Helper: agrega al `RenderModel` la carta compuesta (midpoint
/// composite, Davison 1958) entre la natal del sujeto y la carta del
/// partner. Cada planeta compuesto es el angular midpoint entre los
/// dos correspondientes. Se renderea en `radii.composite` (ring 0.36).
fn build_composite_overlay(
    natal: &NatalChart,
    partner_chart: &Chart,
    render: &mut RenderModel,
) -> Result<(), EngineError> {
    let (partner_natal, _config, _observer) = compute_natal_chart(partner_chart, 0)?;
    let comp = composite(natal, &partner_natal).map_err(|e| {
        EngineError::Eternal(format!("composite: {:?}", e))
    })?;

    let glyphs: Vec<Glyph> = comp
        .placements
        .iter()
        .map(|p| Glyph {
            deg: p.longitude.longitude_deg() as f32,
            symbol: body_symbol(p.body).into(),
            annotation: Some(format!("composite {}", p.body.name())),
            retrograde: false,
            house: Some(p.house_number),
            dignity_marker: None,
        })
        .collect();
    render.layers.push(Layer {
        module_id: "composite".into(),
        kind: LayerKind::Bodies,
        ring: 0.36,
        z: 15,
        geometry: Geometry::GlyphsOnly,
        glyphs,
    });
    Ok(())
}

/// Helper: agrega al `RenderModel` los midpoints entre pares de
/// cuerpos natales. Filtra para mostrar solo los que involucran al
/// Sol o a la Luna (~10 puntos) — son los más significativos
/// astrológicamente y mantiene la rueda legible.
///
/// El midpoint de dos longitudes es la menor distancia angular entre
/// ellas. Si `|a - b| > 180`, hay que sumar 180 al promedio para
/// obtener el midpoint "corto".
fn build_midpoints_overlay(natal: &NatalChart, render: &mut RenderModel) {
    let mut glyphs: Vec<Glyph> = Vec::new();
    let placements = &natal.placements;

    for i in 0..placements.len() {
        for j in (i + 1)..placements.len() {
            let pa = &placements[i];
            let pb = &placements[j];
            // Solo midpoints que involucren Sol o Luna.
            let involves_luminary = matches!(pa.body, Body::Sun | Body::Moon)
                || matches!(pb.body, Body::Sun | Body::Moon);
            if !involves_luminary {
                continue;
            }
            let a = pa.longitude.longitude_deg() as f32;
            let b = pb.longitude.longitude_deg() as f32;
            let diff = (a - b).abs();
            let mid = if diff > 180.0 {
                ((a + b) / 2.0 + 180.0).rem_euclid(360.0)
            } else {
                ((a + b) / 2.0).rem_euclid(360.0)
            };
            glyphs.push(Glyph {
                deg: mid,
                symbol: format!("{}/{}", body_symbol(pa.body), body_symbol(pb.body)),
                annotation: Some(format!("{}/{}", pa.body.name(), pb.body.name())),
                retrograde: false,
                house: None,
                dignity_marker: None,
            });
        }
    }

    render.layers.push(Layer {
        module_id: "midpoints".into(),
        kind: LayerKind::Midpoints,
        ring: 0.62,
        z: 14,
        geometry: Geometry::GlyphsOnly,
        glyphs,
    });
}

/// Helper: agrega al `RenderModel` las capas del overlay de Solar Arc
/// (método true-progressed-Sun por default). Cada cuerpo natal se
/// desplaza por el mismo arco — preserva las relaciones angulares y
/// las posiciones relativas en casas se mantienen.
fn build_solar_arc_overlay(
    natal: &NatalChart,
    target_age_years: f64,
    render: &mut RenderModel,
) -> Result<(), EngineError> {
    let session = session()?;
    let arc = solar_arc_true(natal, session, target_age_years)
        .map_err(|e| EngineError::Eternal(format!("solar_arc_true: {:?}", e)))?;
    let directed = &arc.directed;

    let glyphs: Vec<Glyph> = directed
        .placements
        .iter()
        .map(|p| Glyph {
            deg: p.longitude.longitude_deg() as f32,
            symbol: body_symbol(p.body).into(),
            annotation: Some(format!("{:.1}°", p.longitude.degree_in_sign_decimal())),
            retrograde: p.longitude_rate_rad_per_day < 0.0,
            house: Some(p.house_number),
            dignity_marker: None,
        })
        .collect();
    render.layers.push(Layer {
        module_id: "solar_arc".into(),
        kind: LayerKind::Bodies,
        ring: 0.43,
        z: 8,
        geometry: Geometry::GlyphsOnly,
        glyphs,
    });

    let cross = find_synastry_aspects(
        natal,
        directed,
        &OrbTable::modern_western(),
        EAspectKind::MAJORS,
    );
    let cross_lines: Vec<LineSeg> = cross
        .iter()
        .filter_map(|a| {
            let natal_p = natal.placement(a.person_a_body)?;
            let dir_p = directed.placement(a.person_b_body)?;
            let opacity = orb_to_opacity(a.orb_abs_deg(), a.kind);
            Some(LineSeg {
                from_deg: natal_p.longitude.longitude_deg() as f32,
                to_deg: dir_p.longitude.longitude_deg() as f32,
                kind: aspect_kind_id(a.kind).into(),
                opacity: opacity * 0.7,
                from_body: body_symbol(a.person_a_body).into(),
                to_body: body_symbol(a.person_b_body).into(),
                orb_deg: a.orb_abs_deg() as f32,
            })
        })
        .collect();
    render.layers.push(Layer {
        module_id: "solar_arc".into(),
        kind: LayerKind::Aspects,
        ring: 0.0,
        z: 9,
        geometry: Geometry::Lines(cross_lines),
        glyphs: Vec::new(),
    });
    populate_cross_aspect_summary(&cross, "solar_arc", render);
    Ok(())
}

/// Helper: agrega al `RenderModel` las capas del overlay de sinastría
/// con otra carta natal completa. La carta partner se computa con su
/// propio observer/config (no comparte con la natal). El outer ring
/// se comparte con Transit — mutuamente excluyentes a nivel de Shell.
fn build_synastry_overlay(
    natal: &NatalChart,
    partner_chart: &Chart,
    render: &mut RenderModel,
) -> Result<(), EngineError> {
    let (partner, _config, _observer) = compute_natal_chart(partner_chart, 0)?;

    let glyphs: Vec<Glyph> = partner
        .placements
        .iter()
        .map(|p| Glyph {
            deg: p.longitude.longitude_deg() as f32,
            symbol: body_symbol(p.body).into(),
            annotation: Some(format!("{:.1}°", p.longitude.degree_in_sign_decimal())),
            retrograde: p.longitude_rate_rad_per_day < 0.0,
            house: Some(p.house_number),
            dignity_marker: None,
        })
        .collect();
    render.layers.push(Layer {
        module_id: "synastry".into(),
        kind: LayerKind::Outer,
        ring: 0.82,
        z: 10,
        geometry: Geometry::GlyphsOnly,
        glyphs,
    });

    let cross = find_synastry_aspects(
        natal,
        &partner,
        &OrbTable::modern_western(),
        EAspectKind::MAJORS,
    );
    let cross_lines: Vec<LineSeg> = cross
        .iter()
        .filter_map(|a| {
            let natal_p = natal.placement(a.person_a_body)?;
            let partner_p = partner.placement(a.person_b_body)?;
            let opacity = orb_to_opacity(a.orb_abs_deg(), a.kind);
            Some(LineSeg {
                from_deg: natal_p.longitude.longitude_deg() as f32,
                to_deg: partner_p.longitude.longitude_deg() as f32,
                kind: aspect_kind_id(a.kind).into(),
                opacity: opacity * 0.85,
                from_body: body_symbol(a.person_a_body).into(),
                to_body: body_symbol(a.person_b_body).into(),
                orb_deg: a.orb_abs_deg() as f32,
            })
        })
        .collect();
    render.layers.push(Layer {
        module_id: "synastry".into(),
        kind: LayerKind::Aspects,
        ring: 0.0,
        z: 11,
        geometry: Geometry::Lines(cross_lines),
        glyphs: Vec::new(),
    });
    populate_cross_aspect_summary(&cross, "synastry", render);
    Ok(())
}

/// Helper: agrega al `RenderModel` las capas del overlay de retorno
/// planetario — la carta natal completa computada al instante en que
/// el `body` vuelve a su posición natal cerca de la edad pedida.
/// Sun = retorno solar anual, Moon = mensual, Júpiter/Saturno =
/// generacionales. Esa nueva carta va en el anillo externo (compartido
/// con Transit/Synastry, mutuamente excluyentes a nivel de Shell).
/// Computa la carta del retorno planetario actual y devuelve los
/// datos necesarios para construir un `Chart` standalone que el
/// caller puede mostrar/persistir.
///
/// Devuelve `(StoredBirthData, instant_label)`:
/// - `StoredBirthData` con birth_data del retorno (year/month/day/...
///   del instante del retorno, mismas coordenadas que el natal).
/// - `instant_label` formato corto del momento (ej. "2024-03-14
///   05:22 UTC") — el shell lo concatena en el label final.
pub fn compute_planetary_return_chart(
    chart: &Chart,
    body_str: &str,
    target_age_years: f64,
    shift_days: i64,
) -> Result<(cosmobiologia_model::StoredBirthData, String), EngineError> {
    let (birth_e, config_e, _observer) = build_eternal_inputs(chart, 0)?;
    let session = session()?;
    let natal = NatalChart::compute(&birth_e, &config_e, session)
        .map_err(|e| EngineError::Eternal(format!("NatalChart::compute: {:?}", e)))?;
    let body = map_body(body_str)
        .ok_or_else(|| EngineError::Eternal(format!("body desconocido: {}", body_str)))?;
    let natal_p = natal.placement(body).ok_or_else(|| {
        EngineError::Eternal(format!(
            "natal chart sin {} — return imposible",
            body.name()
        ))
    })?;
    let natal_lon = natal_p.longitude.longitude_rad();

    let after_seconds =
        (target_age_years * 365.242190 - 30.0 + shift_days as f64) * 86400.0;
    const TWO_TROPICAL: f64 = 365.242190 * 86400.0 * 2.0;
    let after_utc = natal
        .birth
        .instant
        .utc()
        .add_seconds(after_seconds.max(-TWO_TROPICAL));
    let after = ESInstant::from_utc(after_utc);

    let return_instant = next_return(session, body, natal_lon, after, None)
        .map_err(|e| EngineError::Eternal(format!("next_return {}: {:?}", body.name(), e)))?;

    // Extraer year/month/day/hour/min/sec del momento del retorno.
    // `to_iso8601` devuelve "YYYY-MM-DDTHH:MM:SS.sss" — parseamos los
    // 5 campos relevantes. La precisión está en sub-segundo; usamos
    // segundo entero (StoredBirthData::second es f64 pero el campo
    // se persiste así).
    let iso = return_instant.utc().to_iso8601();
    let (year, month, day, hour, minute, second) = parse_iso8601_components(&iso)
        .ok_or_else(|| EngineError::Eternal(format!("iso8601 inválido: {}", iso)))?;

    let stored = cosmobiologia_model::StoredBirthData {
        year,
        month,
        day,
        hour,
        minute,
        second,
        // El return se computa en la TZ del observador natal (es la
        // convención clásica del Solar return). Heredamos también
        // lat/lon/alt.
        tz_offset_minutes: chart.birth_data.tz_offset_minutes,
        latitude_deg: chart.birth_data.latitude_deg,
        longitude_deg: chart.birth_data.longitude_deg,
        altitude_m: chart.birth_data.altitude_m,
        time_certainty: Default::default(),
        subject_name: chart.birth_data.subject_name.clone(),
        birthplace_label: chart.birth_data.birthplace_label.clone(),
    };
    let label = format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        year, month, day, hour, minute
    );
    Ok((stored, label))
}

/// Computa la **carta de tránsito** del momento actual sobre las
/// coordenadas del natal — birth_data = "ahora" UTC, mismo
/// observer/lat/lon/TZ que el natal. Útil para snapshot del cielo
/// en este instante anclado al lugar de nacimiento del sujeto.
pub fn compute_transit_chart(
    chart: &Chart,
) -> Result<(cosmobiologia_model::StoredBirthData, String), EngineError> {
    let now_iso = ESInstant::now().utc().to_iso8601();
    let (year, month, day, hour, minute, second) =
        parse_iso8601_components(&now_iso).ok_or_else(|| {
            EngineError::Eternal(format!("iso8601 inválido para now(): {}", now_iso))
        })?;
    let stored = cosmobiologia_model::StoredBirthData {
        year,
        month,
        day,
        hour,
        minute,
        second,
        tz_offset_minutes: chart.birth_data.tz_offset_minutes,
        latitude_deg: chart.birth_data.latitude_deg,
        longitude_deg: chart.birth_data.longitude_deg,
        altitude_m: chart.birth_data.altitude_m,
        time_certainty: Default::default(),
        subject_name: chart.birth_data.subject_name.clone(),
        birthplace_label: chart.birth_data.birthplace_label.clone(),
    };
    let label = format!("{:04}-{:02}-{:02} {:02}:{:02} UTC", year, month, day, hour, minute);
    Ok((stored, label))
}

/// Computa la **carta progresada secundaria** a la edad dada como
/// `StoredBirthData` standalone. Método clásico: el instante de la
/// progresada es `natal_instant + target_age_years * 1 día`
/// (un día simbólico = un año de vida). Las coordenadas del
/// observador se heredan del natal — la progresada es una proyección
/// simbólica sobre el lugar de nacimiento, no un evento real ahí.
pub fn compute_progression_chart(
    chart: &Chart,
    target_age_years: f64,
) -> Result<(cosmobiologia_model::StoredBirthData, String), EngineError> {
    let (birth_e, _config_e, _observer) = build_eternal_inputs(chart, 0)?;
    let advance_seconds = target_age_years * 86400.0; // 1 día / año
    let advanced_utc = birth_e.instant.utc().add_seconds(advance_seconds);
    let iso = advanced_utc.to_iso8601();
    let (year, month, day, hour, minute, second) =
        parse_iso8601_components(&iso).ok_or_else(|| {
            EngineError::Eternal(format!("iso8601 inválido: {}", iso))
        })?;
    let stored = cosmobiologia_model::StoredBirthData {
        year,
        month,
        day,
        hour,
        minute,
        second,
        tz_offset_minutes: chart.birth_data.tz_offset_minutes,
        latitude_deg: chart.birth_data.latitude_deg,
        longitude_deg: chart.birth_data.longitude_deg,
        altitude_m: chart.birth_data.altitude_m,
        time_certainty: Default::default(),
        subject_name: chart.birth_data.subject_name.clone(),
        birthplace_label: chart.birth_data.birthplace_label.clone(),
    };
    let label = format!("{:04}-{:02}-{:02} {:02}:{:02} UTC", year, month, day, hour, minute);
    Ok((stored, label))
}

/// Parsea "YYYY-MM-DDTHH:MM:SS[.fff]" a `(year, month, day, hour,
/// minute, second_float)`. Retorna `None` si el formato no encaja.
fn parse_iso8601_components(s: &str) -> Option<(i32, u32, u32, u32, u32, f64)> {
    // Split en T y luego campo por campo.
    let mut parts = s.splitn(2, 'T');
    let date = parts.next()?;
    let time = parts.next()?;
    let mut d = date.split('-');
    let year: i32 = d.next()?.parse().ok()?;
    let month: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    let mut t = time.split(':');
    let hour: u32 = t.next()?.parse().ok()?;
    let minute: u32 = t.next()?.parse().ok()?;
    let second: f64 = t.next()?.parse().ok()?;
    Some((year, month, day, hour, minute, second))
}

/// Cross aspects natal × return.
fn build_planetary_return_overlay(
    natal: &NatalChart,
    config_e: &ChartConfig,
    observer: Observer,
    body: Body,
    target_age_years: f64,
    shift_days: i64,
    render: &mut RenderModel,
) -> Result<(), EngineError> {
    let session = session()?;
    let natal_p = natal.placement(body).ok_or_else(|| {
        EngineError::Eternal(format!(
            "natal chart sin {} — return imposible",
            body.name()
        ))
    })?;
    let natal_lon = natal_p.longitude.longitude_rad();

    // El offset desde el cumpleaños depende del período sinódico del
    // cuerpo: para Sun/planet lentos, ~30 días antes garantiza captar
    // el return; para Moon, ~15 días. Tomamos un margen amplio que
    // sirve para todos.
    const TROPICAL_YEAR_SECS: f64 = 365.242190 * 86400.0;
    // shift_days permite saltar de un retorno mensual al siguiente
    // cuando body=Moon, o ajustar finamente el año en Solar return.
    let after_seconds =
        (target_age_years * 365.242190 - 30.0 + shift_days as f64) * 86400.0;
    let after_utc = natal
        .birth
        .instant
        .utc()
        .add_seconds(after_seconds.max(-TROPICAL_YEAR_SECS * 2.0));
    let after = ESInstant::from_utc(after_utc);

    let return_instant = next_return(session, body, natal_lon, after, None).map_err(|e| {
        EngineError::Eternal(format!("next_return {}: {:?}", body.name(), e))
    })?;

    // La carta del retorno se computa al return_instant con el mismo
    // observer y config natales (convención clásica: return tropical
    // en la ciudad de nacimiento).
    let return_birth = BirthData::new(return_instant, observer);
    let return_chart = NatalChart::compute(&return_birth, config_e, session).map_err(|e| {
        EngineError::Eternal(format!(
            "NatalChart::compute ({} return): {:?}",
            body.name(),
            e
        ))
    })?;

    let glyphs: Vec<Glyph> = return_chart
        .placements
        .iter()
        .map(|p| Glyph {
            deg: p.longitude.longitude_deg() as f32,
            symbol: body_symbol(p.body).into(),
            annotation: Some(format!("{:.1}°", p.longitude.degree_in_sign_decimal())),
            retrograde: p.longitude_rate_rad_per_day < 0.0,
            house: Some(p.house_number),
            dignity_marker: None,
        })
        .collect();
    render.layers.push(Layer {
        module_id: "planetary_return".into(),
        kind: LayerKind::Outer,
        ring: 0.82,
        z: 12,
        geometry: Geometry::GlyphsOnly,
        glyphs,
    });

    let cross = find_synastry_aspects(
        natal,
        &return_chart,
        &OrbTable::modern_western(),
        EAspectKind::MAJORS,
    );
    let cross_lines: Vec<LineSeg> = cross
        .iter()
        .filter_map(|a| {
            let n_p = natal.placement(a.person_a_body)?;
            let r_p = return_chart.placement(a.person_b_body)?;
            let opacity = orb_to_opacity(a.orb_abs_deg(), a.kind);
            Some(LineSeg {
                from_deg: n_p.longitude.longitude_deg() as f32,
                to_deg: r_p.longitude.longitude_deg() as f32,
                kind: aspect_kind_id(a.kind).into(),
                opacity: opacity * 0.8,
                from_body: body_symbol(a.person_a_body).into(),
                to_body: body_symbol(a.person_b_body).into(),
                orb_deg: a.orb_abs_deg() as f32,
            })
        })
        .collect();
    render.layers.push(Layer {
        module_id: "planetary_return".into(),
        kind: LayerKind::Aspects,
        ring: 0.0,
        z: 13,
        geometry: Geometry::Lines(cross_lines),
        glyphs: Vec::new(),
    });
    populate_cross_aspect_summary(&cross, "planetary_return", render);
    Ok(())
}

// =====================================================================
// NatalChart → RenderModel
// =====================================================================

fn build_render_model(
    chart: &Chart,
    natal: &NatalChart,
    aspects: &[Aspect],
    started: Instant,
) -> RenderModel {
    let ascendant_deg = natal.ascendant().longitude_deg() as f32;
    let midheaven_deg = natal.midheaven().longitude_deg() as f32;
    let descendant_deg = natal.descendant().longitude_deg() as f32;
    let imum_coeli_deg = natal.imum_coeli().longitude_deg() as f32;

    // ─── Capa 0: Sign Dial ────────────────────────────────────────────
    let sign_dial = Layer {
        module_id: "natal".into(),
        kind: LayerKind::SignDial,
        ring: 1.0,
        z: 0,
        geometry: Geometry::Ring {
            cusps_deg: (0..12).map(|i| (i as f32) * 30.0).collect(),
        },
        glyphs: (0..12)
            .map(|i| Glyph {
                deg: (i as f32) * 30.0 + 15.0,
                symbol: ZODIAC_SYMBOLS[i].into(),
                annotation: None,
                retrograde: false,
                house: None,
            dignity_marker: None,
            })
            .collect(),
    };

    // ─── Capa 1: Houses ───────────────────────────────────────────────
    let cusps_deg: Vec<f32> = natal
        .houses
        .cusps
        .iter()
        .map(|c| c.to_degrees() as f32)
        .collect();
    let houses = Layer {
        module_id: "natal".into(),
        kind: LayerKind::Houses,
        ring: 0.86,
        z: 1,
        geometry: Geometry::Ring {
            cusps_deg: cusps_deg.clone(),
        },
        glyphs: cusps_deg
            .iter()
            .enumerate()
            .map(|(i, c)| Glyph {
                deg: *c + 4.0,
                symbol: format!("h{}", i + 1),
                annotation: None,
                retrograde: false,
                house: Some((i as u8) + 1),
                dignity_marker: None,
            })
            .collect(),
    };

    // ─── Capa 2: Bodies ───────────────────────────────────────────────
    let body_glyphs: Vec<Glyph> = natal
        .placements
        .iter()
        .map(|p| Glyph {
            deg: p.longitude.longitude_deg() as f32,
            symbol: body_symbol(p.body).into(),
            annotation: Some(format!("{:.1}°", p.longitude.degree_in_sign_decimal())),
            // `BodyPlacement` cambió entre versiones de eternal entre
            // `pub fn is_retrograde(&self) -> bool` y `pub
            // is_retrograde: bool` — leemos el campo crudo
            // `longitude_rate_rad_per_day` (estable en ambas) para no
            // depender del wrapper.
            retrograde: p.longitude_rate_rad_per_day < 0.0,
            house: Some(p.house_number),
            dignity_marker: None,
        })
        .collect();
    let bodies = Layer {
        module_id: "natal".into(),
        kind: LayerKind::Bodies,
        ring: 0.72,
        z: 2,
        geometry: Geometry::Points(
            natal
                .placements
                .iter()
                .map(|p| crate::PointMark {
                    deg: p.longitude.longitude_deg() as f32,
                    label: p.body.name().into(),
                    tag: body_symbol(p.body).into(),
                })
                .collect(),
        ),
        glyphs: body_glyphs,
    };

    // ─── Capa 3: Aspects ──────────────────────────────────────────────
    // Los aspects ya vienen filtrados por NatalOptions (majors / minors)
    // desde compose(). Acá solo mapeamos a LineSeg.
    let mut aspect_lines: Vec<LineSeg> = Vec::with_capacity(aspects.len());
    for a in aspects {
        let pa = natal.placement(a.a);
        let pb = natal.placement(a.b);
        if let (Some(pa), Some(pb)) = (pa, pb) {
            let opacity = orb_to_opacity(a.orb_abs_deg(), a.kind);
            aspect_lines.push(LineSeg {
                from_deg: pa.longitude.longitude_deg() as f32,
                to_deg: pb.longitude.longitude_deg() as f32,
                kind: aspect_kind_id(a.kind).into(),
                opacity,
                from_body: body_symbol(a.a).into(),
                to_body: body_symbol(a.b).into(),
                orb_deg: a.orb_abs_deg() as f32,
            });
        }
    }
    let aspects_layer = Layer {
        module_id: "natal".into(),
        kind: LayerKind::Aspects,
        ring: 0.58,
        z: 3,
        geometry: Geometry::Lines(aspect_lines),
        glyphs: Vec::new(),
    };

    let subtitle = chart
        .birth_data
        .birthplace_label
        .clone()
        .or_else(|| {
            Some(format!(
                "{:04}-{:02}-{:02} · lat {:+.2}° · lon {:+.2}°",
                chart.birth_data.year,
                chart.birth_data.month,
                chart.birth_data.day,
                chart.birth_data.latitude_deg,
                chart.birth_data.longitude_deg,
            ))
        });

    RenderModel {
        chart_id: chart.id,
        chart_kind: chart.kind,
        title: chart.label.clone(),
        subtitle,
        compute_ms: started.elapsed().as_millis() as u64,
        ascendant_deg,
        midheaven_deg,
        descendant_deg,
        imum_coeli_deg,
        geo_latitude_deg: chart.birth_data.latitude_deg as f32,
        layers: vec![sign_dial, houses, bodies, aspects_layer],
        overlays: Vec::new(),
        aspect_summary: Vec::new(),
        uranian_groups: Vec::new(),
        gr_triggers: Vec::new(),
        harmonic: 1,
        harmonic_spectrum: Vec::new(),
    }
}

/// Construye una `OrbTable` con los orbes default de `modern_western`
/// escalados por `multiplier`. Necesario porque eternal expone
/// `set_orb` pero no permite iterar los base orbs internos.
fn build_orb_table(multiplier: f64) -> OrbTable {
    let mut t = OrbTable::modern_western();
    let m = multiplier.max(0.0);
    t.set_orb(EAspectKind::Conjunction, 8.0 * m);
    t.set_orb(EAspectKind::Opposition, 8.0 * m);
    t.set_orb(EAspectKind::Trine, 7.0 * m);
    t.set_orb(EAspectKind::Square, 7.0 * m);
    t.set_orb(EAspectKind::Sextile, 5.0 * m);
    t.set_orb(EAspectKind::Quincunx, 2.5 * m);
    t.set_orb(EAspectKind::SemiSextile, 2.0 * m);
    t.set_orb(EAspectKind::SemiSquare, 2.0 * m);
    t.set_orb(EAspectKind::Sesquiquadrate, 2.0 * m);
    t.set_orb(EAspectKind::Quintile, 1.5 * m);
    t.set_orb(EAspectKind::BiQuintile, 1.5 * m);
    t.set_orb(EAspectKind::Septile, 1.5 * m);
    t
}

fn push_overlay_meta(render: &mut RenderModel, module_id: &str, label: String) {
    render.overlays.push(OverlayMeta {
        module_id: module_id.to_string(),
        label,
    });
}

/// Helper: agrega al `RenderModel` los Lots arábigos clásicos
/// (helenísticos) — Fortune, Spirit, Eros, Necessity, Courage, Victory,
/// Nemesis. Cada uno se renderea como un glifo `lot:Fo` en el anillo
/// `0.54` (entre midpoints y cuerpos progresados). Retorna la cantidad
/// de lots renderizados.
fn build_lots_overlay(
    natal: &NatalChart,
    render: &mut RenderModel,
) -> Result<usize, EngineError> {
    let lots = all_lots(natal)
        .map_err(|e| EngineError::Eternal(format!("all_lots: {:?}", e)))?;
    let glyphs: Vec<Glyph> = lots
        .iter()
        .map(|l| {
            let name = l.name.map(|n| n.label()).unwrap_or("Lot");
            // Tres-letras compactas para no recargar la rueda.
            let abbrev: String = name.chars().take(2).collect();
            Glyph {
                deg: l.longitude.longitude_deg() as f32,
                symbol: format!("lot:{}", abbrev),
                annotation: Some(name.to_string()),
                retrograde: false,
                house: Some(l.house_number),
                dignity_marker: None,
            }
        })
        .collect();
    let count = glyphs.len();
    render.layers.push(Layer {
        module_id: "lots".into(),
        kind: LayerKind::Lots,
        ring: 0.54,
        z: 13,
        geometry: Geometry::GlyphsOnly,
        glyphs,
    });
    Ok(count)
}

/// Helper: agrega al `RenderModel` 9 estrellas fijas notables. Las
/// longitudes están en J2000 ecliptica tropical; aplicamos precesión
/// general de 50.29″/año hacia adelante hasta el año natal — basta
/// para el orbe de conjunción de ±1.5° con que se interpretan.
fn build_fixed_stars_overlay(chart: &Chart, render: &mut RenderModel) -> usize {
    // (símbolo, nombre, longitud tropical J2000 en grados)
    const STARS: &[(&str, &str, f64)] = &[
        ("✦Ald", "Aldebaran", 69.79),     // 09°47′ Gem
        ("✦Reg", "Regulus", 149.83),      // 29°50′ Leo
        ("✦Ant", "Antares", 249.77),      // 09°46′ Sag
        ("✦Fom", "Fomalhaut", 333.87),    // 03°52′ Pis
        ("✦Spi", "Spica", 203.84),        // 23°50′ Lib
        ("✦Sir", "Sirius", 104.10),       // 14°06′ Can
        ("✦Alg", "Algol", 56.18),         // 26°10′ Tau
        ("✦Veg", "Vega", 285.31),         // 15°19′ Cap
        ("✦Pol", "Pollux", 113.27),       // 23°16′ Can
    ];
    let years_from_j2000 = (chart.birth_data.year - 2000) as f64;
    // 50.29″/año ≈ 0.01397°/año de precesión en longitud eclíptica.
    let precession_deg = years_from_j2000 * (50.29 / 3600.0);
    let glyphs: Vec<Glyph> = STARS
        .iter()
        .map(|(sym, name, j2000_deg)| {
            let lon = (j2000_deg + precession_deg).rem_euclid(360.0) as f32;
            Glyph {
                deg: lon,
                symbol: (*sym).to_string(),
                annotation: Some((*name).to_string()),
                retrograde: false,
                house: None,
                dignity_marker: None,
            }
        })
        .collect();
    let count = glyphs.len();
    render.layers.push(Layer {
        module_id: "fixed_stars".into(),
        kind: LayerKind::FixedStars,
        ring: 1.04,
        z: 16,
        geometry: Geometry::GlyphsOnly,
        glyphs,
    });
    count
}

/// Decora cada Glyph de Bodies (module_id="natal") con su dignity
/// marker en `glyph.dignity_marker`. Usa `essential_dignity(body, sign)`
/// — los cuerpos modernos quedan sin marker.
fn annotate_dignities(natal: &NatalChart, render: &mut RenderModel) {
    use std::collections::HashMap;
    let mut by_symbol: HashMap<&'static str, &'static str> = HashMap::new();
    for p in &natal.placements {
        let sign_idx = (p.longitude.longitude_deg() / 30.0).floor() as u8 % 12;
        if let Some(d) = essential_dignity(p.body, sign_idx) {
            by_symbol.insert(body_symbol(p.body), d.marker());
        }
    }
    for layer in render.layers.iter_mut() {
        if matches!(layer.kind, LayerKind::Bodies) && layer.module_id == "natal" {
            for g in layer.glyphs.iter_mut() {
                if let Some(marker) = by_symbol.get(g.symbol.as_str()) {
                    g.dignity_marker = Some((*marker).to_string());
                }
            }
        }
    }
}

fn populate_natal_aspect_summary(aspects: &[Aspect], render: &mut RenderModel) {
    for a in aspects {
        render.aspect_summary.push(AspectSummary {
            module_id: "natal".into(),
            from_body: body_symbol(a.a).into(),
            to_body: body_symbol(a.b).into(),
            kind: aspect_kind_id(a.kind).into(),
            orb_deg: a.orb_abs_deg(),
            applying: Some(a.applying),
        });
    }
    sort_aspect_summary(render);
}

fn populate_cross_aspect_summary(
    cross: &[eternal_astrology::SynastryAspect],
    module_id: &str,
    render: &mut RenderModel,
) {
    for a in cross {
        render.aspect_summary.push(AspectSummary {
            module_id: module_id.to_string(),
            from_body: body_symbol(a.person_a_body).into(),
            to_body: body_symbol(a.person_b_body).into(),
            kind: aspect_kind_id(a.kind).into(),
            orb_deg: a.orb_abs_deg(),
            applying: None,
        });
    }
    sort_aspect_summary(render);
}

fn sort_aspect_summary(render: &mut RenderModel) {
    render
        .aspect_summary
        .sort_by(|x, y| x.orb_deg.partial_cmp(&y.orb_deg).unwrap_or(std::cmp::Ordering::Equal));
}

/// Mapea el orb absoluto a una opacidad — los aspectos más exactos se
/// pintan más fuerte, los flojos casi se desvanecen.
fn orb_to_opacity(orb_deg: f64, kind: EAspectKind) -> f32 {
    let max = match kind {
        EAspectKind::Conjunction | EAspectKind::Opposition => 8.0,
        EAspectKind::Trine | EAspectKind::Square => 7.0,
        EAspectKind::Sextile => 5.0,
        _ => 3.0,
    };
    let t = (1.0 - (orb_deg / max).min(1.0)).max(0.25);
    t as f32
}

const ZODIAC_SYMBOLS: [&str; 12] = [
    "aries",
    "taurus",
    "gemini",
    "cancer",
    "leo",
    "virgo",
    "libra",
    "scorpio",
    "sagittarius",
    "capricorn",
    "aquarius",
    "pisces",
];
