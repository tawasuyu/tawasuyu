//! Frontend D-Bus: implementa `org.freedesktop.Notifications` en el bus de
//! sesión. Molde tomado de los shims de `arje-compat` (zbus v4 + tokio).
//!
//! Cada `Notify` se persiste en el historial y se reenvía al loop de render
//! vía `Handle::dispatch`. El daemon emite las señales del spec
//! (`NotificationClosed`, `ActionInvoked`) y una propia (`Cambio`) que el panel
//! de historial usa para refrescar sin polling.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use llimphi_ui::Handle;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{info, warn};
use zbus::object_server::SignalContext;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{interface, proxy};

use crate::store::Store;
use crate::{now_usec, Cierre, Msg, Notificacion};

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJ_PATH: &str = "/org/freedesktop/Notifications";

/// Interfaz propia para que el panel de historial (y, más adelante, la capa de
/// triage) consulten el store sin pelear por el lock de sled: `sled` toma lock
/// exclusivo de proceso, así que el dueño del store lo sirve y los demás son
/// clientes. Mismo nombre de bus, distinto path/interfaz.
const HIST_IFACE: &str = "net.tawasuyu.Notificaciones1";
const HIST_PATH: &str = "/net/tawasuyu/Notificaciones1";

/// El objeto que sirve la interfaz freedesktop. `Send + Sync` (lo exige el
/// object server): el contador es atómico, store y handle son compartibles.
pub struct Servicio {
    next_id: AtomicU32,
    handle: Handle<Msg>,
    store: Store,
    /// «No molestar»: con esto puesto, las notificaciones se persisten igual al
    /// historial pero **no** se muestran como toast. Compartido con el
    /// [`Historiador`], que lo conmuta desde la barra.
    dnd: Arc<AtomicBool>,
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
        actions_planas: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> u32 {
        // replaces_id != 0 → el cliente actualiza una notificación viva.
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        };
        let urgency = hints.get("urgency").and_then(urgency_u8).unwrap_or(1);
        let actions = parsear_acciones(&actions_planas);
        let n = Notificacion {
            id,
            app_name,
            summary,
            body,
            urgency,
            actions,
            timeout_ms: expire_timeout,
            created_usec: now_usec(),
        };
        if let Err(e) = self.store.append(&n) {
            warn!(?e, "no se pudo persistir la notificación al historial");
        }
        // Espejo al centro de eventos (no-op si el daemon willay no corre): la
        // notificación también es un evento del timeline unificado.
        willay_emit::emitir_silencioso(&n.a_evento_willay());
        info!(id, app = %n.app_name, urgency = n.urgency, "Notify");
        // En «no molestar» la notificación queda en el historial pero no salta
        // como toast (no la mandamos al loop de render).
        if !self.dnd.load(Ordering::Relaxed) {
            self.handle.dispatch(Msg::Entrante(n));
        }

        // Avisar al panel que el historial cambió (señal en la otra interfaz).
        if let Ok(ctxt) = SignalContext::new(conn, HIST_PATH) {
            let _ = Historiador::cambio(&ctxt).await;
        }
        id
    }

    /// Cierre pedido por el cliente. Dispara el cierre en el loop de render, que
    /// es el único que emite `NotificationClosed` (con el motivo correcto).
    async fn close_notification(&self, id: u32) {
        self.handle.dispatch(Msg::CerrarPorCliente(id));
    }

    /// Capacidades cumplidas: cuerpo, botones de acción e historial persistente.
    async fn get_capabilities(&self) -> Vec<String> {
        vec!["body".into(), "actions".into(), "persistence".into()]
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

    /// Señal del spec: la notificación `id` se cerró. `reason`: 1 expiró, 2 la
    /// cerró el usuario, 3 vía `CloseNotification`, 4 indefinido.
    #[zbus(signal)]
    async fn notification_closed(ctxt: &SignalContext<'_>, id: u32, reason: u32) -> zbus::Result<()>;

    /// Señal del spec: el usuario invocó la acción `action_key` de `id`.
    #[zbus(signal)]
    async fn action_invoked(
        ctxt: &SignalContext<'_>,
        id: u32,
        action_key: String,
    ) -> zbus::Result<()>;
}

/// Extrae la urgencia (hint `urgency`, byte freedesktop) de un valor del dict.
fn urgency_u8(v: &OwnedValue) -> Option<u8> {
    match &**v {
        Value::U8(b) => Some(*b),
        _ => None,
    }
}

/// El array de acciones del spec viene plano `[clave1, etiqueta1, clave2, …]`;
/// lo volvemos pares `(clave, etiqueta)`.
fn parsear_acciones(planas: &[String]) -> Vec<(String, String)> {
    planas
        .chunks(2)
        .filter_map(|c| match c {
            [clave, etiqueta] => Some((clave.clone(), etiqueta.clone())),
            _ => None,
        })
        .collect()
}

/// Sirve el historial persistido. Devuelve JSON (lista de [`Notificacion`])
/// para no modelar un struct D-Bus a mano — el cliente deserializa con serde.
pub struct Historiador {
    pub store: Store,
    /// «No molestar», compartido con el [`Servicio`] (ver su campo `dnd`).
    pub dnd: Arc<AtomicBool>,
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

    /// `true` si «no molestar» está activo (las notificaciones no saltan como
    /// toast, sólo van al historial).
    async fn dnd(&self) -> bool {
        self.dnd.load(Ordering::Relaxed)
    }

    /// Activa/desactiva «no molestar». Lo conmuta la barra (`pata`).
    async fn set_dnd(&self, on: bool) {
        self.dnd.store(on, Ordering::Relaxed);
    }

    /// Señal: el historial cambió (llegó una notificación). El panel refresca.
    #[zbus(signal)]
    async fn cambio(ctxt: &SignalContext<'_>) -> zbus::Result<()>;
}

// ── Cliente proxy (lo usa el panel de historial) ────────────────────────────

/// Proxy generado para consumir la interfaz del historial: consultar, limpiar y
/// suscribirse a `Cambio` (refresco por señal en vez de polling).
#[proxy(
    default_service = "org.freedesktop.Notifications",
    default_path = "/net/tawasuyu/Notificaciones1",
    interface = "net.tawasuyu.Notificaciones1"
)]
pub trait Historial {
    fn historial(&self) -> zbus::Result<String>;
    fn limpiar(&self) -> zbus::Result<()>;
    fn dnd(&self) -> zbus::Result<bool>;
    fn set_dnd(&self, on: bool) -> zbus::Result<()>;
    #[zbus(signal)]
    fn cambio(&self) -> zbus::Result<()>;
}

/// Trae el historial del daemon (atajo sin proxy, para consumidores simples
/// como el CLI de triage).
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

// ── Servidor ────────────────────────────────────────────────────────────────

/// Registra el nombre en el bus de sesión, sirve ambas interfaces y, en el mismo
/// hilo, drena `rx` para emitir `NotificationClosed`/`ActionInvoked` cuando el
/// loop de render cierra un toast o se invoca una acción.
pub async fn serve(handle: Handle<Msg>, store: Store, mut rx: UnboundedReceiver<Cierre>) {
    let dnd = Arc::new(AtomicBool::new(false));
    let historiador = Historiador {
        store: store.clone(),
        dnd: dnd.clone(),
    };
    let svc = Servicio {
        next_id: AtomicU32::new(1),
        handle,
        store,
        dnd,
    };
    let built = zbus::connection::Builder::session()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJ_PATH, svc))
        .and_then(|b| b.serve_at(HIST_PATH, historiador));
    let conn = match built {
        Ok(builder) => match builder.build().await {
            Ok(conn) => {
                info!(name = BUS_NAME, "name adquirido — sirviendo notificaciones");
                conn
            }
            Err(e) => {
                warn!(?e, "build de conexión D-Bus falló — ¿ya hay otro daemon?");
                return;
            }
        },
        Err(e) => {
            warn!(?e, "builder D-Bus falló (¿sin bus de sesión?)");
            return;
        }
    };

    // Emitir señales del spec cuando el render cierra toasts / invoca acciones.
    let ctxt = match SignalContext::new(&conn, OBJ_PATH) {
        Ok(c) => c,
        Err(e) => {
            warn!(?e, "no se pudo armar el SignalContext — sin señales de cierre");
            std::future::pending::<()>().await;
            return;
        }
    };
    while let Some(ev) = rx.recv().await {
        match ev {
            Cierre::Cerrada { id, motivo } => {
                let _ = Servicio::notification_closed(&ctxt, id, motivo).await;
            }
            Cierre::Accion { id, clave } => {
                let _ = Servicio::action_invoked(&ctxt, id, clave).await;
            }
        }
    }
}
