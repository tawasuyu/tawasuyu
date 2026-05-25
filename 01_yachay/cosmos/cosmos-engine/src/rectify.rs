//! Rectificador automático — microajuste por direcciones primarias.
//!
//! La rectificación horaria responde a una pregunta vieja: si la hora de
//! nacimiento registrada es incierta, ¿cuál es la verdadera? El método
//! ascensional la ataca con direcciones primarias: en la hora correcta,
//! los eventos reales de la vida del sujeto **coinciden** con la
//! perfección de una dirección primaria — el arco que la esfera celeste
//! rota tras el nacimiento hasta que un promisor alcanza la posición
//! mundana de un significador.
//!
//! La trigonometría esférica de esos arcos —el método Placidus-mundano,
//! semi-arcos diurnos/nocturnos bajo el polo de cada cuerpo— **no se
//! reimplementa aquí**: la aporta, ya probada, `eternal-astrology`
//! (`primary_direction::all_directions`). Este módulo es la capa de
//! OPTIMIZACIÓN: barre las horas candidatas y minimiza el desajuste
//! entre los eventos conocidos y los arcos teóricos.
//!
//! El barrido es de **dos pasadas**: una gruesa, minuto a minuto sobre
//! toda la ventana (el perfil que la UI dibuja como curva), y una fina,
//! segundo a segundo alrededor del mejor minuto — de ahí la precisión
//! de segundo del microajuste.

use cosmos_astrology::primary_direction::{all_directions, DirectionMethod};
use cosmos_astrology::{DirectionKey as EDirectionKey, NatalChart};

use crate::bridge::compute_natal_chart;
use crate::{Chart, EngineError, EventoConocido, Rectificacion};

/// Edad máxima (años) hasta la que se computan direcciones primarias —
/// cubre con holgura cualquier evento de una vida humana.
const EDAD_MAX: f64 = 100.0;

/// Penalización (años) que se imputa a un evento cuando ninguna
/// dirección primaria cae cerca. Mayor que cualquier desajuste real
/// plausible: un candidato sin dirección queda inequívocamente peor.
const SIN_DIRECCION: f32 = 20.0;

/// Error de una carta candidata frente a los eventos conocidos: por
/// cada evento, la distancia en años a la dirección primaria más
/// cercana; el error total es la suma. Es la función de coste del
/// microajuste — el segundo de nacimiento correcto la lleva a un valle.
fn error_de_carta(
    natal: &NatalChart,
    eventos: &[EventoConocido],
    key: EDirectionKey,
) -> f32 {
    // Todas las direcciones primarias (Placidus-mundano) y la edad a la
    // que cada una perfecciona. La matemática esférica vive en eternal.
    let dirs = all_directions(natal, DirectionMethod::PlacidusMundane, key, EDAD_MAX);
    let mut total = 0.0_f32;
    for evento in eventos {
        // La dirección cuya perfección cae más cerca de la edad del
        // evento: en la hora correcta, esa distancia tiende a cero.
        let cercania = dirs
            .iter()
            .map(|d| (evento.edad_years - d.age_years).abs() as f32)
            .reduce(f32::min)
            .unwrap_or(SIN_DIRECCION);
        total += cercania.min(SIN_DIRECCION);
    }
    total
}

/// Barre los offsets de `[desde, hasta]` segundos con paso `paso` y
/// devuelve `(offset_segundos, error)` por candidato.
fn barrer(
    chart: &Chart,
    eventos: &[EventoConocido],
    key: EDirectionKey,
    desde: i64,
    hasta: i64,
    paso: i64,
) -> Result<Vec<(i64, f32)>, EngineError> {
    let mut perfil = Vec::new();
    let mut offset = desde;
    while offset <= hasta {
        // Una carta natal por hora candidata (cacheada en el bridge).
        let (natal, _, _) = compute_natal_chart(chart, offset)?;
        perfil.push((offset, error_de_carta(&natal, eventos, key)));
        offset += paso;
    }
    Ok(perfil)
}

/// El candidato de menor error. Ante empate, el offset más cercano a 0
/// — la hora registrada se respeta si nada la mejora.
fn mejor_de(perfil: &[(i64, f32)]) -> (i64, f32) {
    perfil
        .iter()
        .copied()
        .min_by(|(oa, pa), (ob, pb)| {
            pa.partial_cmp(pb)
                .unwrap_or(core::cmp::Ordering::Equal)
                .then(oa.abs().cmp(&ob.abs()))
        })
        .unwrap_or((0, 0.0))
}

/// Barre las horas candidatas y devuelve la rectificación. Ver
/// [`crate::rectificar`] para la documentación pública.
pub(crate) fn rectificar(
    chart: &Chart,
    eventos: &[EventoConocido],
    ventana_min: i64,
    key_str: &str,
) -> Result<Rectificacion, EngineError> {
    if eventos.is_empty() {
        return Err(EngineError::Eternal(
            "rectificar: sin eventos conocidos que anclar la búsqueda".into(),
        ));
    }
    let ventana = ventana_min.max(1);
    let key = match key_str {
        "ptolemy" => EDirectionKey::Ptolemy,
        _ => EDirectionKey::Naibod,
    };

    // PASADA 1 — gruesa, minuto a minuto sobre toda la ventana. Es el
    // perfil que la UI dibuja como curva: el valle salta a la vista.
    let perfil = barrer(chart, eventos, key, -ventana * 60, ventana * 60, 60)?;
    let (mejor_minuto, _) = mejor_de(&perfil);

    // PASADA 2 — fina, segundo a segundo en ±60 s alrededor del mejor
    // minuto. Aquí nace la precisión de segundo del microajuste.
    let fino = barrer(chart, eventos, key, mejor_minuto - 60, mejor_minuto + 60, 1)?;
    let (mejor_offset_segundos, mejor_puntaje) = mejor_de(&fino);

    Ok(Rectificacion {
        mejor_offset_segundos,
        mejor_puntaje,
        perfil,
    })
}
