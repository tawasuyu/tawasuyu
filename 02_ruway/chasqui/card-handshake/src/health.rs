//! Observación de la **salud del broker por polling**: diff entre snapshots de
//! matches tomados en ticks sucesivos.
//!
//! A diferencia de [`crate::messages::MatchEvent`] (push del server al consumer,
//! tipo de protocolo serializable), esto es la observación que arma un monitor
//! que **consulta** la `MatchList` cada N segundos y deriva qué apareció
//! (`Available`) y qué desapareció (`Lost`) entre ticks. Vivía atrapado en el
//! frontend `chasqui-broker-explorer-llimphi`; baja acá —junto a `MatchList` y
//! `MatchEventKind`— para que cualquier monitor (UI, CLI) lo reuse (Regla 2).

use std::collections::HashSet;

use chasqui_broker::MatchStrategy;

use crate::messages::{MatchEventKind, MatchList, SessionId};

/// Clave estable de un match: `(sesión_consumer, flow_consumer, sesión_producer,
/// flow_producer)`. Identifica un match entre ticks para detectar altas y bajas.
pub type MatchKey = (SessionId, String, SessionId, String);

/// Una entrada del timeline de salud: un alta/baja observada al diferenciar dos
/// snapshots, con el momento en que el monitor la vio.
#[derive(Clone, Debug)]
pub struct TimelineEntry {
    pub at: std::time::SystemTime,
    pub kind: MatchEventKind,
    pub consumer_label: String,
    pub consumer_flow: String,
    pub producer_label: String,
    pub producer_flow: String,
    pub via: MatchStrategy,
    pub pinned: bool,
}

/// Diff puro entre snapshots de matches. Devuelve la lista de entries nuevas
/// (Available + Lost) en orden Available-primero, y el set actualizado de keys.
pub fn diff_matches(
    last_keys: &HashSet<MatchKey>,
    list: &MatchList,
) -> (Vec<TimelineEntry>, HashSet<MatchKey>) {
    let now = std::time::SystemTime::now();
    let current_keys: HashSet<MatchKey> = list
        .matches
        .iter()
        .map(|m| {
            (
                m.consumer.session,
                m.consumer.flow_name.clone(),
                m.producer.session,
                m.producer.flow_name.clone(),
            )
        })
        .collect();

    let mut entries = Vec::new();
    for m in &list.matches {
        let key = (
            m.consumer.session,
            m.consumer.flow_name.clone(),
            m.producer.session,
            m.producer.flow_name.clone(),
        );
        if !last_keys.contains(&key) {
            entries.push(TimelineEntry {
                at: now,
                kind: MatchEventKind::Available,
                consumer_label: m.consumer_label.clone(),
                consumer_flow: m.consumer.flow_name.clone(),
                producer_label: m.producer_label.clone(),
                producer_flow: m.producer.flow_name.clone(),
                via: m.via,
                pinned: m.pinned,
            });
        }
    }
    for key in last_keys.iter() {
        if !current_keys.contains(key) {
            entries.push(TimelineEntry {
                at: now,
                kind: MatchEventKind::Lost,
                consumer_label: String::new(),
                consumer_flow: key.1.clone(),
                producer_label: String::new(),
                producer_flow: key.3.clone(),
                via: MatchStrategy::Exact,
                pinned: false,
            });
        }
    }
    (entries, current_keys)
}
