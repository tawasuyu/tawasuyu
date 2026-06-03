//! Puente a `cosmos-engine`: la carta de ejemplo y el `compute` que arma el
//! `RenderModel` desde un `Chart` + overlays + armónico.

use cosmos_engine::{compose, NatalOptions, PipelineRequest};
use cosmos_model::{
    Chart, ChartId, ChartKind, ContactId, StoredBirthData, StoredChartConfig, TimeCertainty,
};
use cosmos_render::RenderModel;

use crate::model::OverlayKind;

pub(crate) fn sample_chart() -> Chart {
    Chart {
        id: ChartId::new(),
        contact_id: ContactId::new(),
        kind: ChartKind::Natal,
        label: rimay_localize::t("cosmos-demo-title"),
        birth_data: StoredBirthData {
            year: 1990,
            month: 6,
            day: 21,
            hour: 12,
            minute: 0,
            second: 0.0,
            tz_offset_minutes: -300,
            latitude_deg: -12.0464,
            longitude_deg: -77.0428,
            altitude_m: 154.0,
            time_certainty: TimeCertainty::Estimated,
            subject_name: None,
            birthplace_label: Some("Lima".into()),
        },
        config: StoredChartConfig::default(),
        related_chart_id: None,
        created_at_ms: 0,
    }
}

pub(crate) fn compute(
    chart: &Chart,
    overlays: &[OverlayKind],
    harmonic: u32,
    show_minors: bool,
    offset_min: i64,
) -> (RenderModel, Option<String>) {
    let target_age = 35.0;
    let requests: Vec<PipelineRequest> = overlays
        .iter()
        .map(|k| k.to_request(target_age))
        .collect();
    let opts = NatalOptions {
        show_majors: true,
        show_minors,
        orb_multiplier: 1.0,
        show_dignities: true,
        harmonic,
    };
    // `offset_min` = jog de rectificación: corre la hora de nacimiento sin
    // tocar la carta guardada, para ver moverse ángulos/casas en vivo.
    match cosmos_engine::compose_with_options(chart, offset_min, &requests, &opts) {
        Ok(r) => (r, None),
        Err(e) => {
            let msg = format!("{e}");
            (
                compose(chart, offset_min, &[])
                    .unwrap_or_else(|_| cosmos_engine::compute_mock(chart)),
                Some(msg),
            )
        }
    }
}
