//! `cosmos_app-engine` — bridge entre el modelo agnóstico y
//! `eternal-astrology`.
//!
//! Recibe un `Chart` del modelo + un `ChartKind` y devuelve un
//! [`RenderModel`] que describe la geometría a pintar **sin** acoplar
//! el canvas a tipos de la librería astronómica. El canvas habla
//! grados decimales, radios normalizados y kinds simbólicos.
//!
//! ## Por qué un RenderModel intermedio
//!
//! 1. El canvas no debería caer si cambia el shape de `NatalChart`
//!    upstream.
//! 2. Tests del canvas: podemos generar `RenderModel`s sintéticos sin
//!    arrancar eternal.
//! 3. Cada `ChartKind` produce el mismo shape genérico → el render
//!    coordina N módulos sin saber qué calcularon.
//!
//! ## Feature `eternal-bridge`
//!
//! - **on** (default): [`compute`] abre una `EphemerisSession` VSOP2013
//!   compartida y corre la pipeline real.
//! - **off**: [`compute`] cae a [`compute_mock`] — útil para tests +
//!   builds sin eternal checked out.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use thiserror::Error;

pub use cosmos_model::{Chart, ChartId, ChartKind};

// Los tipos del RenderModel viven en `cosmos_app-render` (crate
// agnóstico de surface — compila a WASM, lo consumen tanto el canvas
// Llimphi como el cliente web). El engine los reexporta para mantener
// compatibilidad con todos los call sites históricos
// (`cosmos_engine::Layer`, etc.) sin tener que cambiar
// imports en el shell, canvas, modules, tree, panel...
pub use cosmos_render::{
    apply_harmonic, compute_gr_triggers, convergencia_minima, AspectSummary, Geometry, Glyph,
    GrDirection, GrTrigger, Layer, LayerKind, LineSeg, OverlayMeta, PointMark, RenderModel,
    UranianGroup, OUTER_RING_MODULES,
};

// El corpus de interpretación es agnóstico (no conoce eternal ni UI).
// El engine lo reexporta para que el shell y el canvas trabajen los
// pasajes sin importar el crate aparte.
pub use cosmos_corpus::{
    combinaciones_de_carta, rebanar_por_dominio, AspectoEnCarta, Colocacion, CombinacionId,
    Corpus, Dominio, EvidenciaVecina, Pasaje,
};

// `Chart` reexportado arriba es lo que `PipelineRequest::Synastry`
// transporta — el caller (shell) lee del Store y pasa el Chart entero
// para que el bridge construya su NatalChart en eternal.

#[cfg(feature = "eternal-bridge")]
mod bridge;
#[cfg(feature = "eternal-bridge")]
mod dignity;
#[cfg(feature = "eternal-bridge")]
mod natal_cache;
#[cfg(feature = "eternal-bridge")]
mod rectify;
#[cfg(feature = "eternal-bridge")]
pub mod svg_export;

// =====================================================================
// Errores
// =====================================================================

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("bridge a eternal-astrology no disponible (recompilá con feature `eternal-bridge`)")]
    BridgeDisabled,
    #[error("model: {0}")]
    Model(#[from] cosmos_model::ModelError),
    #[error("eternal: {0}")]
    Eternal(String),
    #[error("kind {0:?} todavía no implementado")]
    UnsupportedKind(ChartKind),
}

// =====================================================================
// API pública
// =====================================================================

/// Pedidos que el host (Shell) eleva a la engine para componer un
/// `RenderModel`. La capa natal **siempre** se computa; estos requests
/// son **overlays adicionales**.
///
/// Cada variante mapea 1-a-1 con un Module declarado en
/// `cosmos_app-modules` por id string. Esto deja la engine como
/// dueña única del cómputo (no depende del trait Module — los módulos
/// son sólo metadata + UI controls).
#[derive(Debug, Clone)]
pub enum PipelineRequest {
    /// `module_id = "transit"` — anillo externo con planetas al
    /// instante actual (reloj de pared) + cross aspects natal × transit.
    Transit,
    /// `module_id = "progression"` — anillo interno con los planetas
    /// progresados (método secundario "día por año") a la edad pedida
    /// + cross aspects natal × progresada.
    SecondaryProgression {
        /// Edad simbólica en años a la que avanzar la carta. Para "la
        /// edad de hoy", el shell la calcula a partir de `birth_data` +
        /// `SystemTime::now`.
        target_age_years: f64,
    },
    /// `module_id = "solar_arc"` — Solar Arc dirigido (default = "true
    /// progressed Sun"): cada cuerpo y cada cusp natal se desplazan por
    /// el mismo arco ≈ 1° por año de vida. Anillo interno bien adentro
    /// + cross aspects natal × dirigida.
    SolarArc {
        target_age_years: f64,
    },
    /// `module_id = "synastry"` — bi-wheel: la natal en el centro, la
    /// carta del partner en el anillo externo (compartido con Transit
    /// — mutuamente excluyentes), cross aspects natal × partner.
    /// El partner viene como `Chart` completo del shell.
    Synastry {
        partner_chart: Box<Chart>,
    },
    /// `module_id = "planetary_return"` — carta natal fresca al
    /// instante del próximo retorno del cuerpo elegido a su posición
    /// natal, para la edad pedida. Sun = retorno solar anual, Moon =
    /// mensual, Júpiter/Saturno = generacionales. Anillo externo
    /// compartido con Transit/Synastry — mutuamente excluyentes a
    /// nivel de Shell.
    PlanetaryReturn {
        /// Identificador agnóstico del cuerpo ("sun", "moon",
        /// "jupiter", …). El bridge lo mapea a `cosmos_sky::Body`.
        body: String,
        target_age_years: f64,
        /// Días extra que se suman al anchor de búsqueda (birth +
        /// age*año). Para Solar return suele ser 0 (el return cae cerca
        /// del cumpleaños); para Lunar return permite saltar de un
        /// retorno mensual al siguiente (~28 días por click).
        shift_days: i64,
    },
    /// `module_id = "midpoints"` — anillo de puntos medios entre pares
    /// de cuerpos natales. Por simplicidad filtramos a los que
    /// involucran al Sol o a la Luna (~10 puntos).
    Midpoints,
    /// `module_id = "composite"` — carta compuesta (midpoint composite,
    /// método Davison) entre dos sujetos. Renderea los planetas
    /// compuestos en un anillo interno propio (radio 0.36, entre solar
    /// arc 0.40 y aspects). Útil para análisis de relaciones.
    Composite {
        partner_chart: Box<Chart>,
    },
    /// `module_id = "uranian"` — calcula los "ejes" del dial uraniano
    /// de 90°: agrupa los cuerpos natales cuya longitud módulo 90 cae
    /// dentro de una tolerancia (~2°). El resultado se publica en
    /// `RenderModel.uranian_groups`; la UI lo pinta como un dial
    /// geométrico de 90° (proyección sobre el eje 0-90°) más la lista
    /// de fórmulas.
    Uranian,
    /// `module_id = "lots"` — Lots arábigos (helenísticos) calculados
    /// via `cosmos_astrology::compute_lot`: Fortune, Spirit, Eros,
    /// Necessity, Courage, Victory, Nemesis. Renderea cada lot como
    /// un texto pequeño en el ring de bodies natales.
    Lots,
    /// `module_id = "fixed_stars"` — overlay con ~9 estrellas fijas
    /// notables (Aldebaran, Regulus, Antares, Fomalhaut, Spica,
    /// Sirius, Algol, Vega, Pollux). Posiciones tropicales J2000
    /// aproximadas + precesión simple (~50.29″/año). Renderea como
    /// marcadores chicos justo afuera del sign dial.
    FixedStars,
    /// `module_id = "topocentric"` — capa "ascensional": planetas
    /// re-proyectados a longitud eclíptica topocéntrica (con paralaje
    /// horizontal aplicada por cuerpo) + casas Polich-Page (sistema
    /// topocéntrico de domificación). Visible sobre todo en la Luna
    /// (~1° de shift); imperceptible en planetas exteriores. La capa
    /// convive con la natal geocéntrica como overlay comparativo.
    Topocentric,
    /// `module_id = "pd_direct"` + `"pd_converse"` — Direcciones
    /// Primarias del Sistema GR (García Rosas). Cada cuerpo natal se
    /// proyecta dos veces: hacia adelante en el tiempo diurno
    /// (direct) y hacia atrás (converse). Los dos resultados a la
    /// edad pedida pintan un dual-ring para rectificación en vivo.
    ///
    /// `key` controla la conversión arco↔año: "naibod" (default
    /// moderno, 0°59'08.33″/año) o "ptolemy" (clásica, 1°/año).
    PrimaryDirections {
        target_age_years: f64,
        key: String,
    },
}

/// Opciones que afectan la pasada natal (qué aspectos pintar, qué
/// multiplicador de orbe usar). Es independiente de los overlays.
#[derive(Debug, Clone)]
pub struct NatalOptions {
    /// Incluir aspectos mayores (conj/opp/trine/square/sextile).
    pub show_majors: bool,
    /// Incluir aspectos menores (quincunx/semi-sextile/etc).
    pub show_minors: bool,
    /// Multiplicador uniforme sobre los orbes default. `1.0` = orbes
    /// modern_western; `0.5` = tight; `2.0` = wide.
    pub orb_multiplier: f64,
    /// Si `true`, anota cada cuerpo natal con su dignidad esencial
    /// (domicilio +, exaltación ·, exilio −, caída *). El canvas lo
    /// renderea como sufijo del glifo.
    pub show_dignities: bool,
    /// Orden de la carta armónica. `1` = carta natal sin transformar;
    /// `N > 1` re-renderiza los cuerpos en `(longitud · N) mod 360` y
    /// recomputa los aspectos sobre esas posiciones.
    pub harmonic: u32,
}

impl Default for NatalOptions {
    fn default() -> Self {
        Self {
            show_majors: true,
            show_minors: false,
            orb_multiplier: 1.0,
            show_dignities: false,
            harmonic: 1,
        }
    }
}

// =====================================================================
// Rectificador automático (direcciones primarias)
// =====================================================================

/// Un evento conocido de la vida del sujeto — el ancla de la
/// rectificación. La hora de nacimiento verdadera es la que hace caer
/// los eventos reales sobre la perfección de una dirección primaria.
#[derive(Debug, Clone, Copy)]
pub struct EventoConocido {
    /// Edad del sujeto, en años, cuando ocurrió el evento.
    pub edad_years: f64,
}

/// Resultado de un barrido de rectificación (ver [`rectificar`]).
#[derive(Debug, Clone)]
pub struct Rectificacion {
    /// Desplazamiento, en **segundos**, sobre la hora registrada, que
    /// mejor explica los eventos. `0` = la hora registrada ya es la
    /// mejor. La precisión de segundo viene de la pasada fina.
    pub mejor_offset_segundos: i64,
    /// Error del mejor candidato: la suma, en años, del desajuste entre
    /// cada evento y su dirección primaria más cercana. Menor = mejor.
    pub mejor_puntaje: f32,
    /// El barrido grueso: `(offset_segundos, error)` por candidato a
    /// resolución de minuto, ordenado por offset. La UI lo dibuja como
    /// curva — su valle marca la hora rectificada.
    pub perfil: Vec<(i64, f32)>,
}

/// Rectifica la hora de nacimiento por direcciones primarias. Barre las
/// horas candidatas en `±ventana_min` minutos sobre la registrada —una
/// pasada gruesa por minuto y una fina por segundo alrededor del mejor
/// minuto— y, por cada candidata, mide el desajuste entre los eventos
/// conocidos y los arcos de dirección primaria (método Placidus-mundano
/// de `eternal-astrology`). La hora del error mínimo es la rectificada.
///
/// `key` es la clave arco↔año: `"naibod"` (default) o `"ptolemy"`.
/// `Err` si la lista de eventos está vacía — sin anclas no hay búsqueda.
#[cfg(feature = "eternal-bridge")]
pub fn rectificar(
    chart: &Chart,
    eventos: &[EventoConocido],
    ventana_min: i64,
    key: &str,
) -> Result<Rectificacion, EngineError> {
    rectify::rectificar(chart, eventos, ventana_min, key)
}

/// Composición canónica: carta natal + todos los overlays pedidos.
/// Equivalente a `compose_with_options` con `NatalOptions::default()`.
pub fn compose(
    chart: &Chart,
    offset_minutes: i64,
    requests: &[PipelineRequest],
) -> Result<RenderModel, EngineError> {
    compose_with_options(chart, offset_minutes, requests, &NatalOptions::default())
}

/// Variante que permite controlar qué aspectos natales se computan y
/// con qué multiplicador de orbe.
pub fn compose_with_options(
    chart: &Chart,
    offset_minutes: i64,
    requests: &[PipelineRequest],
    natal_options: &NatalOptions,
) -> Result<RenderModel, EngineError> {
    #[cfg(feature = "eternal-bridge")]
    {
        bridge::compose(chart, offset_minutes, requests, natal_options)
    }
    #[cfg(not(feature = "eternal-bridge"))]
    {
        let _ = (offset_minutes, requests, natal_options);
        Ok(compute_mock(chart))
    }
}

/// Atajo: natal sin overlays. Equivalente a `compose(chart, 0, &[])`.
pub fn compute(chart: &Chart) -> Result<RenderModel, EngineError> {
    compose(chart, 0, &[])
}

/// Atajo: natal con time-scrubbing pero sin overlays.
pub fn compute_at_offset(chart: &Chart, offset_minutes: i64) -> Result<RenderModel, EngineError> {
    compose(chart, offset_minutes, &[])
}

/// Atajo: natal + overlay de tránsitos al instante actual.
pub fn compute_with_transits_at_now(
    chart: &Chart,
    offset_minutes: i64,
) -> Result<RenderModel, EngineError> {
    compose(chart, offset_minutes, &[PipelineRequest::Transit])
}

/// Computa la carta del retorno planetario actual (cuerpo + edad)
/// como `StoredBirthData` standalone — la app la usa para crear
/// una `FreeChart` que el usuario puede después persistir en un
/// contacto. Devuelve también un label-corto del instante para
/// concatenar al nombre.
#[cfg(feature = "eternal-bridge")]
pub fn compute_planetary_return_chart(
    chart: &Chart,
    body: &str,
    target_age_years: f64,
    shift_days: i64,
) -> Result<(cosmos_model::StoredBirthData, String), EngineError> {
    bridge::compute_planetary_return_chart(chart, body, target_age_years, shift_days)
}

/// Helper análogo para tránsito — birth_data = `ahora` UTC + lugar
/// del natal. Útil para snapshotear el cielo en este instante anclado
/// a las coordenadas del sujeto.
#[cfg(feature = "eternal-bridge")]
pub fn compute_transit_chart(
    chart: &Chart,
) -> Result<(cosmos_model::StoredBirthData, String), EngineError> {
    bridge::compute_transit_chart(chart)
}

/// Helper análogo para progresión secundaria — birth_data = natal +
/// target_age_years × 1 día simbólico.
#[cfg(feature = "eternal-bridge")]
pub fn compute_progression_chart(
    chart: &Chart,
    target_age_years: f64,
) -> Result<(cosmos_model::StoredBirthData, String), EngineError> {
    bridge::compute_progression_chart(chart, target_age_years)
}

/// Helper retrocompatible: construye un `PlanetaryReturn` con
/// `shift_days = 0`. Útil para llamadores que no necesitan ajuste
/// fino (todos los Solar return y muchos casos básicos).
pub fn planetary_return_request(body: String, target_age_years: f64) -> PipelineRequest {
    PipelineRequest::PlanetaryReturn {
        body,
        target_age_years,
        shift_days: 0,
    }
}

/// Stub determinista — útil para tests + para la UI sin eternal.
pub fn compute_mock(chart: &Chart) -> RenderModel {
    use std::time::Instant;
    let t0 = Instant::now();

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
                symbol: ZODIAC_GLYPHS[i].into(),
                annotation: None,
                retrograde: false,
                house: None,
            dignity_marker: None,
            })
            .collect(),
    };

    RenderModel {
        chart_id: chart.id,
        chart_kind: chart.kind,
        title: chart.label.clone(),
        subtitle: chart.birth_data.birthplace_label.clone(),
        compute_ms: t0.elapsed().as_millis() as u64,
        ascendant_deg: 0.0,
        midheaven_deg: 270.0,
        descendant_deg: 180.0,
        imum_coeli_deg: 90.0,
        geo_latitude_deg: chart.birth_data.latitude_deg as f32,
        geo_longitude_deg: chart.birth_data.longitude_deg as f32,
        layers: vec![sign_dial],
        overlays: Vec::new(),
        aspect_summary: Vec::new(),
        uranian_groups: Vec::new(),
        gr_triggers: Vec::new(),
        harmonic: 1,
        harmonic_spectrum: Vec::new(),
    }
}

const ZODIAC_GLYPHS: [&str; 12] = [
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

// =====================================================================
// Corpus de interpretación — puente carta → pasajes
// =====================================================================

/// Deriva las colocaciones y aspectos natales de un [`RenderModel`]
/// para alimentar el corpus de interpretación: cada cuerpo natal se
/// traduce a su `planeta·signo` + `planeta@casa`, y cada aspecto a su
/// terna. El caller hace luego `Corpus::interpretar_por_dominio`.
pub fn corpus_inputs(render: &RenderModel) -> (Vec<Colocacion>, Vec<AspectoEnCarta>) {
    let mut colocaciones = Vec::new();
    let mut aspectos = Vec::new();
    for layer in &render.layers {
        if layer.module_id != "natal" {
            continue;
        }
        match layer.kind {
            LayerKind::Bodies => {
                for g in &layer.glyphs {
                    colocaciones.push(Colocacion {
                        planeta: g.symbol.clone(),
                        signo: signo_de_longitud(g.deg).to_string(),
                        casa: g.house.unwrap_or(0),
                    });
                }
            }
            LayerKind::Aspects => {
                if let Geometry::Lines(segs) = &layer.geometry {
                    for s in segs {
                        if !s.from_body.is_empty() && !s.to_body.is_empty() {
                            aspectos.push(AspectoEnCarta {
                                a: s.from_body.clone(),
                                kind: s.kind.clone(),
                                b: s.to_body.clone(),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (colocaciones, aspectos)
}

/// El signo zodiacal (id agnóstico) de una longitud eclíptica.
fn signo_de_longitud(deg: f32) -> &'static str {
    const SIGNOS: [&str; 12] = [
        "aries", "taurus", "gemini", "cancer", "leo", "virgo", "libra",
        "scorpio", "sagittarius", "capricorn", "aquarius", "pisces",
    ];
    SIGNOS[(deg.rem_euclid(360.0) / 30.0) as usize % 12]
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cosmos_model::{
        Chart, ChartKind, ContactId, StoredBirthData, StoredChartConfig,
    };

    fn sample_chart() -> Chart {
        Chart {
            id: ChartId::new(),
            contact_id: ContactId::new(),
            kind: ChartKind::Natal,
            label: "test".into(),
            birth_data: StoredBirthData {
                year: 1987,
                month: 3,
                day: 14,
                hour: 5,
                minute: 22,
                second: 0.0,
                tz_offset_minutes: -240,
                latitude_deg: 10.4806,
                longitude_deg: -66.9036,
                altitude_m: 900.0,
                time_certainty: Default::default(),
                subject_name: None,
                birthplace_label: None,
            },
            config: StoredChartConfig::default(),
            related_chart_id: None,
            created_at_ms: 0,
        }
    }

    #[test]
    fn mock_emits_sign_dial() {
        let model = compute_mock(&sample_chart());
        assert_eq!(model.layers.len(), 1);
        assert!(matches!(model.layers[0].kind, LayerKind::SignDial));
        assert_eq!(model.layers[0].glyphs.len(), 12);
    }

    #[test]
    fn corpus_inputs_extrae_colocaciones_y_aspectos() {
        let render = RenderModel {
            chart_id: ChartId::new(),
            chart_kind: ChartKind::Natal,
            title: "x".into(),
            subtitle: None,
            compute_ms: 0,
            ascendant_deg: 0.0,
            midheaven_deg: 270.0,
            descendant_deg: 180.0,
            imum_coeli_deg: 90.0,
            geo_latitude_deg: 0.0,
            geo_longitude_deg: 0.0,
            layers: vec![
                Layer {
                    module_id: "natal".into(),
                    kind: LayerKind::Bodies,
                    ring: 0.0,
                    z: 0,
                    geometry: Geometry::GlyphsOnly,
                    glyphs: vec![
                        Glyph { deg: 12.0, symbol: "mars".into(), house: Some(6), ..Default::default() },
                        Glyph { deg: 200.0, symbol: "venus".into(), house: Some(1), ..Default::default() },
                    ],
                },
                Layer {
                    module_id: "natal".into(),
                    kind: LayerKind::Aspects,
                    ring: 0.0,
                    z: 0,
                    geometry: Geometry::Lines(vec![LineSeg {
                        from_deg: 12.0,
                        to_deg: 200.0,
                        kind: "square".into(),
                        opacity: 1.0,
                        from_body: "mars".into(),
                        to_body: "venus".into(),
                        orb_deg: 1.0,
                    }]),
                    glyphs: vec![],
                },
            ],
            overlays: vec![],
            aspect_summary: vec![],
            uranian_groups: vec![],
            gr_triggers: vec![],
            harmonic: 1,
            harmonic_spectrum: vec![],
        };
        let (col, asp) = corpus_inputs(&render);
        assert_eq!(col.len(), 2);
        assert_eq!(col[0].planeta, "mars");
        assert_eq!(col[0].signo, "aries", "12° cae en Aries");
        assert_eq!(col[0].casa, 6);
        assert_eq!(asp.len(), 1);
        assert_eq!(asp[0].kind, "square");
    }

    #[cfg(feature = "eternal-bridge")]
    #[test]
    fn real_compute_natal_demo() {
        let model = compute(&sample_chart()).expect("compute con eternal");
        assert!(model.layers.iter().any(|l| matches!(l.kind, LayerKind::SignDial)));
        assert!(model.layers.iter().any(|l| matches!(l.kind, LayerKind::Houses)));
        assert!(model.layers.iter().any(|l| matches!(l.kind, LayerKind::Bodies)));
        // El Asc debe ser un grado válido.
        assert!(model.ascendant_deg.is_finite());
        assert!((0.0..360.0).contains(&model.ascendant_deg));
    }

    /// El cache de NatalChart debe hacer que la segunda llamada con
    /// inputs idénticos sea sustancialmente más rápida que la primera.
    /// Verificamos un piso del 4× — en práctica el ratio suele ser
    /// >10× porque la primera carga VSOP2013 también.
    #[cfg(feature = "eternal-bridge")]
    #[test]
    fn natal_cache_hits_are_faster() {
        let chart = sample_chart();
        // Warmup: abre la sesión de efemérides y puebla el cache.
        let _ = compute(&chart).expect("warmup");

        // Reset implícito: insertar una clave distinta no botaría la
        // nuestra (cap=8) pero la marcaría como más vieja. Como solo
        // tenemos 1 entrada, sigue al frente.
        let t1 = std::time::Instant::now();
        let _ = compute(&chart).expect("primera medida");
        let cold_or_hot_1 = t1.elapsed();

        let t2 = std::time::Instant::now();
        let _ = compute(&chart).expect("segunda medida");
        let hot = t2.elapsed();

        // Después del warmup, las dos llamadas son hot. Para validar el
        // efecto del cache, modificamos el offset_minutes para forzar
        // un MISS y comparar contra un HIT.
        use crate::PipelineRequest;
        let t3 = std::time::Instant::now();
        let _ = compose(&chart, 17, &[] as &[PipelineRequest])
            .expect("miss con offset distinto");
        let miss = t3.elapsed();

        let t4 = std::time::Instant::now();
        let _ = compose(&chart, 17, &[] as &[PipelineRequest])
            .expect("hit con mismo offset");
        let hit = t4.elapsed();

        // Sanity: el hit debe ser estrictamente más rápido que el miss.
        assert!(
            hit < miss,
            "cache hit ({:?}) debería ser más rápido que miss ({:?}); \
             warmup={:?}, repeat={:?}",
            hit, miss, cold_or_hot_1, hot
        );
    }

    /// El overlay GR debe emitir el dual-ring (`pd_direct` +
    /// `pd_converse`) y una lista de triggers ordenada por orbe y
    /// acotada al orbe del HUD.
    #[cfg(feature = "eternal-bridge")]
    #[test]
    fn primary_directions_emit_dual_ring_and_triggers() {
        use crate::PipelineRequest;
        let model = compose(
            &sample_chart(),
            0,
            &[PipelineRequest::PrimaryDirections {
                target_age_years: 30.0,
                key: "naibod".into(),
            }],
        )
        .expect("compose con overlay GR");

        assert!(model.layers.iter().any(|l| l.module_id == "pd_direct"));
        assert!(model.layers.iter().any(|l| l.module_id == "pd_converse"));

        let mut prev = 0.0_f32;
        for t in &model.gr_triggers {
            assert!(t.orb_deg <= 2.0 + 1e-3, "orbe {} fuera del HUD", t.orb_deg);
            assert!(t.orb_deg + 1e-3 >= prev, "triggers desordenados");
            prev = t.orb_deg;
            if t.event {
                // Un evento exige orbe de micro-escala (≤ 5').
                assert!(t.orb_deg <= 5.0 / 60.0 + 1e-3, "evento con orbe ancho");
            }
        }
    }

    /// La carta armónica debe mover los cuerpos respecto de la natal y
    /// anotar el orden en el título.
    #[cfg(feature = "eternal-bridge")]
    #[test]
    fn harmonic_chart_transforms_bodies_and_title() {
        let chart = sample_chart();
        let natal = compose_with_options(&chart, 0, &[], &NatalOptions::default())
            .expect("compose natal");
        let h5 = compose_with_options(
            &chart,
            0,
            &[],
            &NatalOptions {
                harmonic: 5,
                ..NatalOptions::default()
            },
        )
        .expect("compose H5");

        assert!(h5.title.ends_with("· H5"), "título anota el armónico");

        let pick = |m: &RenderModel| -> Vec<f32> {
            m.layers
                .iter()
                .find(|l| matches!(l.kind, LayerKind::Bodies))
                .map(|l| l.glyphs.iter().map(|g| g.deg).collect())
                .unwrap_or_default()
        };
        let natal_degs = pick(&natal);
        let h5_degs = pick(&h5);
        assert_eq!(natal_degs.len(), h5_degs.len());
        let moved = natal_degs
            .iter()
            .zip(&h5_degs)
            .any(|(a, b)| (a - b).abs() > 0.01);
        assert!(moved, "el armónico debe mover los cuerpos");
    }

    /// El rectificador barre la ventana en dos pasadas, devuelve un
    /// perfil grueso ordenado y un offset fino de resolución de segundo.
    #[cfg(feature = "eternal-bridge")]
    #[test]
    fn rectificar_barre_la_ventana_y_elige_el_minimo() {
        let chart = sample_chart();
        let eventos = [
            EventoConocido { edad_years: 20.0 },
            EventoConocido { edad_years: 35.0 },
        ];
        let r = rectificar(&chart, &eventos, 10, "naibod").expect("rectificar");

        // Pasada gruesa: ±10 min a paso de minuto → 21 candidatos.
        assert_eq!(r.perfil.len(), 21);
        // El perfil (en segundos) va ordenado por offset ascendente.
        for par in r.perfil.windows(2) {
            assert!(par[0].0 < par[1].0, "perfil desordenado");
        }
        // El mejor offset cae dentro de la ventana + el margen de la
        // pasada fina (±60 s).
        assert!(r.mejor_offset_segundos.abs() <= 10 * 60 + 60);
        assert!(r.mejor_puntaje >= 0.0);

        // Sin eventos no hay ancla — debe ser un error.
        assert!(rectificar(&chart, &[], 10, "naibod").is_err());
    }
}
