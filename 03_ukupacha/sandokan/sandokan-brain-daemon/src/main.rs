//! `sandokan-cerebro` — el daemon que le da **teeth** al cerebro de reglas.
//!
//! Cierra el último hueco de despliegue de la capa 3 (SDD §8): el motor de
//! reglas determinista (`arje-brain-rules`) existía como librería pero ningún
//! proceso lo conectaba al contrato. Este daemon es ese consumidor:
//!
//! 1. Se **suscribe** al stream de eventos de ciclo de vida del init (`arje-bus`
//!    `Subscribe` → `BusEvent`: crash/exit/restart/parked/refloored).
//! 2. Los traduce al vocabulario del cerebro (`EventKind` + `SubjectInfo`) y los
//!    pasa por el `RuleEngine` cargado de disco.
//! 3. Despacha las acciones que matchean por el **contrato `Engine`** vía
//!    [`EngineSink`](sandokan_brain::EngineSink) → `stop`/`set_cpu_weight`/
//!    `freeze` (los verbos de capa 1), sobre el engine que elija `sandokan::auto`
//!    (en un host arje: `ArjeEngine` sobre el mismo bus).
//!
//! Sin reglas en disco (default) es un no-op seguro: escucha y no actúa. Las
//! reglas viven en `~/.config/sandokan/cerebro.json` (formato de
//! `arje-brain-rules`: array de `Rule`, o JSONL).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use arje_brain_rules::{dispatch_actions, EventKind, RuleEngine, SubjectInfo, TimedEvent};
use arje_bus::{BusClient, BusEvent};
use sandokan::Engine;
use sandokan_brain::EngineSink;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use ulid::Ulid;

/// Cuántos eventos recientes retener para evaluar patrones `Sequence`.
const HISTORY_MAX: usize = 64;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // El cerebro necesita el bus del init para recibir eventos. Sin él no hay
    // fuente de qué reaccionar — salir claro en vez de quedar mudo.
    let bus_sock = std::env::var(arje_bus::ENV_BUS_SOCK).map_err(|_| {
        anyhow::anyhow!(
            "{} no definido — sandokan-cerebro necesita el bus del init (arje) para suscribirse a eventos",
            arje_bus::ENV_BUS_SOCK
        )
    })?;

    let reglas = cargar_reglas();
    info!(n = reglas.len(), "reglas del cerebro cargadas");

    // Engine + sink: las acciones de control viajan por el contrato. En un host
    // arje, `auto` elige `ArjeEngine` sobre este mismo bus.
    let engine: Arc<dyn Engine> = Arc::from(sandokan::auto_default().await);
    let sink = EngineSink::new(engine);

    let mut client = BusClient::connect(&bus_sock).await?;
    client.subscribe().await?;
    info!(sock = %bus_sock, "cerebro suscrito al bus; escuchando eventos de ciclo de vida");

    let mut history: Vec<TimedEvent> = Vec::new();
    loop {
        let ev = match client.next_event().await {
            Ok(ev) => ev,
            Err(e) => {
                warn!(error = %e, "stream de eventos cerrado; el cerebro termina");
                break;
            }
        };
        let Some((kind, subject)) = evento_a_cerebro(&ev) else {
            continue;
        };
        history.push(TimedEvent { kind: kind.clone(), at: Instant::now() });
        if history.len() > HISTORY_MAX {
            history.remove(0);
        }
        let matched = reglas.dispatch(&kind, &subject, &history);
        if !matched.is_empty() {
            info!(evento = ?kind, n = matched.len(), "reglas matchean; despachando acciones");
            dispatch_actions(&matched, &sink).await;
        }
    }
    Ok(())
}

/// Traduce un `BusEvent` del init al vocabulario del cerebro. Las muertes
/// (crash o exit limpio) son `EnteDied`; el resto del ciclo de vida viaja como
/// `Custom(..)` para que una regla pueda matchearlo por nombre si quiere.
fn evento_a_cerebro(ev: &BusEvent) -> Option<(EventKind, SubjectInfo)> {
    match ev {
        BusEvent::EnteCrashed { id, label, .. } | BusEvent::EnteExited { id, label } => {
            Some((EventKind::EnteDied, sujeto(*id, label)))
        }
        BusEvent::EnteRestarting { id, label, .. } => {
            Some((EventKind::Custom("ente_restarting".into()), sujeto(*id, label)))
        }
        BusEvent::EnteParked { id, label } => {
            Some((EventKind::Custom("ente_parked".into()), sujeto(*id, label)))
        }
        BusEvent::EnteRefloored { id, label } => {
            Some((EventKind::Custom("ente_refloored".into()), sujeto(*id, label)))
        }
    }
}

fn sujeto(id: Ulid, label: &str) -> SubjectInfo {
    SubjectInfo {
        id: Some(id),
        label: Some(label.to_string()),
        capabilities: Vec::new(),
    }
}

/// Resuelve el `cerebro.json`. El cerebro corre como Ente de SISTEMA (root) —
/// su `$HOME` es `/root`, así que `~/.config` no sirve para el despliegue real.
/// Orden de resolución: override explícito por env `SANDOKAN_CEREBRO_REGLAS`
/// (para tests y overrides puntuales) → `/etc/sandokan/cerebro.json` (ruta de
/// sistema canónica, root-controlada, donde la siembra el install) → fallback
/// `~/.config/sandokan/cerebro.json` (corrida como usuario en dev). `None` sólo
/// si no hay ni env, ni `/etc`, ni config dir.
fn ruta_reglas() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SANDOKAN_CEREBRO_REGLAS") {
        return Some(PathBuf::from(p));
    }
    let etc = PathBuf::from("/etc/sandokan/cerebro.json");
    if etc.exists() {
        return Some(etc);
    }
    directories::ProjectDirs::from("", "", "sandokan").map(|d| d.config_dir().join("cerebro.json"))
}

/// Carga el `RuleEngine` de disco; vacío si no existe o está corrupto (no-op
/// seguro — el daemon escucha pero no actúa hasta que haya reglas).
fn cargar_reglas() -> RuleEngine {
    let Some(p) = ruta_reglas() else {
        return RuleEngine::empty();
    };
    match std::fs::read_to_string(&p) {
        Ok(s) => RuleEngine::load_json(&s).unwrap_or_else(|e| {
            warn!(path = %p.display(), error = %e, "cerebro.json inválido, uso reglas vacías");
            RuleEngine::empty()
        }),
        Err(_) => RuleEngine::empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arje_bus::LifecycleStatus;

    /// El `cerebro.json` que siembra el install DEBE cargar — si no, el daemon
    /// arranca con reglas vacías y el usuario no se entera. Este test lo delata.
    #[test]
    fn ejemplo_sembrado_carga() {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ejemplos/cerebro.json");
        let s = std::fs::read_to_string(&p).expect("leer ejemplos/cerebro.json");
        let eng = RuleEngine::load_json(&s).expect("cerebro.json de ejemplo no parsea");
        assert_eq!(eng.len(), 2, "el ejemplo trae 2 reglas (death + crash-storm)");
    }

    /// El override por env gana sobre `/etc` y `~/.config`: es lo que usan los
    /// tests y un operador que quiera apuntar a otro archivo sin tocar el sistema.
    #[test]
    fn env_override_gana_la_ruta() {
        let antes = std::env::var("SANDOKAN_CEREBRO_REGLAS").ok();
        std::env::set_var("SANDOKAN_CEREBRO_REGLAS", "/tmp/cerebro-test-xyz.json");
        assert_eq!(ruta_reglas(), Some(PathBuf::from("/tmp/cerebro-test-xyz.json")));
        match antes {
            Some(v) => std::env::set_var("SANDOKAN_CEREBRO_REGLAS", v),
            None => std::env::remove_var("SANDOKAN_CEREBRO_REGLAS"),
        }
    }

    #[test]
    fn crash_y_exit_son_muerte_el_resto_custom() {
        let id = Ulid::new();
        let crash = BusEvent::EnteCrashed { id, label: "x".into(), status: LifecycleStatus::Exited(1) };
        assert!(matches!(evento_a_cerebro(&crash), Some((EventKind::EnteDied, _))));
        let exit = BusEvent::EnteExited { id, label: "x".into() };
        assert!(matches!(evento_a_cerebro(&exit), Some((EventKind::EnteDied, _))));
        let parked = BusEvent::EnteParked { id, label: "x".into() };
        assert!(matches!(evento_a_cerebro(&parked), Some((EventKind::Custom(_), _))));
    }
}
