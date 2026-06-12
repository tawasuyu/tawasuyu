//! Operaciones del rectificador de hora: barrido por direcciones primarias
//! (Sistema GR), jog de nacimiento y aplicación del mejor offset.

use crate::model::Model;
use crate::update::{recompute_astro, recompute_chart};

/// Clave arco↔año para el motor.
pub(crate) fn rectify_key(m: &Model) -> &'static str {
    if m.rectify_naibod {
        "naibod"
    } else {
        "ptolemy"
    }
}

/// Corre el barrido de rectificación con los eventos cargados (±2 h).
pub(crate) fn run_rectify(m: &mut Model) {
    if m.rectify_events.is_empty() {
        m.error = Some("Rectificador: cargá al menos un evento (edad)".into());
        return;
    }
    let eventos: Vec<cosmos_engine::EventoConocido> = m
        .rectify_events
        .iter()
        .map(|&edad_years| cosmos_engine::EventoConocido { edad_years })
        .collect();
    let key = rectify_key(m);
    match cosmos_engine::rectificar(&m.chart, &eventos, 120, key) {
        Ok(res) => {
            let secs = res.mejor_offset_segundos;
            m.status_note = Some(format!(
                "Rectificación: {:+} s ({:+} min) · error {:.2}",
                secs,
                secs / 60,
                res.mejor_puntaje
            ));
            m.rectify_result = Some(res);
        }
        Err(e) => m.error = Some(format!("rectificar: {e}")),
    }
}

/// Calcula los triggers GR (contactos directo/converso) a la edad de
/// inspección, con la carta y el offset de jog actuales.
pub(crate) fn compute_triggers(m: &mut Model) {
    let req = cosmos_engine::PipelineRequest::PrimaryDirections {
        target_age_years: m.rectify_age,
        key: rectify_key(m).to_string(),
    };
    match cosmos_engine::compose(&m.chart, m.rectify_offset_min, &[req]) {
        Ok(r) => {
            m.rectify_triggers = r.gr_triggers;
            if m.rectify_triggers.is_empty() {
                m.status_note = Some(format!("Sin triggers GR a los {:.1} años", m.rectify_age));
            }
        }
        Err(e) => m.error = Some(format!("triggers GR: {e}")),
    }
}

/// Aplica el mejor offset hallado a la hora de nacimiento de la carta.
pub(crate) fn apply_rectify(m: &mut Model) {
    let Some(res) = &m.rectify_result else {
        m.error = Some("Rectificador: corré primero el barrido".into());
        return;
    };
    let secs = res.mejor_offset_segundos;
    let bd = &mut m.chart.birth_data;
    // Total de segundos del día + offset, normalizado a [0, 86400).
    let total = ((bd.hour as i64 * 60 + bd.minute as i64) * 60) + bd.second as i64 + secs;
    let total = total.rem_euclid(86_400);
    bd.hour = (total / 3600) as u32;
    bd.minute = ((total % 3600) / 60) as u32;
    bd.second = (total % 60) as f64;
    bd.time_certainty = cosmos_model::TimeCertainty::Exact;
    // Refleja en la pestaña activa, persiste y recomputa con offset 0.
    if let Some(t) = m.open.get_mut(m.active_tab) {
        t.chart = m.chart.clone();
    }
    m.rectify_offset_min = 0;
    m.rectify_result = None;
    crate::persist::save_chart_to_disk(&m.chart);
    recompute_chart(m);
    recompute_astro(m);
    m.status_note = Some(format!(
        "Hora rectificada: {:02}:{:02}:{:02}",
        m.chart.birth_data.hour, m.chart.birth_data.minute, m.chart.birth_data.second as u32
    ));
}
