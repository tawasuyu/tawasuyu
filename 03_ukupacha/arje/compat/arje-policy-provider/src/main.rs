//! ente-policy-provider: Ente que arbitra autorizaciones de Polkit.
//!
//! Se anuncia como proveedor de `POLKIT_DECISION_IFACE` en el bus interno.
//! Cuando `ente-polkit-compat` recibe `CheckAuthorization` D-Bus, forwarda
//! a este Ente vía Invoke. Aquí decidimos sí/no según política configurada.
//!
//! Wire format del blob de entrada: `pid_be_u32 | uid_be_u32 | action_id_utf8`.
//! Respuesta: `[decision_byte]` — 1 = allow, 0 = deny.
//!
//! Política se carga de `/etc/ente/policy.json` (o ruta override por env
//! `ENTE_POLICY_FILE`). Formato:
//! ```json
//! {
//!   "default": "allow",
//!   "rules": [
//!     { "match": "org.freedesktop.hostname1.*", "decision": "allow" },
//!     { "match": "org.freedesktop.login1.power-off", "require_uid": 0 },
//!     { "match": "*.set-*", "decision": "deny", "audit": true }
//!   ]
//! }
//! ```

use arje_bus::{BusResponse, BusServer, InvokeHandler, POLKIT_DECISION_IFACE};
use arje_card::Capability;
use serde::Deserialize;
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, Deserialize)]
struct PolicyConfig {
    #[serde(default = "default_decision")]
    default: Decision,
    #[serde(default)]
    rules: Vec<Rule>,
}

fn default_decision() -> Decision { Decision::Allow }

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Decision { Allow, Deny }

#[derive(Debug, Clone, Deserialize)]
struct Rule {
    /// Glob simple: `*` = wildcard. `org.freedesktop.hostname1.*` matchea
    /// cualquier action_id con ese prefijo.
    r#match: String,
    #[serde(default)]
    decision: Option<Decision>,
    /// Si presente, sólo este uid pasa. Otros se denegen.
    #[serde(default)]
    require_uid: Option<u32>,
    /// Si presente, sólo este pid pasa.
    #[serde(default)]
    require_pid: Option<u32>,
    #[serde(default)]
    audit: bool,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        // Default sensato: caps escaladas requieren uid 0; el resto allow.
        Self {
            default: Decision::Allow,
            rules: vec![
                // Power management: cualquiera puede pedir el reboot,
                // pero la decisión final está en el holder de Capability::Spawn.
                Rule {
                    r#match: "org.freedesktop.login1.set-wall-message".into(),
                    decision: Some(Decision::Allow), require_uid: None, require_pid: None, audit: true,
                },
                // hostname/timezone/locale: requieren root.
                Rule {
                    r#match: "org.freedesktop.hostname1.*".into(),
                    decision: None, require_uid: Some(0), require_pid: None, audit: true,
                },
                Rule {
                    r#match: "org.freedesktop.timedate1.*".into(),
                    decision: None, require_uid: Some(0), require_pid: None, audit: true,
                },
                Rule {
                    r#match: "org.freedesktop.locale1.*".into(),
                    decision: None, require_uid: Some(0), require_pid: None, audit: true,
                },
            ],
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    info!("ente-policy-provider: arrancando");

    let policy = load_policy();
    info!(rules = policy.rules.len(), default = ?policy.default, "policy cargada");

    let handler = PolicyHandler { policy: Arc::new(policy) };

    tokio::spawn(async {
        let mut term = signal(SignalKind::terminate()).unwrap();
        let mut int_ = signal(SignalKind::interrupt()).unwrap();
        tokio::select! {
            _ = term.recv() => info!("SIGTERM"),
            _ = int_.recv() => info!("SIGINT"),
        }
        std::process::exit(0);
    });

    // Una única conexión: announce + serve. Bidirectional bajo el hood.
    let mut server = BusServer::from_env().await?;
    server.announce(vec![Capability::Endpoint {
        interface: POLKIT_DECISION_IFACE,
        version: 1,
    }]).await?;
    info!("Announce OK; sirviendo invokes de policy decision");
    server.serve(handler).await?;
    Ok(())
}

struct PolicyHandler {
    policy: Arc<PolicyConfig>,
}

impl InvokeHandler for PolicyHandler {
    fn handle(&mut self, cap: Capability, blob: Vec<u8>) -> BusResponse {
        // Validar cap (defensa contra forwarding a interface incorrecto).
        if !matches!(&cap, Capability::Endpoint { interface, .. } if *interface == POLKIT_DECISION_IFACE) {
            return BusResponse::Error(format!("policy-provider: cap inesperado {cap:?}"));
        }
        // Decodificar blob: [pid:4][uid:4][action_id...]
        if blob.len() < 8 {
            return BusResponse::Error("blob demasiado corto (esperado pid|uid|action_id)".into());
        }
        let pid = u32::from_be_bytes(blob[0..4].try_into().unwrap());
        let uid = u32::from_be_bytes(blob[4..8].try_into().unwrap());
        let action_id = match std::str::from_utf8(&blob[8..]) {
            Ok(s) => s,
            Err(_) => return BusResponse::Error("action_id no es UTF-8".into()),
        };

        let decision = decide(&self.policy, action_id, pid, uid);
        let byte = if decision == Decision::Allow { 1u8 } else { 0u8 };
        info!(action_id, pid, uid, ?decision, "policy decision");
        BusResponse::Invoked { result: vec![byte] }
    }
}

fn decide(policy: &PolicyConfig, action_id: &str, pid: u32, uid: u32) -> Decision {
    for rule in &policy.rules {
        if !glob_match(&rule.r#match, action_id) { continue; }
        if let Some(req_uid) = rule.require_uid {
            if uid != req_uid {
                if rule.audit {
                    info!(action_id, uid, req_uid, "AUDIT: deny por uid mismatch");
                }
                return Decision::Deny;
            }
        }
        if let Some(req_pid) = rule.require_pid {
            if pid != req_pid {
                if rule.audit {
                    info!(action_id, pid, req_pid, "AUDIT: deny por pid mismatch");
                }
                return Decision::Deny;
            }
        }
        if let Some(d) = rule.decision {
            if rule.audit {
                info!(action_id, ?d, "AUDIT: rule match con decisión explícita");
            }
            return d;
        }
        // Rule matched pero sin decisión explícita (sólo require_*) y todos
        // los requires pasaron — caemos al default.
        if rule.audit {
            info!(action_id, ?policy.default, "AUDIT: rule match → default");
        }
        return policy.default;
    }
    policy.default
}

/// Glob simple: `*` matchea cualquier cosa. Soporta prefix (`foo.*`),
/// suffix (`*.bar`) y wildcard exacto (`*`). No es PCRE — intencional.
fn glob_match(pattern: &str, target: &str) -> bool {
    if pattern == "*" { return true; }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return target == prefix || target.starts_with(&format!("{prefix}."));
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return target == suffix || target.ends_with(&format!(".{suffix}"));
    }
    pattern == target
}

fn load_policy() -> PolicyConfig {
    let path = std::env::var("ENTE_POLICY_FILE")
        .unwrap_or_else(|_| "/etc/ente/policy.json".into());
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(p) => { info!(path, "policy file cargado"); p }
            Err(e) => {
                warn!(?e, path, "policy file inválido, usando defaults");
                PolicyConfig::default()
            }
        },
        Err(_) => {
            info!(path, "policy file ausente — usando defaults conservadores");
            PolicyConfig::default()
        }
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("arje_policy_provider=info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}
