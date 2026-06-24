//! El render: una `App` de Llimphi que apila toasts en la esquina inferior
//! derecha. Corre dentro de `llimphi-layer` como caja wlr-layer-shell, y en
//! `init` lanza el frontend D-Bus en su propio hilo con runtime tokio.

use std::time::{Duration, Instant};

use llimphi_ui::{App, Handle, View};
use llimphi_widget_toast::{toast_stack_view, Toast, ToastKind};

use crate::store::Store;
use crate::{dbus, Msg, Notificacion};

/// Ancho de la caja en la esquina (px). `TOAST_W` (320) + márgenes del widget.
pub const BOX_W: u32 = 352;
/// Alto de la caja (px). Cubre ~4 toasts; el resto es transparente. Mantenerla
/// modesta minimiza el área de la esquina que intercepta el puntero.
pub const BOX_H: u32 = 240;

/// Timeout por defecto cuando el cliente manda `-1` (decide el servidor).
const DEFAULT_TIMEOUT_MS: u64 = 5_000;
/// Duración "no expira" para `timeout == 0`: ~10 años, equivale a persistente
/// hasta que el usuario la descarte.
const PERSISTENTE: Duration = Duration::from_secs(10 * 365 * 24 * 3_600);

pub struct Daemon;

pub struct Model {
    /// Toasts vivos en pantalla. El historial completo vive en `sled`, no acá.
    toasts: Vec<Toast>,
}

impl App for Daemon {
    type Model = Model;
    type Msg = Msg;

    fn init(handle: &Handle<Msg>) -> Model {
        let store = Store::open()
            .or_else(|_| Store::temporary())
            .expect("store de notificaciones (ni disco ni memoria)");

        // El frontend D-Bus corre en su propio hilo: zbus es async y el loop de
        // llimphi-layer es bloqueante (sctk). El handler reentra acá vía el
        // handle clonado.
        let h = handle.clone();
        let st = store.clone();
        std::thread::Builder::new()
            .name("pata-notify-dbus".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.block_on(dbus::serve(h, st)),
                    Err(e) => eprintln!("pata-notify · sin runtime tokio: {e}"),
                }
            })
            .expect("hilo del frontend D-Bus");

        Model { toasts: Vec::new() }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Entrante(n) => {
                let id = n.id as u64;
                let kind = kind_de_urgencia(n.urgency);
                let (dur, persistente) = match n.timeout_ms {
                    0 => (PERSISTENTE, true),
                    t if t < 0 => (Duration::from_millis(DEFAULT_TIMEOUT_MS), false),
                    t => (Duration::from_millis(t as u64), false),
                };
                let toast = Toast {
                    id,
                    kind,
                    text: texto(&n),
                    expires_at: Instant::now() + dur,
                };
                // replaces_id: reemplazá el slot si ya existe ese id.
                if let Some(slot) = model.toasts.iter_mut().find(|t| t.id == id) {
                    *slot = toast;
                } else {
                    model.toasts.push(toast);
                }
                // Programá el vencimiento salvo que sea persistente.
                if !persistente {
                    let nid = n.id;
                    handle.spawn(move || {
                        std::thread::sleep(dur);
                        Msg::Expira(nid)
                    });
                }
            }
            Msg::Expira(id) | Msg::Descarta(id) => {
                model.toasts.retain(|t| t.id != id as u64);
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let ahora = Instant::now();
        let vivos: Vec<Toast> = model
            .toasts
            .iter()
            .filter(|t| t.is_alive(ahora))
            .cloned()
            .collect();
        toast_stack_view(&vivos, (BOX_W as f32, BOX_H as f32), |id| {
            Msg::Descarta(id as u32)
        })
    }

    fn app_id() -> Option<&'static str> {
        Some("pata-notify")
    }

    fn title() -> &'static str {
        "pata-notify"
    }
}

/// Mapea la urgencia freedesktop a la severidad del toast. Sin urgencias para
/// success/warning en el spec: crítica → Error, el resto → Info neutro.
fn kind_de_urgencia(urgency: u8) -> ToastKind {
    match urgency {
        2 => ToastKind::Error,
        _ => ToastKind::Info,
    }
}

/// Arma el texto del toast a partir de summary + body.
fn texto(n: &Notificacion) -> String {
    match (n.summary.trim().is_empty(), n.body.trim().is_empty()) {
        (false, false) => format!("{} — {}", n.summary, n.body),
        (false, true) => n.summary.clone(),
        (true, false) => n.body.clone(),
        (true, true) => String::new(),
    }
}
