//! Rectificador automático — Sistema GR.
//!
//! La rectificación horaria responde a una pregunta vieja: si la hora de
//! nacimiento registrada es incierta, ¿cuál es la verdadera? El método GR
//! (García Rosas) la ataca con direcciones primarias: en la hora correcta,
//! los eventos reales de la vida del sujeto caen sobre **convergencias** —
//! un promisor directo y otro converso que se cruzan sobre un mismo punto
//! natal.
//!
//! Este módulo automatiza la búsqueda. Dada una carta, una ventana de horas
//! candidatas alrededor de la registrada, y una lista de eventos conocidos
//! (cada uno, una edad), **barre** las candidatas: para cada hora, computa
//! la carta y mide —con [`convergencia_minima`]— qué tan cerrada es la mejor
//! convergencia GR a la edad de cada evento. La hora cuyo puntaje total es
//! mínimo es la rectificada.
//!
//! El cómputo pesado —la carta natal por hora candidata— se delega a
//! `bridge::compute_natal_chart`, que cachea; la proyección primaria por
//! cuerpo es aritmética barata. La función de puntaje, [`convergencia_minima`],
//! es lógica pura y vive en `cosmobiologia-render`.

use eternal_astrology::{
    directed_longitude, primary_direction::PrimaryDirection, DirectionKey as EDirectionKey,
    NatalChart,
};

use crate::bridge::{
    body_symbol, compute_natal_chart, GR_EVENT_ORB_DEG, GR_HUD_ORB_DEG, GR_MAX_TRIGGERS,
};
use crate::{
    compute_gr_triggers, convergencia_minima, Chart, EngineError, EventoConocido, GrDirection,
    GrTrigger, Rectificacion,
};

/// Puntaje que se imputa a un evento cuando la carta candidata no halla
/// convergencia GR alguna a esa edad. Debe superar a cualquier suma real
/// de orbes (el HUD acota cada orbe a 2°, así que una convergencia real
/// nunca pasa de ~4°): así un candidato sin convergencias queda
/// inequívocamente por detrás de uno que sí las tiene.
const SIN_CONVERGENCIA: f32 = 8.0;

/// Computa los triggers GR de una carta natal ya calculada, a una edad
/// dada. Proyecta cada cuerpo en ambos sentidos (directo y converso) y los
/// empareja contra los puntos natales —cuerpos y los cuatro ángulos—.
///
/// Es la misma matemática que `bridge::build_primary_directions_overlay`,
/// pero sin construir el dual-ring de glifos: el rectificador sólo necesita
/// los triggers, no la capa visual.
fn gr_triggers_de_natal(
    natal: &NatalChart,
    edad_years: f64,
    key: EDirectionKey,
) -> Vec<GrTrigger> {
    let eps = natal.obliquity_rad;

    // Proyectar cada cuerpo natal por dirección primaria, en ambos sentidos.
    let mut directed: Vec<(String, GrDirection, f32)> = Vec::new();
    for (gr_dir, pd_dir) in [
        (GrDirection::Direct, PrimaryDirection::Direct),
        (GrDirection::Converse, PrimaryDirection::Converse),
    ] {
        for p in &natal.placements {
            let lon_rad = directed_longitude(
                p.right_ascension_rad,
                p.declination_rad,
                edad_years,
                pd_dir,
                key,
                eps,
            );
            let deg = (lon_rad.to_degrees() as f32).rem_euclid(360.0);
            directed.push((body_symbol(p.body).to_string(), gr_dir, deg));
        }
    }

    // Puntos natales objetivo: los cuerpos + los cuatro ángulos.
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
    natal_targets.push(("asc".into(), natal.ascendant().longitude_deg() as f32));
    natal_targets.push(("mc".into(), natal.midheaven().longitude_deg() as f32));
    natal_targets.push(("desc".into(), natal.descendant().longitude_deg() as f32));
    natal_targets.push(("ic".into(), natal.imum_coeli().longitude_deg() as f32));

    compute_gr_triggers(
        &directed,
        &natal_targets,
        GR_HUD_ORB_DEG,
        GR_EVENT_ORB_DEG,
        GR_MAX_TRIGGERS,
    )
}

/// Barre las horas candidatas y devuelve la rectificación. Ver
/// [`crate::rectificar`] para la documentación pública.
pub(crate) fn rectificar(
    chart: &Chart,
    eventos: &[EventoConocido],
    ventana_min: i64,
    paso_min: i64,
    key_str: &str,
) -> Result<Rectificacion, EngineError> {
    if eventos.is_empty() {
        return Err(EngineError::Eternal(
            "rectificar: sin eventos conocidos que anclar la búsqueda".into(),
        ));
    }
    let ventana = ventana_min.max(0);
    let paso = paso_min.max(1);
    let key = match key_str {
        "ptolemy" => EDirectionKey::Ptolemy,
        _ => EDirectionKey::Naibod,
    };

    // Barrer las horas candidatas: cada offset es una hora de nacimiento a
    // probar, en minutos sobre la registrada.
    let mut perfil: Vec<(i64, f32)> = Vec::new();
    let mut offset = -ventana;
    while offset <= ventana {
        // Una sola carta natal por hora candidata (cacheada en el bridge);
        // la proyección por edad de evento es barata sobre ella.
        let (natal, _, _) = compute_natal_chart(chart, offset)?;
        let mut puntaje = 0.0_f32;
        for evento in eventos {
            let triggers = gr_triggers_de_natal(&natal, evento.edad_years, key);
            // Menor orbe de convergencia = mejor explicación del evento;
            // sin convergencia, la penalización.
            puntaje += convergencia_minima(&triggers).unwrap_or(SIN_CONVERGENCIA);
        }
        perfil.push((offset, puntaje));
        offset += paso;
    }

    // El mejor candidato: puntaje mínimo. Ante empate, el offset más
    // cercano a 0 — la hora registrada se respeta si nada la mejora.
    let (mejor_offset_minutos, mejor_puntaje) = perfil
        .iter()
        .copied()
        .min_by(|(oa, pa), (ob, pb)| {
            pa.partial_cmp(pb)
                .unwrap_or(core::cmp::Ordering::Equal)
                .then(oa.abs().cmp(&ob.abs()))
        })
        .expect("el perfil tiene al menos un candidato — la ventana incluye el 0");

    Ok(Rectificacion {
        mejor_offset_minutos,
        mejor_puntaje,
        perfil,
    })
}
