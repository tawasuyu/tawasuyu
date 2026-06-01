//! Seguimiento de ventanas abiertas vía **wlr-foreign-toplevel-management**, el
//! protocolo que usan waybar/eww para enumerar y activar toplevels en cualquier
//! compositor wlroots (Hyprland, Sway, river…).
//!
//! El compositor anuncia un `zwlr_foreign_toplevel_handle_v1` por cada ventana y
//! le manda sus atributos (título, app_id, estado) en eventos sueltos que se
//! confirman con `done`. Aquí acumulamos esos atributos en un [`Toplevel`] y los
//! aplicamos de golpe al recibir `done`, para no pintar estados a medias. El
//! cableado Wayland (los `Dispatch`) vive en [`crate::layer`], que es quien tiene
//! el `QueueHandle`; este módulo sólo modela el dato.
//!
//! La activación —traer la ventana al frente— es interacción, igual que el
//! `shuma_input`: por eso `window_list` no pasa por el `build` agnóstico de
//! `pata-core`, sino que lo intercepta el frontend (ver [`crate::SlotWidget`]).

use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::{
    State, ZwlrForeignToplevelHandleV1,
};

/// El bit `activated` del array de estados que manda el evento `state`.
const ESTADO_ACTIVADO: u32 = State::Activated as u32;

/// Una ventana reportada por el compositor. Los campos `p_*` acumulan lo que
/// llega entre `done`s; [`Toplevel::confirmar`] los vuelca a los definitivos.
pub struct Toplevel {
    /// Identificador estable que viaja en [`crate::Msg::ActivateWindow`]. Es un
    /// contador local (no el ObjectId, que no es `Clone`-friendly para el `Msg`).
    pub id: u32,
    /// El handle del protocolo: por él se activa/cierra la ventana.
    pub handle: ZwlrForeignToplevelHandleV1,
    /// Título de la ventana, ya confirmado.
    pub title: String,
    /// `app_id` (clase de la app), ya confirmado.
    pub app_id: String,
    /// `true` si es la ventana activa.
    pub activated: bool,
    p_title: Option<String>,
    p_app_id: Option<String>,
    p_activated: Option<bool>,
}

impl Toplevel {
    /// Una ventana recién anunciada, todavía sin atributos.
    pub fn new(id: u32, handle: ZwlrForeignToplevelHandleV1) -> Self {
        Self {
            id,
            handle,
            title: String::new(),
            app_id: String::new(),
            activated: false,
            p_title: None,
            p_app_id: None,
            p_activated: None,
        }
    }

    /// Guarda el título pendiente (evento `title`).
    pub fn set_title(&mut self, title: String) {
        self.p_title = Some(title);
    }

    /// Guarda el `app_id` pendiente (evento `app_id`).
    pub fn set_app_id(&mut self, app_id: String) {
        self.p_app_id = Some(app_id);
    }

    /// Decodifica el array de estados (`u32` little-endian empaquetados en bytes)
    /// y registra si la ventana quedó activa (evento `state`).
    pub fn set_state(&mut self, bytes: &[u8]) {
        let activado = bytes
            .chunks_exact(4)
            .any(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]) == ESTADO_ACTIVADO);
        self.p_activated = Some(activado);
    }

    /// Aplica lo acumulado (evento `done`). Devuelve `true` si algo cambió, para
    /// que el caller sepa si tiene que re-pintar.
    pub fn confirmar(&mut self) -> bool {
        let mut cambio = false;
        if let Some(t) = self.p_title.take() {
            if t != self.title {
                self.title = t;
                cambio = true;
            }
        }
        if let Some(a) = self.p_app_id.take() {
            if a != self.app_id {
                self.app_id = a;
                cambio = true;
            }
        }
        if let Some(act) = self.p_activated.take() {
            if act != self.activated {
                self.activated = act;
                cambio = true;
            }
        }
        cambio
    }

    /// La etiqueta a mostrar: el título si lo hay, si no el `app_id`, si no un
    /// genérico. Nunca vacía (un chip vacío no se podría clickear).
    pub fn etiqueta(&self) -> String {
        if !self.title.is_empty() {
            self.title.clone()
        } else if !self.app_id.is_empty() {
            self.app_id.clone()
        } else {
            "ventana".to_string()
        }
    }
}

/// Lo que el render necesita de cada ventana: el `id` para el click, la etiqueta
/// y si está activa (para resaltarla). Desacopla el pincel del protocolo Wayland.
#[derive(Clone, Debug)]
pub struct WindowEntry {
    /// Identificador estable (el de [`Toplevel::id`]).
    pub id: u32,
    /// Texto a pintar en el chip.
    pub label: String,
    /// `true` si es la ventana activa (chip resaltado).
    pub active: bool,
}
