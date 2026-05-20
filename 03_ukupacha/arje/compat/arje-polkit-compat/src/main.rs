//! ente-polkit-compat: shim de `org.freedesktop.PolicyKit1.Authority`.
//!
//! Polkit autoriza llamadas privilegiadas (e.g. SetHostname, PowerOff).
//! En el fractal no usamos polkit como gatekeeper — la auth se hace en
//! el bus interno via SO_PEERCRED y capability grants. Pero apps que
//! usan polkit (gnome-control-center, etc) bloquean en `CheckAuthorization`
//! si no responde nadie.
//!
//! Este shim responde "is_authorized=true" siempre — el fractal queda
//! como sistema confiado. El logging deja audit trail de qué acciones se
//! han pedido para futuro análisis.
//!
//! Producción real: integrar con el grant system del bus interno —
//! CheckAuthorization solicita un token al graph y devuelve true/false
//! según el resultado.

use arje_bus::{BusClient, BusRequest, BusResponse, POLKIT_DECISION_IFACE, POLKIT_SERVICE_IFACE};
use arje_card::Capability;
use std::collections::HashMap;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;
use zbus::{fdo, interface, zvariant::OwnedValue};

const BUS_NAME: &str = "org.freedesktop.PolicyKit1";
const OBJ_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    info!("ente-polkit-compat: arrancando");
    announce_to_fractal().await;

    let manager = PolkitAuthority;
    let conn_result = zbus::connection::Builder::system()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJ_PATH, manager));
    match conn_result {
        Ok(builder) => match builder.build().await {
            Ok(_conn) => {
                info!(name = BUS_NAME, "name acquired, sirviendo");
                wait_for_term().await
            }
            Err(e) => { warn!(?e, "build conn falló — modo idle"); wait_for_term().await }
        },
        Err(e) => { warn!(?e, "builder D-Bus falló — modo idle"); wait_for_term().await }
    }
}

struct PolkitAuthority;

/// Wire format de Polkit: `Subject = (s, a{sv})` — kind ("unix-session",
/// "unix-process", "system-bus-name") + detalles. El detail típico:
///   {"pid": u32, "start-time": u64, "uid": u32}
type Subject = (String, HashMap<String, OwnedValue>);

/// Resultado de `CheckAuthorization`: `(b, b, a{ss})` —
/// is_authorized, is_challenge, details.
type AuthResult = (bool, bool, HashMap<String, String>);

#[interface(name = "org.freedesktop.PolicyKit1.Authority")]
impl PolkitAuthority {
    async fn check_authorization(
        &self,
        subject: Subject,
        action_id: String,
        _details: HashMap<String, String>,
        _flags: u32,
        _cancellation_id: String,
    ) -> fdo::Result<AuthResult> {
        let (subj_kind, subj_details) = subject;
        let pid = subj_details.get("pid")
            .and_then(|v| u32::try_from(v).ok());
        let uid = subj_details.get("uid")
            .and_then(|v| u32::try_from(v).ok());

        // Pregunta al bus interno del fractal si hay un policy provider.
        // Si lo hay, su decisión gobierna. Si no (NoProvider), default = allow.
        let decision = query_policy(&action_id, pid, uid).await;
        info!(%action_id, %subj_kind, ?pid, ?uid, ?decision, "CheckAuthorization");
        Ok((decision.allow, false, HashMap::new()))
    }

    async fn check_authorization_by_async(
        &self,
        subject: Subject,
        action_id: String,
        details: HashMap<String, String>,
        flags: u32,
        cancellation_id: String,
    ) -> fdo::Result<AuthResult> {
        // Mismo comportamiento; algunos clientes llaman la versión async.
        self.check_authorization(subject, action_id, details, flags, cancellation_id).await
    }

    async fn cancel_check_authorization(&self, _cancellation_id: String) -> fdo::Result<()> {
        Ok(())
    }

    async fn enumerate_actions(&self, _locale: String) -> fdo::Result<Vec<EnumeratedAction>> {
        // Devolvemos lista vacía — no enumeramos acciones registradas.
        // El llamador (típicamente gnome-control-center settings panel)
        // debería degradar grácilmente.
        Ok(vec![])
    }

    async fn register_authentication_agent(
        &self,
        _subject: Subject,
        _locale: String,
        _object_path: String,
    ) -> fdo::Result<()> {
        info!("RegisterAuthenticationAgent (no-op)");
        Ok(())
    }

    async fn register_authentication_agent_with_options(
        &self,
        _subject: Subject,
        _locale: String,
        _object_path: String,
        _options: HashMap<String, OwnedValue>,
    ) -> fdo::Result<()> {
        Ok(())
    }

    async fn unregister_authentication_agent(
        &self,
        _subject: Subject,
        _object_path: String,
    ) -> fdo::Result<()> {
        Ok(())
    }

    async fn authentication_agent_response(
        &self,
        _cookie: String,
        _identity: (String, HashMap<String, OwnedValue>),
    ) -> fdo::Result<()> {
        Ok(())
    }

    async fn enumerate_temporary_authorizations(
        &self,
        _subject: Subject,
    ) -> fdo::Result<Vec<TemporaryAuth>> {
        Ok(vec![])
    }

    async fn revoke_temporary_authorizations(&self, _subject: Subject) -> fdo::Result<()> {
        Ok(())
    }

    async fn revoke_temporary_authorization_by_id(&self, _id: String) -> fdo::Result<()> {
        Ok(())
    }

    #[zbus(property)]
    async fn backend_name(&self) -> String { "ente-polkit-compat".into() }

    #[zbus(property)]
    async fn backend_version(&self) -> String { env!("CARGO_PKG_VERSION").into() }

    #[zbus(property)]
    async fn backend_features(&self) -> u32 { 0 }
}

/// Wire signature de EnumerateActions item:
/// `(ssssssuusa{ss})` — action_id, descripción, message, vendor, vendor_url,
/// icon_name, implicit_any, implicit_inactive, implicit_active, annotations.
type EnumeratedAction = (
    String, String, String, String, String, String,
    u32, u32, String, HashMap<String, String>,
);

/// Wire signature de TemporaryAuthorization:
/// `(sssss)` — id, action_id, subject_kind, subject_detail, time_obtained, time_expires.
/// Aquí `(string)` * 5 + 2 timestamps. Simplificamos al subset relevante.
type TemporaryAuth = (String, String, (String, HashMap<String, OwnedValue>), u64, u64);

/// Resultado de una consulta de policy al fractal.
#[derive(Debug)]
struct PolicyDecision {
    allow: bool,
    /// Origen: "fractal" si vino del bus, "default-allow" si no había proveedor.
    /// Sólo aparece en `Debug` (logging); ningún consumer lo lee programmático.
    #[allow(dead_code)]
    source: &'static str,
}

/// Pregunta al bus interno: ¿hay alguien que decida sobre `action_id`?
/// Wire format del blob: `pid_u32_be | uid_u32_be | action_id_utf8`.
/// El proveedor responde con `Invoked { result: [0|1] }` — 1 = allow.
async fn query_policy(action_id: &str, pid: Option<u32>, uid: Option<u32>) -> PolicyDecision {
    let mut blob = Vec::with_capacity(8 + action_id.len());
    blob.extend_from_slice(&pid.unwrap_or(0).to_be_bytes());
    blob.extend_from_slice(&uid.unwrap_or(0).to_be_bytes());
    blob.extend_from_slice(action_id.as_bytes());

    let mut client = match BusClient::from_env().await {
        Ok(c) => c,
        Err(e) => {
            debug!(?e, "no bus client — default allow");
            return PolicyDecision { allow: true, source: "no-bus" };
        }
    };
    let req = BusRequest::Invoke {
        cap: Capability::Endpoint {
            interface: POLKIT_DECISION_IFACE,
            version: 1,
        },
        blob,
    };
    match client.call(req).await {
        Ok(BusResponse::Invoked { result }) => {
            let allow = result.first().copied().unwrap_or(1) != 0;
            PolicyDecision { allow, source: "fractal" }
        }
        Ok(BusResponse::Error(msg)) if msg.contains("sin proveedor") => {
            // No hay policy provider — default allow.
            PolicyDecision { allow: true, source: "default-allow" }
        }
        Ok(other) => {
            warn!(?other, "policy: respuesta inesperada — default allow");
            PolicyDecision { allow: true, source: "default-allow" }
        }
        Err(e) => {
            warn!(?e, "policy: bus call falló — default allow");
            PolicyDecision { allow: true, source: "default-allow" }
        }
    }
}

async fn announce_to_fractal() {
    if let Ok(mut client) = BusClient::from_env().await {
        let req = BusRequest::Announce {
            capabilities: vec![Capability::Endpoint {
                interface: POLKIT_SERVICE_IFACE,
                version: 1,
            }],
        };
        match client.call(req).await {
            Ok(BusResponse::Ok) => info!("Announce → bus interno OK"),
            Ok(other) => warn!(?other, "Announce respuesta inesperada"),
            Err(e) => warn!(?e, "Announce falló"),
        }
    }
}

async fn wait_for_term() -> anyhow::Result<()> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int_ = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = term.recv() => info!("SIGTERM"),
        _ = int_.recv() => info!("SIGINT"),
    }
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("arje_polkit_compat=info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}
