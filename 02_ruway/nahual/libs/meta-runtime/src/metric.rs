//! Cómputo de los agregados de un tablero (`DashboardCard`).

use std::collections::BTreeMap;

use serde_json::Value;
use uuid::Uuid;

use std::cmp::Ordering;

use nahual_meta_schema::{CardFilter, DateBucket, FilterOp, Metric};

/// Resultado de computar una [`Metric`] sobre un conjunto de records.
#[derive(Debug, Clone, PartialEq)]
pub enum MetricResult {
    /// Un único número — `Count` / `Sum` / `Avg` / `Min` / `Max`.
    Scalar(f64),
    /// Conteo por grupo, ordenado de mayor a menor — `GroupBy`.
    Breakdown(Vec<(String, usize)>),
    /// Valor numérico agregado por grupo, ordenado de mayor a menor —
    /// `SumBy` / `AvgBy`. Se formatea con el `ValueFormat` de la
    /// tarjeta (p.ej. moneda), a diferencia del conteo de `Breakdown`.
    ValueBreakdown(Vec<(String, f64)>),
}

/// Computa el agregado de una tarjeta sobre `records`, aplicando el
/// `filter` si lo hay.
pub fn compute_metric(
    metric: &Metric,
    filter: Option<&CardFilter>,
    records: &[(Uuid, Value)],
) -> MetricResult {
    let passes = |v: &Value| match filter {
        None => true,
        Some(f) => filter_passes(v, f),
    };
    match metric {
        Metric::Count => {
            let n = records.iter().filter(|(_, v)| passes(v)).count();
            MetricResult::Scalar(n as f64)
        }
        Metric::Sum { field } => {
            let total: f64 = records
                .iter()
                .filter(|(_, v)| passes(v))
                .filter_map(|(_, v)| v.get(field).and_then(Value::as_f64))
                .sum();
            MetricResult::Scalar(total)
        }
        Metric::Avg { field } => {
            let nums: Vec<f64> = records
                .iter()
                .filter(|(_, v)| passes(v))
                .filter_map(|(_, v)| v.get(field).and_then(Value::as_f64))
                .collect();
            let avg = if nums.is_empty() {
                0.0
            } else {
                nums.iter().sum::<f64>() / nums.len() as f64
            };
            MetricResult::Scalar(avg)
        }
        Metric::Min { field } => {
            let m = records
                .iter()
                .filter(|(_, v)| passes(v))
                .filter_map(|(_, v)| v.get(field).and_then(Value::as_f64))
                .fold(f64::INFINITY, f64::min);
            MetricResult::Scalar(if m.is_finite() { m } else { 0.0 })
        }
        Metric::Max { field } => {
            let m = records
                .iter()
                .filter(|(_, v)| passes(v))
                .filter_map(|(_, v)| v.get(field).and_then(Value::as_f64))
                .fold(f64::NEG_INFINITY, f64::max);
            MetricResult::Scalar(if m.is_finite() { m } else { 0.0 })
        }
        Metric::GroupBy { field } => {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for (_, v) in records.iter().filter(|(_, v)| passes(v)) {
                let key = field_as_text(v, field).unwrap_or_else(|| "(vacío)".to_string());
                *counts.entry(key).or_default() += 1;
            }
            let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
            // Mayor conteo primero; empates ordenados por nombre.
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            MetricResult::Breakdown(ranked)
        }
        Metric::SumBy { group, value } => {
            MetricResult::ValueBreakdown(grouped_aggregate(records, &passes, group, value, false))
        }
        Metric::AvgBy { group, value } => {
            MetricResult::ValueBreakdown(grouped_aggregate(records, &passes, group, value, true))
        }
    }
}

/// Acumula `value` por cada valor distinto de `group`, devolviendo la
/// suma (`avg = false`) o el promedio (`avg = true`) por grupo,
/// ordenado de mayor a menor (empates por nombre de grupo).
fn grouped_aggregate(
    records: &[(Uuid, Value)],
    passes: &impl Fn(&Value) -> bool,
    group: &str,
    value: &str,
    avg: bool,
) -> Vec<(String, f64)> {
    // (suma, cuenta-de-numéricos) por grupo.
    let mut acc: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for (_, v) in records.iter().filter(|(_, v)| passes(v)) {
        let key = field_as_text(v, group).unwrap_or_else(|| "(vacío)".to_string());
        let entry = acc.entry(key).or_insert((0.0, 0));
        if let Some(n) = v.get(value).and_then(Value::as_f64) {
            entry.0 += n;
            entry.1 += 1;
        }
    }
    let mut ranked: Vec<(String, f64)> = acc
        .into_iter()
        .map(|(k, (sum, count))| {
            let out = if avg && count > 0 {
                sum / count as f64
            } else if avg {
                0.0
            } else {
                sum
            };
            (k, out)
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked
}

/// Versión pública del predicado de filtro: decide si un record entra
/// dado un [`CardFilter`]. Útil para componer filtros fuera del motor
/// (p.ej. controles interactivos que pre-filtran los records).
pub fn record_matches(v: &Value, f: &CardFilter) -> bool {
    filter_passes(v, f)
}

/// Decide si un record pasa el filtro de una tarjeta. Las comparaciones
/// de orden (`gt`/`lt`/`between`) son numéricas cuando ambos lados
/// parsean como número, y lexicográficas si no — lo que cubre rangos
/// de fecha en ISO-8601 sin parser de fechas.
fn filter_passes(v: &Value, f: &CardFilter) -> bool {
    let cell = field_as_text(v, &f.field);
    match f.op {
        FilterOp::Eq => cell.as_deref() == f.value.as_deref(),
        FilterOp::Ne => cell.as_deref() != f.value.as_deref(),
        FilterOp::NonEmpty => cell.map(|s| !s.is_empty()).unwrap_or(false),
        FilterOp::Gt | FilterOp::Gte | FilterOp::Lt | FilterOp::Lte => {
            let (Some(cell), Some(bound)) = (cell, f.value.as_ref()) else {
                return false;
            };
            let ord = cmp_text(&cell, bound);
            match f.op {
                FilterOp::Gt => ord == Ordering::Greater,
                FilterOp::Gte => ord != Ordering::Less,
                FilterOp::Lt => ord == Ordering::Less,
                FilterOp::Lte => ord != Ordering::Greater,
                _ => unreachable!(),
            }
        }
        FilterOp::Between => {
            let Some(cell) = cell else {
                return false;
            };
            let lo_ok = f
                .min
                .as_ref()
                .map_or(true, |lo| cmp_text(&cell, lo) != Ordering::Less);
            let hi_ok = f
                .max
                .as_ref()
                .map_or(true, |hi| cmp_text(&cell, hi) != Ordering::Greater);
            lo_ok && hi_ok
        }
    }
}

/// Orden entre dos valores como texto: numérico si ambos parsean,
/// lexicográfico en caso contrario.
fn cmp_text(a: &str, b: &str) -> Ordering {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        _ => a.cmp(b),
    }
}

/// Serializa un desglose (`Breakdown` conteo o `ValueBreakdown` valor)
/// a CSV de dos columnas. `value_header` rotula la segunda columna.
/// Reusa el `to_csv` del runtime para el quoting.
pub fn breakdown_to_csv(
    result: &MetricResult,
    group_header: &str,
    value_header: &str,
) -> Option<String> {
    let rows: Vec<Vec<String>> = match result {
        MetricResult::Breakdown(rows) => rows
            .iter()
            .map(|(k, n)| vec![k.clone(), n.to_string()])
            .collect(),
        MetricResult::ValueBreakdown(rows) => rows
            .iter()
            .map(|(k, v)| {
                let n = if v.fract() == 0.0 {
                    format!("{}", *v as i64)
                } else {
                    v.to_string()
                };
                vec![k.clone(), n]
            })
            .collect(),
        MetricResult::Scalar(_) => return None,
    };
    Some(crate::csv::to_csv(
        &[group_header.to_string(), value_header.to_string()],
        &rows,
    ))
}

/// Etiqueta de la fila que agrupa el resto de un desglose recortado.
pub const OTROS_LABEL: &str = "Otros";

/// Recorta un desglose a sus `limit` filas de mayor valor (el motor ya
/// las entrega ordenadas de mayor a menor) y colapsa el resto en una
/// fila [`OTROS_LABEL`]. Devuelve `true` si efectivamente agregó esa
/// fila (hubo más de `limit` grupos), para que la capa de presentación
/// la marque como no-navegable.
///
/// El valor de "Otros" es la **suma** del resto para conteos
/// (`GroupBy`) y para sumas (`SumBy`, `additive = true`); para
/// promedios (`AvgBy`, `additive = false`) es el promedio aritmético de
/// los valores de los grupos restantes (aproximación clara: la fila
/// "Otros" no es un grupo real). `limit == 0` no recorta nada.
pub fn limit_breakdown(result: &mut MetricResult, limit: usize, additive: bool) -> bool {
    if limit == 0 {
        return false;
    }
    match result {
        MetricResult::Breakdown(rows) if rows.len() > limit => {
            let tail: usize = rows[limit..].iter().map(|(_, n)| *n).sum();
            rows.truncate(limit);
            rows.push((OTROS_LABEL.to_string(), tail));
            true
        }
        MetricResult::ValueBreakdown(rows) if rows.len() > limit => {
            let rest = &rows[limit..];
            let value = if additive {
                rest.iter().map(|(_, v)| *v).sum()
            } else {
                let sum: f64 = rest.iter().map(|(_, v)| *v).sum();
                sum / rest.len() as f64
            };
            rows.truncate(limit);
            rows.push((OTROS_LABEL.to_string(), value));
            true
        }
        _ => false,
    }
}

/// Trunca una fecha ISO-8601 a la granularidad pedida, para agrupar
/// series temporales (`2026-01-15T10:30Z` / `2026-01-15` → `2026`,
/// `2026-01` o `2026-01-15`). Si `raw` no empieza con un año de 4
/// dígitos seguido de `-` (o no es una fecha reconocible), se devuelve
/// sin cambios — el bucketing es inofensivo sobre datos no-fecha.
pub fn bucket_date(raw: &str, bucket: DateBucket) -> String {
    // Reconocer el prefijo `YYYY-`: 4 dígitos + guión.
    let looks_like_date = raw.len() >= 5
        && raw.as_bytes()[..4].iter().all(u8::is_ascii_digit)
        && raw.as_bytes()[4] == b'-';
    if !looks_like_date {
        return raw.to_string();
    }
    // Fecha = los primeros 10 chars (`YYYY-MM-DD`) si los hay.
    let date = raw.get(0..10).unwrap_or(raw);
    let end = match bucket {
        DateBucket::Year => 4,
        DateBucket::Month => 7,
        DateBucket::Day => 10,
    };
    date.get(0..end).unwrap_or(date).to_string()
}

/// Reordena las filas de un desglose por su clave de grupo ascendente
/// (lexicográfico — coincide con el orden cronológico para claves ISO
/// de [`bucket_date`]). No-op sobre escalares.
pub fn sort_breakdown_by_key(result: &mut MetricResult) {
    match result {
        MetricResult::Breakdown(rows) => rows.sort_by(|a, b| a.0.cmp(&b.0)),
        MetricResult::ValueBreakdown(rows) => rows.sort_by(|a, b| a.0.cmp(&b.0)),
        MetricResult::Scalar(_) => {}
    }
}

/// Valor de un campo de nivel superior como texto plano, para comparar
/// (filtros) o agrupar (`GroupBy`).
fn field_as_text(v: &Value, field: &str) -> Option<String> {
    match v.get(field)? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn recs(items: &[Value]) -> Vec<(Uuid, Value)> {
        items.iter().map(|v| (Uuid::new_v4(), v.clone())).collect()
    }

    #[test]
    fn bucket_date_truncates_iso() {
        assert_eq!(bucket_date("2026-01-15", DateBucket::Year), "2026");
        assert_eq!(bucket_date("2026-01-15", DateBucket::Month), "2026-01");
        assert_eq!(bucket_date("2026-01-15", DateBucket::Day), "2026-01-15");
        // Con hora.
        assert_eq!(bucket_date("2026-03-09T10:30:00Z", DateBucket::Month), "2026-03");
        // No-fecha → sin cambios (inofensivo).
        assert_eq!(bucket_date("activo", DateBucket::Month), "activo");
        assert_eq!(bucket_date("", DateBucket::Year), "");
    }

    #[test]
    fn sort_breakdown_by_key_is_chronological_for_iso() {
        let mut r = MetricResult::ValueBreakdown(vec![
            ("2026-03".into(), 30.0),
            ("2026-01".into(), 10.0),
            ("2026-02".into(), 20.0),
        ]);
        sort_breakdown_by_key(&mut r);
        assert_eq!(
            r,
            MetricResult::ValueBreakdown(vec![
                ("2026-01".into(), 10.0),
                ("2026-02".into(), 20.0),
                ("2026-03".into(), 30.0),
            ])
        );
    }

    #[test]
    fn limit_breakdown_counts_sums_the_tail() {
        let mut r = MetricResult::Breakdown(vec![
            ("a".into(), 10),
            ("b".into(), 6),
            ("c".into(), 3),
            ("d".into(), 1),
        ]);
        let collapsed = limit_breakdown(&mut r, 2, true);
        assert!(collapsed);
        assert_eq!(
            r,
            MetricResult::Breakdown(vec![
                ("a".into(), 10),
                ("b".into(), 6),
                (OTROS_LABEL.into(), 4), // 3 + 1
            ])
        );
    }

    #[test]
    fn limit_breakdown_avg_bucket_is_mean_not_sum() {
        let mut r = MetricResult::ValueBreakdown(vec![
            ("a".into(), 100.0),
            ("b".into(), 80.0),
            ("c".into(), 60.0),
            ("d".into(), 40.0),
        ]);
        // additive = false (AvgBy): el bucket promedia los restantes.
        let collapsed = limit_breakdown(&mut r, 2, false);
        assert!(collapsed);
        assert_eq!(
            r,
            MetricResult::ValueBreakdown(vec![
                ("a".into(), 100.0),
                ("b".into(), 80.0),
                (OTROS_LABEL.into(), 50.0), // (60 + 40) / 2
            ])
        );
    }

    #[test]
    fn limit_breakdown_noop_when_within_limit() {
        let mut r = MetricResult::Breakdown(vec![("a".into(), 3), ("b".into(), 1)]);
        let before = r.clone();
        assert!(!limit_breakdown(&mut r, 5, true));
        assert!(!limit_breakdown(&mut r, 0, true));
        assert_eq!(r, before);
    }

    #[test]
    fn count_all_and_filtered() {
        let rs = recs(&[
            json!({"etapa": "ganada"}),
            json!({"etapa": "ganada"}),
            json!({"etapa": "perdida"}),
        ]);
        assert_eq!(
            compute_metric(&Metric::Count, None, &rs),
            MetricResult::Scalar(3.0)
        );
        let f = CardFilter {
            field: "etapa".into(),
            op: FilterOp::Eq,
            value: Some("ganada".into()),
            min: None,
            max: None,
        };
        assert_eq!(
            compute_metric(&Metric::Count, Some(&f), &rs),
            MetricResult::Scalar(2.0)
        );
    }

    fn filt(field: &str, op: FilterOp, value: Option<&str>) -> CardFilter {
        CardFilter {
            field: field.into(),
            op,
            value: value.map(Into::into),
            min: None,
            max: None,
        }
    }

    #[test]
    fn numeric_range_filters() {
        let rs = recs(&[
            json!({"monto": 100}),
            json!({"monto": 500}),
            json!({"monto": 900}),
        ]);
        // gte 500 → 500 y 900.
        assert_eq!(
            compute_metric(&Metric::Count, Some(&filt("monto", FilterOp::Gte, Some("500"))), &rs),
            MetricResult::Scalar(2.0)
        );
        // lt 500 → solo 100.
        assert_eq!(
            compute_metric(&Metric::Count, Some(&filt("monto", FilterOp::Lt, Some("500"))), &rs),
            MetricResult::Scalar(1.0)
        );
        // between [200, 800] → solo 500.
        let between = CardFilter {
            field: "monto".into(),
            op: FilterOp::Between,
            value: None,
            min: Some("200".into()),
            max: Some("800".into()),
        };
        assert_eq!(
            compute_metric(&Metric::Count, Some(&between), &rs),
            MetricResult::Scalar(1.0)
        );
    }

    #[test]
    fn date_range_is_lexicographic() {
        let rs = recs(&[
            json!({"fecha": "2026-01-15"}),
            json!({"fecha": "2026-06-30"}),
            json!({"fecha": "2027-02-01"}),
        ]);
        let q1_h1 = CardFilter {
            field: "fecha".into(),
            op: FilterOp::Between,
            value: None,
            min: Some("2026-01-01".into()),
            max: Some("2026-12-31".into()),
        };
        assert_eq!(
            compute_metric(&Metric::Count, Some(&q1_h1), &rs),
            MetricResult::Scalar(2.0)
        );
    }

    #[test]
    fn non_empty_filter() {
        let rs = recs(&[json!({"nota": "x"}), json!({"nota": ""}), json!({"otro": 1})]);
        assert_eq!(
            compute_metric(&Metric::Count, Some(&filt("nota", FilterOp::NonEmpty, None)), &rs),
            MetricResult::Scalar(1.0)
        );
    }

    #[test]
    fn breakdown_csv_roundtrip() {
        let res = MetricResult::ValueBreakdown(vec![
            ("ACME".into(), 1500.0),
            ("Globex".into(), 2000.0),
        ]);
        let csv = breakdown_to_csv(&res, "Cliente", "Monto").unwrap();
        assert_eq!(csv, "Cliente,Monto\nACME,1500\nGlobex,2000\n");
        assert!(breakdown_to_csv(&MetricResult::Scalar(1.0), "a", "b").is_none());
    }

    #[test]
    fn sum_skips_missing_and_non_numeric() {
        let rs = recs(&[
            json!({"monto": 1000}),
            json!({"monto": 2500}),
            json!({"otro": 1}),
        ]);
        assert_eq!(
            compute_metric(
                &Metric::Sum {
                    field: "monto".into()
                },
                None,
                &rs
            ),
            MetricResult::Scalar(3500.0)
        );
    }

    #[test]
    fn group_by_counts_and_ranks_by_frequency() {
        let rs = recs(&[
            json!({"etapa": "prospecto"}),
            json!({"etapa": "ganada"}),
            json!({"etapa": "ganada"}),
        ]);
        assert_eq!(
            compute_metric(
                &Metric::GroupBy {
                    field: "etapa".into()
                },
                None,
                &rs
            ),
            MetricResult::Breakdown(vec![
                ("ganada".to_string(), 2),
                ("prospecto".to_string(), 1),
            ])
        );
    }

    #[test]
    fn avg_min_max_over_numeric() {
        let rs = recs(&[
            json!({"monto": 100}),
            json!({"monto": 300}),
            json!({"otro": 1}), // ignorado
        ]);
        assert_eq!(
            compute_metric(&Metric::Avg { field: "monto".into() }, None, &rs),
            MetricResult::Scalar(200.0)
        );
        assert_eq!(
            compute_metric(&Metric::Min { field: "monto".into() }, None, &rs),
            MetricResult::Scalar(100.0)
        );
        assert_eq!(
            compute_metric(&Metric::Max { field: "monto".into() }, None, &rs),
            MetricResult::Scalar(300.0)
        );
    }

    #[test]
    fn avg_empty_is_zero_not_nan() {
        let rs = recs(&[json!({"otro": 1})]);
        assert_eq!(
            compute_metric(&Metric::Avg { field: "monto".into() }, None, &rs),
            MetricResult::Scalar(0.0)
        );
        assert_eq!(
            compute_metric(&Metric::Min { field: "monto".into() }, None, &rs),
            MetricResult::Scalar(0.0)
        );
    }

    #[test]
    fn sum_by_aggregates_and_ranks_by_value() {
        let rs = recs(&[
            json!({"cliente": "ACME", "monto": 1000}),
            json!({"cliente": "ACME", "monto": 500}),
            json!({"cliente": "Globex", "monto": 2000}),
        ]);
        assert_eq!(
            compute_metric(
                &Metric::SumBy {
                    group: "cliente".into(),
                    value: "monto".into()
                },
                None,
                &rs
            ),
            MetricResult::ValueBreakdown(vec![
                ("Globex".to_string(), 2000.0),
                ("ACME".to_string(), 1500.0),
            ])
        );
    }

    #[test]
    fn avg_by_is_per_group_mean() {
        let rs = recs(&[
            json!({"plan": "pro", "monto": 100}),
            json!({"plan": "pro", "monto": 300}),
            json!({"plan": "free", "monto": 50}),
        ]);
        assert_eq!(
            compute_metric(
                &Metric::AvgBy {
                    group: "plan".into(),
                    value: "monto".into()
                },
                None,
                &rs
            ),
            MetricResult::ValueBreakdown(vec![
                ("pro".to_string(), 200.0),
                ("free".to_string(), 50.0),
            ])
        );
    }
}
