//! `ext_idle_notify_v1` — avisa a clientes externos cuándo el seat entra y sale
//! de **inactividad** (lo usan `swayidle` y apps que fijan «ausente»/presencia).
//!
//! Implementado **a mano y conducido por el tick de mirada**
//! ([`App::drive_idle_notifs`], llamado desde [`App::idle_tick`]), no por timers
//! de `calloop` como hace el `IdleNotifierState` de smithay. Por qué: ese estado
//! exige un `LoopHandle<'static, App>` (su timer dispara con `&mut App`) y un
//! `Dispatch` sobre `App`, pero los bucles de mirada despachan `DrmState`/winit
//! —tipos distintos del estado de protocolo (`App`)—. Unificarlos es un refactor
//! grande; conducir las notificaciones desde el tick —que **ya** corre la
//! política de inactividad con el mismo reloj de ocio— lo evita por completo y
//! reusa el cómputo de inhibición (`zwp_idle_inhibit`) que ya hay.
//!
//! Cada notificación lleva su propio `timeout` (lo elige el cliente,
//! independiente de los umbrales de apagado/bloqueo de mirada). `idled` se emite
//! al cruzar el timeout sin actividad; `resumed`, al primer input. Las creadas
//! con `get_input_idle_notification` (v2) **ignoran** los inhibidores: sólo el
//! input real las reanima.

use smithay::reexports::wayland_protocols::ext::idle_notify::v1::server::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::{self, ExtIdleNotifierV1},
};
use smithay::reexports::wayland_server::{
    backend::ClientId, backend::GlobalId, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch,
    New, Resource,
};

use crate::App;

/// v2 trae `get_input_idle_notification` (las que ignoran inhibidores).
const VERSION: u32 = 2;

/// Estado runtime de una notificación viva. Vive en [`App::idle_notifs`] (no en
/// el `user_data` del recurso) porque el reloj de ocio se actualiza por mutación
/// desde el tick; el `user_data` del recurso es sólo lectura compartida.
pub(crate) struct IdleNotif {
    /// El recurso del cliente, para emitir `idled`/`resumed`.
    pub res: ExtIdleNotificationV1,
    /// Umbral en ms que pidió el cliente.
    pub timeout_ms: u64,
    /// `true` para `get_input_idle_notification`: los inhibidores no la pausan.
    pub ignore_inhibitor: bool,
    /// Ms de ocio acumulados desde la última actividad.
    pub elapsed_ms: u64,
    /// Si ya emitió `idled` y aún no `resumed` (edge-triggered).
    pub idle: bool,
}

/// Mantiene vivo el global `ext_idle_notifier_v1` durante la sesión.
pub struct IdleNotifyState {
    _global: GlobalId,
}

impl IdleNotifyState {
    /// Crea el global. Sin filtro por ejecutable: saber «idle» no es una fuga de
    /// privacidad como screencopy (no expone contenido), igual que gamma.
    pub fn new(dh: &DisplayHandle) -> Self {
        let global = dh.create_global::<App, ExtIdleNotifierV1, _>(VERSION, ());
        Self { _global: global }
    }
}

/// Flanco a emitir tras un paso (mantiene la lógica pura y testeable, separada
/// del envío del evento Wayland).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    None,
    Idled,
    Resumed,
}

/// Avance puro de **un** reloj de ocio. Dado el estado `(elapsed, idle)` y el
/// paso `dt`, con `paused` = inhibido y la notificación respeta inhibidores,
/// devuelve `(elapsed', idle', flanco)`. Sin Wayland → testeable.
pub(crate) fn step(
    elapsed_ms: u64,
    idle: bool,
    timeout_ms: u64,
    dt_ms: u64,
    paused: bool,
) -> (u64, bool, Edge) {
    if paused {
        // Inhibidor activo: cuenta como actividad continua (rearma; reanima).
        return (0, false, if idle { Edge::Resumed } else { Edge::None });
    }
    let e = elapsed_ms.saturating_add(dt_ms);
    if !idle && e >= timeout_ms {
        (e, true, Edge::Idled)
    } else {
        (e, idle, Edge::None)
    }
}

impl App {
    /// Avanza el reloj de ocio de cada notificación y emite los flancos. Lo llama
    /// [`App::idle_tick`] con el `dt` del backend y si hay inhibición activa.
    pub(crate) fn drive_idle_notifs(&mut self, dt_ms: u64, inhibited: bool) {
        for n in &mut self.idle_notifs {
            let paused = inhibited && !n.ignore_inhibitor;
            let (elapsed, idle, edge) = step(n.elapsed_ms, n.idle, n.timeout_ms, dt_ms, paused);
            n.elapsed_ms = elapsed;
            n.idle = idle;
            match edge {
                Edge::Idled => n.res.idled(),
                Edge::Resumed => n.res.resumed(),
                Edge::None => {}
            }
        }
    }

    /// Input del usuario: reinicia el ocio y reanima lo que estaba idle. Lo llama
    /// [`App::idle_activity`] (en cada evento de libinput de cada backend).
    pub(crate) fn idle_notify_activity(&mut self) {
        for n in &mut self.idle_notifs {
            if n.idle {
                n.res.resumed();
                n.idle = false;
            }
            n.elapsed_ms = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{step, Edge};

    #[test]
    fn acumula_y_dispara_idled() {
        // Sin cruzar el umbral: acumula, sin flanco.
        let (e, idle, edge) = step(0, false, 1000, 400, false);
        assert_eq!((e, idle, edge), (400, false, Edge::None));
        // Al cruzar: idled, una sola vez.
        let (e, idle, edge) = step(800, false, 1000, 400, false);
        assert_eq!((e, idle, edge), (1200, true, Edge::Idled));
        // Ya idle: no re-dispara.
        let (_, idle, edge) = step(1200, true, 1000, 400, false);
        assert_eq!((idle, edge), (true, Edge::None));
    }

    #[test]
    fn inhibidor_reanima_y_rearma() {
        // Pausado e idle → resumed + reloj a cero.
        assert_eq!(step(5000, true, 1000, 100, true), (0, false, Edge::Resumed));
        // Pausado y no-idle → sólo rearma, sin flanco.
        assert_eq!(step(500, false, 1000, 100, true), (0, false, Edge::None));
    }

    #[test]
    fn timeout_cero_idlea_de_una() {
        // timeout 0 = ocioso de inmediato (al primer paso).
        assert_eq!(step(0, false, 0, 0, false), (0, true, Edge::Idled));
    }
}

impl GlobalDispatch<ExtIdleNotifierV1, ()> for App {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ExtIdleNotifierV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for App {
    fn request(
        state: &mut Self,
        _client: &Client,
        _notifier: &ExtIdleNotifierV1,
        request: ext_idle_notifier_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            // Respeta inhibidores (un reproductor que inhibe el ocio la mantiene
            // «activa»).
            ext_idle_notifier_v1::Request::GetIdleNotification { id, timeout, seat: _ } => {
                let res = data_init.init(id, ());
                state.idle_notifs.push(IdleNotif {
                    res,
                    timeout_ms: timeout as u64,
                    ignore_inhibitor: false,
                    elapsed_ms: 0,
                    idle: false,
                });
            }
            // Ignora inhibidores: sólo el input real cuenta como actividad.
            ext_idle_notifier_v1::Request::GetInputIdleNotification { id, timeout, seat: _ } => {
                let res = data_init.init(id, ());
                state.idle_notifs.push(IdleNotif {
                    res,
                    timeout_ms: timeout as u64,
                    ignore_inhibitor: true,
                    elapsed_ms: 0,
                    idle: false,
                });
            }
            ext_idle_notifier_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for App {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _res: &ExtIdleNotificationV1,
        request: ext_idle_notification_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // La única request es `destroy`; la limpieza la hace `destroyed`.
        match request {
            ext_idle_notification_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, res: &ExtIdleNotificationV1, _data: &()) {
        state.idle_notifs.retain(|n| &n.res != res);
    }
}
