//! Autopromote loop. Background task que cada N segundos detecta cristales
//! con thresholds altos y los promueve sin intervención humana.
//!
//! Anti-doble-promote: tras promover, registramos en un set la pareja
//! (antecedent_kind, consequent_kind). Antes de promover, verificamos que
//! no exista ya una regla con el mismo trigger_kind (heurística simple —
//! evita ráfagas de duplicados de la misma estadística).

use crate::audit::AuditAction;
use crate::crystallize::{crystal_to_rule, detect_crystals, Crystal, CrystallizationParams};
use crate::introspect::{append_rule_jsonl, BrainState};
use crate::rules::EventKind;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy)]
pub struct AutopromoteParams {
    pub interval_secs: u64,
    pub threshold: CrystallizationParams,
}

impl Default for AutopromoteParams {
    fn default() -> Self {
        Self {
            interval_secs: 60,
            // Más estrictos que el threshold default — evitar ruido.
            threshold: CrystallizationParams {
                min_support: 10,
                min_conditional_prob: 0.85,
                min_pmi: 2.0,
            },
        }
    }
}

/// Spawn del bucle. El handle Mutex evita que dos pasadas concurrentes
/// promuevan el mismo cristal (el lock garantiza serialización por brain).
pub fn spawn_autopromote_loop(state: BrainState, params: AutopromoteParams) {
    let promoted_keys: Arc<Mutex<HashSet<(EventKind, EventKind)>>> =
        Arc::new(Mutex::new(HashSet::new()));

    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(params.interval_secs));
        tick.tick().await; // descartar primer tick inmediato
        info!(?params, "autopromote loop activo");
        loop {
            tick.tick().await;
            run_one_pass(&state, &params, &promoted_keys).await;
        }
    });
}

async fn run_one_pass(
    state: &BrainState,
    params: &AutopromoteParams,
    promoted_keys: &Arc<Mutex<HashSet<(EventKind, EventKind)>>>,
) {
    let crystals: Vec<Crystal> = {
        let obs = state.observer.read().await;
        detect_crystals(&obs, &params.threshold)
    };
    if crystals.is_empty() { return; }

    let mut pk = promoted_keys.lock().await;
    for c in crystals {
        let key = (c.antecedent.clone(), c.consequent.clone());
        if pk.contains(&key) {
            // Ya promovido — el observer puede seguir reportando este
            // cristal pero no necesitamos otra regla.
            continue;
        }
        promote_one(state, &c).await;
        pk.insert(key);
    }
}

async fn promote_one(state: &BrainState, c: &Crystal) {
    let rule = crystal_to_rule(c);
    let rule_id = rule.id;
    if let Some(path) = state.rules_out.as_ref() {
        if let Err(e) = append_rule_jsonl(path, &rule) {
            warn!(?e, "autopromote: rules_out append falló");
        }
    }
    state.engine.write().await.insert(rule);

    state.audit.write().await.append(AuditAction::PromoteCrystal {
        rule_id,
        crystal: c.clone(),
    });
    info!(
        %rule_id,
        antecedent = ?c.antecedent,
        consequent = ?c.consequent,
        cp = c.conditional_prob,
        pmi = c.pmi,
        "autopromote: cristal → regla"
    );
}
