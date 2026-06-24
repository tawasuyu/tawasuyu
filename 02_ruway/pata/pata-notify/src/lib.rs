//! `pata-notify` — el daemon de notificaciones de escritorio de tawasuyu.
//!
//! Tres caras, deliberadamente desacopladas:
//! - **Frontend** [`dbus`]: registra `org.freedesktop.Notifications` en el bus
//!   de sesión. Tanto apps ajenas (cualquier cliente freedesktop) como nativas
//!   (con un helper que hable la misma interfaz) entran por acá.
//! - **Render** [`app`]: una `App` de Llimphi que se pinta a sí misma como una
//!   **caja wlr-layer-shell** anclada a la esquina (vía `llimphi-layer`), usando
//!   el widget render-only `llimphi-widget-toast`. Agnóstico del compositor.
//! - **Historial** [`store`]: cada notificación se persiste en `sled`. Es el
//!   sustrato que luego leerán el panel de historial y la capa de triage/IA —
//!   el daemon en sí se mantiene tonto y fiable.
//!
//! El puente entre el frontend (runtime tokio en su propio hilo) y el loop Elm
//! de Llimphi es un `Handle<Msg>` clonado: el handler D-Bus reentra al `update`
//! con `Handle::dispatch(Msg::Entrante(..))`, sin sockets extra.

pub mod app;
pub mod dbus;
pub mod store;

use serde::{Deserialize, Serialize};

/// Una notificación entrante, normalizada del protocolo freedesktop. Es lo que
/// viaja al render y lo que se persiste en el historial.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notificacion {
    /// Id asignado por el servidor (o `replaces_id` si el cliente lo pidió).
    pub id: u32,
    /// Nombre declarado por la app emisora (puede venir vacío).
    pub app_name: String,
    /// Título corto.
    pub summary: String,
    /// Cuerpo (puede venir vacío).
    pub body: String,
    /// Urgencia freedesktop: 0 baja, 1 normal, 2 crítica.
    pub urgency: u8,
    /// Acciones `(clave, etiqueta)` ofrecidas por el emisor. Al clickear una,
    /// el daemon emite la señal `ActionInvoked(id, clave)`.
    #[serde(default)]
    pub actions: Vec<(String, String)>,
    /// Timeout pedido por el cliente: `-1` default del servidor, `0` nunca
    /// expira, `>0` milisegundos explícitos.
    pub timeout_ms: i32,
    /// Momento de recepción (µs desde epoch) — para ordenar el historial.
    pub created_usec: u64,
}

/// Mensajes del loop Elm del daemon. `Clone + Send + 'static` para poder cruzar
/// la frontera de hilo desde el handler D-Bus.
#[derive(Clone)]
pub enum Msg {
    /// Llegó una notificación nueva (o un reemplazo de una existente).
    Entrante(Notificacion),
    /// Venció el timeout de la notificación `id` — sacarla del stack.
    Expira(u32),
    /// El usuario la cerró con un click en el cuerpo del toast.
    Descarta(u32),
    /// El cliente pidió cerrarla vía `CloseNotification` (motivo 3).
    CerrarPorCliente(u32),
    /// El usuario clickeó un botón de acción.
    Accion { id: u32, clave: String },
}

/// Evento del loop de render hacia el hilo D-Bus, para emitir señales del
/// protocolo freedesktop hacia los clientes.
#[derive(Debug, Clone)]
pub enum Cierre {
    /// Emitir `NotificationClosed(id, motivo)`. Motivo: 1 expiró, 2 la cerró el
    /// usuario, 3 vía `CloseNotification`.
    Cerrada { id: u32, motivo: u32 },
    /// Emitir `ActionInvoked(id, clave)`.
    Accion { id: u32, clave: String },
}

/// Instante actual en µs desde epoch (best-effort).
pub fn now_usec() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Inicializa `tracing` con filtro desde el entorno (default `pata_notify=info`).
pub fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("pata_notify=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

/// Levanta el daemon: arranca el render layer-shell (que a su vez lanza el
/// frontend D-Bus en su hilo). Bloquea hasta que la superficie se cierra.
pub fn run() {
    let cfg = llimphi_layer::LayerConfig {
        corner: Some(llimphi_layer::Corner::BottomRight),
        size: Some((app::BOX_W, app::BOX_H)),
        layer: llimphi_layer::LayerKind::Overlay,
        exclusive: false,
        keyboard: llimphi_layer::Keyboard::None,
        namespace: "pata-notify".to_string(),
        ..Default::default()
    };
    if let Err(e) = llimphi_layer::run::<app::Daemon>(cfg) {
        eprintln!("pata-notify · sin wlr-layer-shell: {e}");
    }
}
