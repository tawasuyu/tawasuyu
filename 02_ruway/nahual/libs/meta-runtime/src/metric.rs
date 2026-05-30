//! Cómputo de los agregados de un tablero (`DashboardCard`).

use std::collections::BTreeMap;

use serde_json::Value;
use uuid::Uuid;

use nahual_meta_schema::{CardFilter, Metric};

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
        Some(f) => field_as_text(v, &f.field).as_deref() == Some(f.equals.as_str()),
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
            equals: "ganada".into(),
        };
        assert_eq!(
            compute_metric(&Metric::Count, Some(&f), &rs),
            MetricResult::Scalar(2.0)
        );
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
