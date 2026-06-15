//! `zwlr_foreign_toplevel_management_v1` — servidor, implementado a mano.
//!
//! smithay 0.7 sólo trae `ext_foreign_toplevel_list` (censo de ventanas,
//! **solo-listar**); el protocolo wlr —que además permite **activar** y
//! **cerrar** ventanas desde una taskbar— no tiene lógica de servidor en
//! smithay, así que el dispatch vive acá. Es lo que `pata` (la barra) habla en
//! layer-shell para pintar el `window_list` y enfocar/cerrar al clickear.
//!
//! Espeja los mismos hooks de ciclo de vida que alimentan el censo ext (alta
//! de ventana, cambio de título/`app_id`, foco, baja): cada uno reenvía a los
//! handles wlr vivos. El global nace **gateado por ejecutable**
//! (`Permisos.window_list_*`), igual que el censo ext: al denegado no se le
//! anuncia —frontera física, no tabla eludible—.
//!
//! Un handle wlr es **por binding del manager**: cada cliente que bindea el
//! manager recibe su propio handle por ventana. Por eso cada [`ManagedWindow`]
//! guarda un `Vec` de handles (uno por manager). Al cerrarse la ventana se les
//! manda `closed`; al destruirse un manager, se purgan sus handles.

pub use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1;
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1,
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use smithay::reexports::wayland_server::{
    backend::{ClientId, GlobalId},
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

use crate::estado::ManagedWindow;
use crate::App;

/// v3 = `parent`. Sin parentelado por ahora (lo mandamos vacío implícitamente),
/// pero anunciamos v3 para que clientes nuevos (waybar, pata) bindeen pleno.
const VERSION: u32 = 3;

/// Valor `activated` del enum `state` del protocolo (el orden del XML:
/// maximized=0, minimized=1, activated=2, fullscreen=3). Se manda como array
/// de u32 en endianness nativa (wayland es IPC local).
const STATE_ACTIVATED: u32 = 2;

/// Datos del global: el filtro de visibilidad por cliente (la frontera).
pub struct ForeignToplevelManagerGlobalData {
    filtro: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// Estado del protocolo: el global (para mantenerlo vivo) y los managers
/// bindeados vivos, a los que se les anuncian altas/cambios.
pub struct ForeignToplevelManagerState {
    _global: GlobalId,
    instances: Vec<ZwlrForeignToplevelManagerV1>,
}

impl ForeignToplevelManagerState {
    /// Crea el global `zwlr_foreign_toplevel_manager_v1`, gateado por `filtro`.
    pub fn new<F>(dh: &DisplayHandle, filtro: F) -> Self
    where
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let _global = dh.create_global::<App, ZwlrForeignToplevelManagerV1, _>(
            VERSION,
            ForeignToplevelManagerGlobalData {
                filtro: Box::new(filtro),
            },
        );
        Self {
            _global,
            instances: Vec::new(),
        }
    }
}

/// User data de cada handle: a qué ventana del compositor representa.
pub struct ToplevelHandleData {
    pub window_id: u64,
}

/// El `app_id` actual de una ventana, leído de su `XdgToplevelSurfaceData`
/// (no se cachea en [`ManagedWindow`]). Vacío si el cliente no lo fijó.
fn app_id_de(w: &ManagedWindow) -> String {
    with_states(&w.surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|d| d.lock().ok())
            .and_then(|d| d.app_id.clone())
            .unwrap_or_default()
    })
}

/// Manda el evento `state` (sólo `activated`, o vacío) seguido de `done`.
fn enviar_estado(handle: &ZwlrForeignToplevelHandleV1, focused: bool) {
    let mut st = Vec::new();
    if focused {
        st.extend_from_slice(&STATE_ACTIVATED.to_ne_bytes());
    }
    handle.state(st);
}

/// Crea un handle para la ventana sobre un manager y le manda el estado
/// inicial completo (`title`/`app_id`/`state`/`done`). `None` si el manager
/// ya no tiene cliente o falló crear el recurso.
fn anunciar_en(
    dh: &DisplayHandle,
    manager: &ZwlrForeignToplevelManagerV1,
    id: u64,
    title: &str,
    app_id: &str,
    focused: bool,
) -> Option<ZwlrForeignToplevelHandleV1> {
    let client = manager.client()?;
    let handle = client
        .create_resource::<ZwlrForeignToplevelHandleV1, ToplevelHandleData, App>(
            dh,
            manager.version(),
            ToplevelHandleData { window_id: id },
        )
        .ok()?;
    manager.toplevel(&handle);
    handle.title(title.to_string());
    handle.app_id(app_id.to_string());
    enviar_estado(&handle, focused);
    handle.done();
    Some(handle)
}

/// Anuncia la ventana `id` a todos los managers wlr bindeados. Se llama al
/// mapear una ventana nueva. No-op para la ventana del shell (la barra no se
/// lista a sí misma) y si no hay managers.
pub(crate) fn anunciar_ventana(app: &mut App, id: u64) {
    let instances = app.foreign_toplevel_manager.instances.clone();
    if instances.is_empty() {
        return;
    }
    let dh = app.dh.clone();
    let Some(w) = app.windows.iter().find(|w| w.id == id) else {
        return;
    };
    if w.is_shell {
        return;
    }
    let (title, app_id, focused) = (w.title.clone(), app_id_de(w), w.focused);
    let nuevos: Vec<_> = instances
        .iter()
        .filter_map(|m| anunciar_en(&dh, m, id, &title, &app_id, focused))
        .collect();
    if let Some(w) = app.windows.iter_mut().find(|w| w.id == id) {
        w.wlr_handles.extend(nuevos);
    }
}

/// Reenvía el título a los handles wlr de la ventana `id`.
pub(crate) fn actualizar_titulo(app: &mut App, id: u64, title: &str) {
    if let Some(w) = app.windows.iter().find(|w| w.id == id) {
        for h in &w.wlr_handles {
            h.title(title.to_string());
            h.done();
        }
    }
}

/// Reenvía el `app_id` a los handles wlr de la ventana `id`.
pub(crate) fn actualizar_app_id(app: &mut App, id: u64, app_id: &str) {
    if let Some(w) = app.windows.iter().find(|w| w.id == id) {
        for h in &w.wlr_handles {
            h.app_id(app_id.to_string());
            h.done();
        }
    }
}

/// Reemite el estado (`activated`) de todas las ventanas a sus handles wlr.
/// Se llama tras cada cambio de foco para que la taskbar resalte la activa.
pub(crate) fn refrescar_estados(app: &mut App) {
    for w in &app.windows {
        for h in &w.wlr_handles {
            enviar_estado(h, w.focused);
            h.done();
        }
    }
}

/// Manda `closed` a los handles de una ventana que se cierra (los handles van
/// dentro de la `ManagedWindow` ya removida, así que se pasan por valor).
pub(crate) fn cerrar_handles(handles: &[ZwlrForeignToplevelHandleV1]) {
    for h in handles {
        h.closed();
    }
}

impl GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignToplevelManagerGlobalData> for App {
    fn bind(
        state: &mut Self,
        dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _global_data: &ForeignToplevelManagerGlobalData,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let manager = data_init.init(resource, ());
        state.foreign_toplevel_manager.instances.push(manager.clone());
        // Censo inicial: anuncia todas las ventanas del usuario ya abiertas.
        let dh = dh.clone();
        let infos: Vec<(u64, String, String, bool)> = state
            .windows
            .iter()
            .filter(|w| !w.is_shell)
            .map(|w| (w.id, w.title.clone(), app_id_de(w), w.focused))
            .collect();
        for (id, title, app_id, focused) in infos {
            if let Some(h) = anunciar_en(&dh, &manager, id, &title, &app_id, focused) {
                if let Some(w) = state.windows.iter_mut().find(|w| w.id == id) {
                    w.wlr_handles.push(h);
                }
            }
        }
    }

    fn can_view(client: Client, global_data: &ForeignToplevelManagerGlobalData) -> bool {
        (global_data.filtro)(&client)
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for App {
    fn request(
        state: &mut Self,
        _client: &Client,
        manager: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            manager.finished();
            state
                .foreign_toplevel_manager
                .instances
                .retain(|m| m != manager);
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        manager: &ZwlrForeignToplevelManagerV1,
        _data: &(),
    ) {
        state
            .foreign_toplevel_manager
            .instances
            .retain(|m| m != manager);
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ToplevelHandleData> for App {
    fn request(
        state: &mut Self,
        _client: &Client,
        handle: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        data: &ToplevelHandleData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let id = data.window_id;
        match request {
            zwlr_foreign_toplevel_handle_v1::Request::Activate { .. } => {
                state.activar_ventana(id);
            }
            zwlr_foreign_toplevel_handle_v1::Request::Close => {
                if let Some(w) = state.windows.iter().find(|w| w.id == id) {
                    w.toplevel.send_close();
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::Destroy => {
                for w in state.windows.iter_mut() {
                    w.wlr_handles.retain(|h| h != handle);
                }
            }
            // set_maximized/minimized/fullscreen/rectangle: aún no mapeados al
            // Cerebro (el teselado no expone esos estados); no-op por ahora.
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        handle: &ZwlrForeignToplevelHandleV1,
        _data: &ToplevelHandleData,
    ) {
        for w in state.windows.iter_mut() {
            w.wlr_handles.retain(|h| h != handle);
        }
    }
}
