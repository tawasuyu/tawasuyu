//! Puente a `cosmos-engine`: la carta de ejemplo y el `compute` que arma el
//! `RenderModel` desde un `Chart` + overlays + armónico.

use cosmos_engine::{compose, NatalOptions, PipelineRequest};
use cosmos_model::{
    Chart, ChartId, ChartKind, ContactId, StoredBirthData, StoredChartConfig, TimeCertainty,
};
use cosmos_render::RenderModel;

use crate::model::OverlayKind;

/// Parsea `"YYYY-MM-DDTHH:MM:SS.sss"` → `(y, mo, d, h, mi, s)`. Tolerante:
/// ante un formato inesperado cae a 2000-01-01T00:00:00.
fn parse_iso(iso: &str) -> (i32, u32, u32, u32, u32, f64) {
    let parse = || -> Option<(i32, u32, u32, u32, u32, f64)> {
        let (date, time) = iso.split_once('T')?;
        let mut dp = date.split('-');
        let y = dp.next()?.parse().ok()?;
        let mo = dp.next()?.parse().ok()?;
        let d = dp.next()?.parse().ok()?;
        let mut tp = time.split(':');
        let h = tp.next()?.parse().ok()?;
        let mi = tp.next()?.parse().ok()?;
        let s = tp.next()?.parse().ok()?;
        Some((y, mo, d, h, mi, s))
    };
    parse().unwrap_or((2000, 1, 1, 0, 0, 0.0))
}

/// Carta del **instante actual** (UTC) en una ubicación dada — para la rama
/// «Hoy». Tipo `Mundane` (sin sujeto natal). La fecha/hora es el ahora del
/// reloj; al refrescar (cada hora) se reconstruye con el nuevo instante.
pub(crate) fn now_chart(label: &str, lat: f64, lon: f64) -> Chart {
    let iso = cosmos_time::UTC::now().to_iso8601();
    let (year, month, day, hour, minute, second) = parse_iso(&iso);
    Chart {
        id: ChartId::new(),
        contact_id: ContactId::new(),
        kind: ChartKind::Mundane,
        label: label.to_string(),
        birth_data: StoredBirthData {
            year,
            month,
            day,
            hour,
            minute,
            second,
            tz_offset_minutes: 0,
            latitude_deg: lat,
            longitude_deg: lon,
            altitude_m: 0.0,
            time_certainty: TimeCertainty::Exact,
            subject_name: None,
            birthplace_label: Some(label.to_string()),
        },
        config: StoredChartConfig::default(),
        related_chart_id: None,
        created_at_ms: 0,
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso_ok() {
        assert_eq!(
            parse_iso("2026-06-04T15:30:12.345"),
            (2026, 6, 4, 15, 30, 12.345)
        );
        // Formato inválido → fallback.
        assert_eq!(parse_iso("xxx"), (2000, 1, 1, 0, 0, 0.0));
    }

    #[test]
    fn now_chart_uses_today_and_location() {
        let c = now_chart("Mi ubicación", -12.0464, -77.0428);
        assert_eq!(c.kind, ChartKind::Mundane);
        // La fecha es la de hoy (UTC) — al menos 2026 dado el reloj actual.
        assert!(c.birth_data.year >= 2026, "año = {}", c.birth_data.year);
        assert_eq!(c.birth_data.tz_offset_minutes, 0);
        assert!((c.birth_data.latitude_deg + 12.0464).abs() < 1e-9);
        assert!((c.birth_data.longitude_deg + 77.0428).abs() < 1e-9);
    }
}
