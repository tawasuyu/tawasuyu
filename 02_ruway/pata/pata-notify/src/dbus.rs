//! Frontend D-Bus: implementa `org.freedesktop.Notifications` en el bus de
//! sesión. Molde tomado de los shims de `arje-compat` (zbus v4 + tokio).
//!
//! Cada `Notify` se persiste en el historial y se reenvía al loop de render
//! vía `Handle::dispatch`. El daemon es deliberadamente tonto: no decide,
//! no agrupa, no filtra — eso vive (más adelante) en una capa aparte que lee
//! el mismo store.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use llimphi_ui::Handle;
use tracing::{info, warn};
use zbus::interface;
use zbus::zvariant::{OwnedValue, Value};

use crate::store::Store;
use crate::{now_usec, Msg, Notificacion};

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJ_PATH: &str = "/org/freedesktop/Notifications";

/// Interfaz propia para que el panel de historial (y, más adelante, la capa de
/// triage) consulten el store sin pelear por el lock de sled: `sled` toma lock
/// exclusivo de proceso, así que el dueño del store lo sirve y los demás son
/// clientes. Mismo nombre de bus, distinto path/interfaz.
const HIST_IFACE: &str = "net.tawasuyu.Notificaciones1";
const HIST_PATH: &str = "/net/tawasuyu/Notificaciones1";

/// El objeto que sirve la interfaz. `Send + Sync` (lo exige el object server):
/// el contador es atómico, el store y el handle son clonables/compartibles.
pub struct Servicio {
    next_id: AtomicU32,
    handle: Handle<Msg>,
    store: Store,
}

#[interface(name = "org.freedesktop.Notifications")]
impl Servicio {
    /// El método central del spec. Devuelve el id de la notificación.
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        // replaces_id != 0 → el cliente actualiza una notificación viva.
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        };
        let urgency = hints.get("urgency").and_then(urgency_u8).unwrap_or(1);
        let n = Notificacion {
            id,
            app_name,
            summary,
            body,
            urgency,
            timeout_ms: expire_timeout,
            created_usec: now_usec(),
        };
        if let Err(e) = self.store.append(&n) {
            warn!(?e, "no se pudo persistir la notificación al historial");
        }
        info!(id, app = %n.app_name, urgency = n.urgency, "Notify");
        self.handle.dispatch(Msg::Entrante(n));
        id
    }

    /// Cierre pedido por el cliente. Saca el toast del stack vivo (el historial
    /// queda intacto). La señal `NotificationClosed` queda pendiente para una
    /// iteración futura.
    async fn close_notification(&self, id: u32) {
        self.handle.dispatch(Msg::Descarta(id));
    }

    /// Capacidades que el daemon realmente cumple. No anunciamos `actions`
    /// porque todavía no pintamos botones de acción (los toasts son
    /// click-para-descartar) — anunciarlo engañaría a los clientes.
    async fn get_capabilities(&self) -> Vec<String> {
        vec!["body".into(), "persistence".into()]
    }

    /// Identificación del servidor: (nombre, vendor, versión, versión-spec).
    async fn get_server_information(&self) -> (String, String, String, String) {
        (
            "pata-notify".into(),
            "tawasuyu".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.2".into(),
        )
    }
}

/// Extrae la urgencia (hint `urgency`, byte freedesktop) de un valor del dict.
fn urgency_u8(v: &OwnedValue) -> Option<u8> {
    match &**v {
        Value::U8(b) => Some(*b),
        _ => None,
    }
}

/// Sirve el historial persistido. Devuelve JSON (lista de [`Notificacion`])
/// para no modelar un struct D-Bus a mano — el cliente deserializa con serde.
pub struct Historiador {
    pub store: Store,
}

#[interface(name = "net.tawasuyu.Notificaciones1")]
impl Historiador {
    /// Historial completo en orden temporal, serializado como JSON.
    async fn historial(&self) -> String {
        match self.store.list() {
            Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| "[]".into()),
            Err(e) => {
                warn!(?e, "no se pudo leer el historial");
                "[]".into()
            }
        }
    }

    /// Vacía el historial.
    async fn limpiar(&self) {
        if let Err(e) = self.store.clear() {
            warn!(?e, "no se pudo limpiar el historial");
        }
    }
}

// ── Cliente (lo usa el panel de historial) ──────────────────────────────────

/// Trae el historial del daemon. El panel vive en otro proceso y no puede abrir
/// el sled directamente (lock exclusivo), así que pregunta por D-Bus.
pub async fn fetch_historial() -> anyhow::Result<Vec<Notificacion>> {
    let conn = zbus::Connection::session().await?;
    let reply = conn
        .call_method(Some(BUS_NAME), HIST_PATH, Some(HIST_IFACE), "Historial", &())
        .await?;
    let json: String = reply.body().deserialize()?;
    Ok(serde_json::from_str(&json)?)
}

/// Pide al daemon que vacíe el historial.
pub async fn limpiar_historial() -> anyhow::Result<()> {
    let conn = zbus::Connection::session().await?;
    conn.call_method(Some(BUS_NAME), HIST_PATH, Some(HIST_IFACE), "Limpiar", &())
        .await?;
    Ok(())
}

/// Registra el nombre en el bus de sesión y sirve la interfaz hasta que el
/// proceso termina. Pensado para correr dentro de un runtime tokio en su
/// propio hilo (ver [`crate::app::Daemon::init`]).
pub async fn serve(handle: Handle<Msg>, store: Store) {
    let historiador = Historiador { store: store.clone() };
    let svc = Servicio {
        next_id: AtomicU32::new(1),
        handle,
        store,
    };
    let built = zbus::connection::Builder::session()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJ_PATH, svc))
        .and_then(|b| b.serve_at(HIST_PATH, historiador));
    match built {
        Ok(builder) => match builder.build().await {
            Ok(_conn) => {
                info!(name = BUS_NAME, "name adquirido — sirviendo notificaciones");
                // Mantener viva la conexión; el object server atiende en tareas.
                std::future::pending::<()>().await;
            }
            Err(e) => warn!(?e, "build de conexión D-Bus falló — ¿ya hay otro daemon?"),
        },
        Err(e) => warn!(?e, "builder D-Bus falló (¿sin bus de sesión?)"),
    }
}
