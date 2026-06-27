//! `mirada-compositor` — el Cuerpo del compositor mirada.
//!
//! Un compositor Wayland teselante real, sobre `smithay`, con backend
//! `winit`: corre **anidado** como una ventana dentro de tu sesión
//! gráfica actual (X11 o Wayland). Habla el protocolo Wayland con los
//! clientes, compone sus superficies y aplica la geometría que decide el
//! Cerebro.
//!
//! Dos modos:
//!
//! - **Autónomo** (por defecto): lleva un [`Desktop`] embebido — es un
//!   compositor teselante completo en un solo proceso. Lánzalo y abre
//!   clientes; el teclado (`Super+…`) maneja el escritorio.
//! - **Enlazado** (`MIRADA_SOCKET=/ruta`): el Cuerpo escucha ahí y la
//!   app `mirada` (el Cerebro GPUI) se conecta; la geometría viaja por
//!   [`mirada_link`].
//!
//! Cómo probarlo en un Linux real: ver `crates/apps/mirada-compositor/README.md`.

use std::sync::Arc;
use std::time::Instant;

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::input::{InputEvent, KeyState, KeyboardKeyEvent};
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::{
    draw_render_elements, on_commit_buffer_handler, with_renderer_surface_state,
};
use smithay::backend::renderer::{Color32F, Frame, ImportDma, Renderer};
use smithay::backend::winit::{self, WinitEvent};
use smithay::input::keyboard::{xkb, FilterResult, KeyboardHandle, Keysym, ModifiersState};
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData, PointerHandle};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_buffer;
use smithay::reexports::wayland_server::protocol::wl_output;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, Display, DisplayHandle, ListeningSocket, Resource,
};
use smithay::reexports::winit::platform::pump_events::PumpStatus;
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER};
use smithay::utils::{Serial, Transform};
use smithay::backend::egl::EGLDevice;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::dmabuf::{
    DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier,
};
use smithay::wayland::compositor::{
    get_parent, with_states, with_surface_tree_downward, CompositorClientState,
    CompositorHandler, CompositorState, SurfaceAttributes, TraversalAction,
};
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
};
use smithay::wayland::selection::wlr_data_control::{DataControlHandler, DataControlState};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::shell::wlr_layer::{
    KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData,
    WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState;
use smithay::desktop::{layer_map_for_output, LayerSurface as DesktopLayerSurface, WindowSurfaceType};
use smithay::output::Output;
use smithay::{
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_dmabuf,
    delegate_foreign_toplevel_list, delegate_layer_shell, delegate_output, delegate_seat,
    delegate_shm, delegate_virtual_keyboard_manager, delegate_xdg_decoration, delegate_xdg_shell,
};

use auth_core::{SessionTicket, UserInfo};
use mirada_body::{BodyOp, BodyState};
use mirada_brain::{
    BodyEvent, BrainCommand, CtlReply, CtlRequest, CtlServer, Desktop, Keymap, Rules,
};
use mirada_link::BodyLink;

mod cube;
mod cursor_theme;
#[macro_use]
mod diag;
mod drm_backend;
mod gamma_control;
mod handoff;
mod idle_notify;
mod menu;
mod screencopy;
mod thumbs;
mod foreign_toplevel;
mod switcher;
mod text;
mod zone_clipboard;

mod estado;
mod operaciones;
mod handlers;
mod cliente;
mod utilidades;
mod setup;
mod bucle_winit;

pub(crate) use estado::*;
pub(crate) use cliente::*;
pub(crate) use utilidades::*;
pub(crate) use setup::*;

fn main() {
    // Telemetría primero: el panic hook y la bitácora deben estar en pie
    // antes de tocar nada, para que cualquier fallo del arranque ya quede
    // en disco (en el directorio local persistente, no /tmp).
    // `bitacora` captura el firehose crudo de stderr (los eprintln!) + panics;
    // `diag` mantiene la bitácora estructurada (eventos.log + migas + crash-N).
    bitacora::abrir("mirada");
    diag::init();

    // Banderas en cualquier orden: `--greeter` (modo DM) es ortogonal
    // al backend (`--winit` anidado · `--drm` nativo · auto si falta).
    let args: Vec<String> = std::env::args().skip(1).collect();
    for a in &args {
        if !matches!(a.as_str(), "--greeter" | "--winit" | "--drm") {
            eprintln!(
                "mirada-compositor: opción desconocida «{a}» — usa --greeter, --winit o --drm"
            );
            std::process::exit(2);
        }
    }
    let greeter = args.iter().any(|a| a == "--greeter");
    let backend = args.iter().find(|a| matches!(a.as_str(), "--winit" | "--drm"));

    let result = match backend.map(String::as_str) {
        Some("--drm") => drm_backend::run(greeter),
        Some("--winit") => bucle_winit::run_winit(greeter),
        _ => {
            // Auto: con sesión gráfica anfitriona → winit (anidado);
            // sin ella (una TTY pelada) → backend DRM.
            let nested = std::env::var_os("WAYLAND_DISPLAY").is_some()
                || std::env::var_os("DISPLAY").is_some();
            if nested {
                println!("mirada-compositor · sesión gráfica detectada → backend winit.");
                bucle_winit::run_winit(greeter)
            } else {
                println!("mirada-compositor · sin sesión gráfica → backend DRM.");
                drm_backend::run(greeter)
            }
        }
    };
    if let Err(e) = result {
        eprintln!("mirada-compositor · error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vt_switch_cubre_fn_y_keysym_dedicado() {
        let ctrl_alt = ModifiersState {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        let none = ModifiersState::default();
        // Ctrl+Alt+F3 → VT3.
        assert_eq!(vt_target(&ctrl_alt, Keysym::new(xkb::keysyms::KEY_F3)), Some(3));
        // F3 sin modificadores no conmuta.
        assert_eq!(vt_target(&none, Keysym::new(xkb::keysyms::KEY_F3)), None);
        // El keysym dedicado conmuta por sí mismo (keymaps con srvr_ctrl).
        assert_eq!(
            vt_target(&none, Keysym::new(xkb::keysyms::KEY_XF86Switch_VT_5)),
            Some(5)
        );
        // Otras teclas y F-keys fuera de rango → None.
        assert_eq!(vt_target(&ctrl_alt, Keysym::new(xkb::keysyms::KEY_a)), None);
        assert_eq!(vt_target(&ctrl_alt, Keysym::new(xkb::keysyms::KEY_F13)), None);
    }

    #[test]
    fn anchor_parse_y_default() {
        assert_eq!(ShellAnchor::parse("top"), ShellAnchor::Top);
        assert_eq!(ShellAnchor::parse("LEFT"), ShellAnchor::Left);
        assert_eq!(ShellAnchor::parse("right"), ShellAnchor::Right);
        // desconocido o vacío → bottom.
        assert_eq!(ShellAnchor::parse("xyz"), ShellAnchor::Bottom);
        assert_eq!(ShellAnchor::parse(""), ShellAnchor::Bottom);
    }

    #[test]
    fn anchor_horizontalidad() {
        assert!(ShellAnchor::Top.es_horizontal());
        assert!(ShellAnchor::Bottom.es_horizontal());
        assert!(!ShellAnchor::Left.es_horizontal());
        assert!(!ShellAnchor::Right.es_horizontal());
    }

    #[test]
    fn franja_del_shell_por_borde() {
        // Salida 1920×1080, grosor 40.
        assert_eq!(shell_strip(ShellAnchor::Top, 1920, 1080, 40), (0, 0, 1920, 40));
        assert_eq!(
            shell_strip(ShellAnchor::Bottom, 1920, 1080, 40),
            (0, 1040, 1920, 40)
        );
        assert_eq!(shell_strip(ShellAnchor::Left, 1920, 1080, 40), (0, 0, 40, 1080));
        assert_eq!(
            shell_strip(ShellAnchor::Right, 1920, 1080, 40),
            (1880, 0, 40, 1080)
        );
    }

    #[test]
    fn insets_reservan_la_zona_del_borde_correcto() {
        // (top, bottom, left, right) — sólo el borde anclado lleva el grosor.
        assert_eq!(shell_insets(ShellAnchor::Top, 40), (40, 0, 0, 0));
        assert_eq!(shell_insets(ShellAnchor::Bottom, 40), (0, 40, 0, 0));
        assert_eq!(shell_insets(ShellAnchor::Left, 48), (0, 0, 48, 0));
        assert_eq!(shell_insets(ShellAnchor::Right, 48), (0, 0, 0, 48));
    }

    #[test]
    fn autohide_bottom_revela_en_el_borde_y_oculta_al_salir() {
        let (ow, oh, t, b) = (800, 600, 40, SHELL_REVEAL_BAND);
        // Oculto: sólo tocar la banda del borde inferior revela.
        assert!(!autohide_next_hidden(ShellAnchor::Bottom, ow, oh, t, 400, 599, true, b));
        assert!(autohide_next_hidden(ShellAnchor::Bottom, ow, oh, t, 400, 300, true, b));
        // Visible: se mantiene sobre la franja (y∈[560,600)), se oculta al salir.
        assert!(!autohide_next_hidden(ShellAnchor::Bottom, ow, oh, t, 400, 570, false, b));
        assert!(autohide_next_hidden(ShellAnchor::Bottom, ow, oh, t, 400, 500, false, b));
    }

    #[test]
    fn autohide_top_usa_el_borde_superior() {
        let (ow, oh, t, b) = (800, 600, 30, SHELL_REVEAL_BAND);
        assert!(!autohide_next_hidden(ShellAnchor::Top, ow, oh, t, 400, 1, true, b));
        assert!(autohide_next_hidden(ShellAnchor::Top, ow, oh, t, 400, 200, true, b));
        assert!(!autohide_next_hidden(ShellAnchor::Top, ow, oh, t, 400, 10, false, b));
        assert!(autohide_next_hidden(ShellAnchor::Top, ow, oh, t, 400, 100, false, b));
    }

    #[test]
    fn banda_de_revelado_pegada_al_borde() {
        // Bottom: 3px abajo, a todo el ancho.
        assert_eq!(shell_reveal_band(ShellAnchor::Bottom, 800, 600, 40, 3), (0, 597, 800, 3));
        // Right: 3px a la derecha, a todo el alto.
        assert_eq!(shell_reveal_band(ShellAnchor::Right, 800, 600, 40, 3), (797, 0, 3, 600));
    }
}
