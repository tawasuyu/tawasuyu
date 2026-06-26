//! `zwlr_gamma_control_unstable_v1` — control de gamma por salida, implementado
//! a mano (smithay 0.7 sólo trae los bindings del XML, como con `screencopy`).
//!
//! Lo usan `wlsunset`/`gammastep` (la «luz nocturna»): el cliente pide un control
//! de gamma para una salida, el compositor le dice el **tamaño** del LUT de esa
//! salida (`gamma_size`), y el cliente manda por un fd una rampa de `3·size`
//! `u16` (R, G, B). El compositor la aplica al **CRTC** vía DRM. Al soltar el
//! control (o al desconectarse el cliente) la gamma vuelve a la identidad.
//!
//! Sólo el backend **DRM** tiene CRTC con LUT de gamma; en winit (anidado) no hay
//! gamma → al pedir un control para una salida sin tamaño registrado se responde
//! `failed`. El tamaño lo siembra el backend DRM en el `user_data` de cada
//! [`Output`] al crearla ([`GammaSize`]); la aplicación de la rampa la hace el
//! backend drenando [`App::pending_gamma`] (mismo patrón que DPMS/sesión: el
//! protocolo —que corre sobre `App`— no toca el hardware, sólo deja el pedido).
//!
//! Runtime no verificable headless (norma de mirada): la aplicación legacy de
//! `set_gamma` sobre un CRTC en modesetting atómico depende del driver.

use std::io::Read;

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::gamma_control::v1::server::{
    zwlr_gamma_control_manager_v1::{self, ZwlrGammaControlManagerV1},
    zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
};
use smithay::reexports::wayland_server::{
    backend::{ClientId, GlobalId},
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use crate::App;

/// Versión del global (el protocolo wlr-gamma-control está en v1).
const VERSION: u32 = 1;

/// Tamaño del LUT de gamma de una salida (entradas por canal). Lo siembra el
/// backend DRM en el `user_data` de la [`Output`]; en winit no se siembra (no hay
/// gamma), así que su ausencia significa «esta salida no soporta gamma».
#[derive(Debug, Clone, Copy)]
pub struct GammaSize(pub u32);

/// Una rampa de gamma ya partida por canal, lista para `DrmDevice::set_gamma`.
#[derive(Debug, Clone)]
pub struct GammaRamp {
    pub red: Vec<u16>,
    pub green: Vec<u16>,
    pub blue: Vec<u16>,
}

/// Estado del protocolo — sólo retiene el global vivo.
pub struct GammaControlState {
    _global: GlobalId,
}

impl GammaControlState {
    /// Crea el global `zwlr_gamma_control_manager_v1`.
    pub fn new(dh: &DisplayHandle) -> Self {
        let global = dh.create_global::<App, ZwlrGammaControlManagerV1, _>(VERSION, ());
        Self { _global: global }
    }
}

/// User data de cada `zwlr_gamma_control_v1`: a qué salida controla (o `None` si
/// nació fallido) y el tamaño de LUT que se le anunció (para validar el fd).
pub struct GammaControlData {
    output: Option<Output>,
    size: u32,
}

impl GlobalDispatch<ZwlrGammaControlManagerV1, ()> for App {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrGammaControlManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrGammaControlManagerV1, ()> for App {
    fn request(
        state: &mut Self,
        _client: &Client,
        _mgr: &ZwlrGammaControlManagerV1,
        request: zwlr_gamma_control_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_gamma_control_manager_v1::Request::GetGammaControl { id, output } => {
                let salida = Output::from_resource(&output);
                // Tamaño del LUT: lo sembró el backend DRM en el user_data. Sin él
                // (winit, o salida muerta) la salida no soporta gamma → `failed`.
                let size = salida
                    .as_ref()
                    .and_then(|o| o.user_data().get::<GammaSize>().map(|g| g.0));
                // Activable sólo si: la salida existe, tiene tamaño de gamma
                // (backend DRM), y no hay ya un control activo para ella (un solo
                // dueño de la gamma por salida, como wlroots).
                let libre = size.is_some()
                    && salida
                        .as_ref()
                        .is_some_and(|o| !state.gamma_active.iter().any(|(out, _)| out == o));
                if let (true, Some(o), Some(sz)) = (libre, salida.clone(), size) {
                    let ctl = data_init.init(id, GammaControlData { output: Some(o.clone()), size: sz });
                    ctl.gamma_size(sz);
                    state.gamma_active.push((o, ctl));
                } else {
                    // Nace fallido (winit, salida muerta, o ya hay dueño): igual
                    // hay que inicializar el `New<>`, luego `failed`.
                    let ctl = data_init.init(id, GammaControlData { output: None, size: 0 });
                    ctl.failed();
                }
            }
            zwlr_gamma_control_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<ZwlrGammaControlV1, GammaControlData> for App {
    fn request(
        state: &mut Self,
        _client: &Client,
        ctl: &ZwlrGammaControlV1,
        request: zwlr_gamma_control_v1::Request,
        data: &GammaControlData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_gamma_control_v1::Request::SetGamma { fd } => {
                // Sólo el control activo de la salida puede fijar gamma (uno que
                // ya recibió `failed` no está en `gamma_active`).
                if !state.gamma_active.iter().any(|(_, c)| c == ctl) {
                    return;
                }
                let Some(output) = data.output.clone() else { return };
                match leer_rampa(fd, data.size) {
                    Some(rampa) => state.pending_gamma.push((output, Some(rampa))),
                    None => {
                        // Longitud equivocada = error de protocolo tipificado.
                        ctl.post_error(
                            zwlr_gamma_control_v1::Error::InvalidGamma,
                            "la rampa no tiene 3·gamma_size entradas u16",
                        );
                    }
                }
            }
            zwlr_gamma_control_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        ctl: &ZwlrGammaControlV1,
        data: &GammaControlData,
    ) {
        // Si era el control activo de su salida, lo soltamos y pedimos restaurar
        // la gamma identidad (None = reset) en el backend.
        if let Some(pos) = state.gamma_active.iter().position(|(_, c)| c == ctl) {
            state.gamma_active.remove(pos);
            if let Some(output) = data.output.clone() {
                state.pending_gamma.push((output, None));
            }
        }
    }
}

/// Lee del `fd` una rampa de `3·size` `u16` en orden nativo (R, G, B). `None` si
/// la longitud no cuadra exactamente (el cliente mintió el tamaño).
fn leer_rampa(fd: std::os::fd::OwnedFd, size: u32) -> Option<GammaRamp> {
    let size = size as usize;
    if size == 0 {
        return None;
    }
    let mut buf = Vec::new();
    let mut f = std::fs::File::from(fd);
    f.read_to_end(&mut buf).ok()?;
    if buf.len() != size * 3 * 2 {
        return None;
    }
    let leer_canal = |off: usize| -> Vec<u16> {
        (0..size)
            .map(|i| {
                let b = off + i * 2;
                u16::from_ne_bytes([buf[b], buf[b + 1]])
            })
            .collect()
    };
    Some(GammaRamp {
        red: leer_canal(0),
        green: leer_canal(size * 2),
        blue: leer_canal(size * 4),
    })
}

/// Construye la rampa **identidad** (gamma neutra) de `size` entradas: una
/// interpolación lineal de 0 a 65535. La usa el backend para restaurar la gamma
/// cuando un control se suelta.
pub fn identidad(size: u32) -> GammaRamp {
    let n = size.max(1);
    let canal: Vec<u16> = (0..n)
        .map(|i| ((i as u64 * 65535) / (n as u64 - 1).max(1)) as u16)
        .collect();
    GammaRamp {
        red: canal.clone(),
        green: canal.clone(),
        blue: canal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identidad_va_de_cero_a_max() {
        let r = identidad(256);
        assert_eq!(r.red.len(), 256);
        assert_eq!(r.red[0], 0);
        assert_eq!(*r.red.last().unwrap(), 65535);
        // Monótona creciente.
        assert!(r.red.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn identidad_size_1_no_panica() {
        let r = identidad(1);
        assert_eq!(r.red, vec![0]);
    }
}
