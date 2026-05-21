//! Cómputo de los agregados de un tablero (`DashboardCard`).

use std::collections::BTreeMap;

use serde_json::Value;
use uuid::Uuid;

use nahual_meta_schema::{CardFilter, Metric};

/// Resultado de computar una [`Metric`] sobre un conjunto de records.
#[derive(Debug, Clone, PartialEq)]
pub enum MetricResult {
    /// Un único número — `Count` o `Sum`.
    Scalar(f64),
    /// Conteo por grupo, ordenado de mayor a menor — `GroupBy`.
    Breakdown(Vec<(String, usize)>),
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
}
