//! Sesión global de efemérides + cómputo de cartas (natal/compose/retornos).

use super::*;

// =====================================================================
// Sesión global cacheada
// =====================================================================

pub(crate) static SESSION: OnceLock<EphemerisSession> = OnceLock::new();

pub(crate) fn session() -> Result<&'static EphemerisSession, EngineError> {
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
// compute()
// =====================================================================

/// Construye los tipos eternales (`BirthData`, `ChartConfig`) desde el
/// `Chart` agnóstico, aplicando el offset temporal. Devuelve también el
/// `Observer` y la `ChartConfig` para reusar en pipelines extendidas
/// (transits, sinastría) sin re-traducir.
pub(crate) fn build_eternal_inputs(
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
                build_topocentric_overlay(&natal, natal_options.show_minors, &mut render)?;
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
/// - `instant_label` format corto del momento (ej. "2024-03-14
///   05:22 UTC") — el shell lo concatena en el label final.
pub fn compute_planetary_return_chart(
    chart: &Chart,
    body_str: &str,
    target_age_years: f64,
    shift_days: i64,
) -> Result<(cosmos_model::StoredBirthData, String), EngineError> {
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

    let stored = cosmos_model::StoredBirthData {
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
) -> Result<(cosmos_model::StoredBirthData, String), EngineError> {
    let now_iso = ESInstant::now().utc().to_iso8601();
    let (year, month, day, hour, minute, second) =
        parse_iso8601_components(&now_iso).ok_or_else(|| {
            EngineError::Eternal(format!("iso8601 inválido para now(): {}", now_iso))
        })?;
    let stored = cosmos_model::StoredBirthData {
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
) -> Result<(cosmos_model::StoredBirthData, String), EngineError> {
    let (birth_e, _config_e, _observer) = build_eternal_inputs(chart, 0)?;
    let advance_seconds = target_age_years * 86400.0; // 1 día / año
    let advanced_utc = birth_e.instant.utc().add_seconds(advance_seconds);
    let iso = advanced_utc.to_iso8601();
    let (year, month, day, hour, minute, second) =
        parse_iso8601_components(&iso).ok_or_else(|| {
            EngineError::Eternal(format!("iso8601 inválido: {}", iso))
        })?;
    let stored = cosmos_model::StoredBirthData {
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
/// minute, second_float)`. Retorna `None` si el format no encaja.
pub(crate) fn parse_iso8601_components(s: &str) -> Option<(i32, u32, u32, u32, u32, f64)> {
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
