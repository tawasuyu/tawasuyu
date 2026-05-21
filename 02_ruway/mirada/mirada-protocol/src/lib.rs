//! `mirada-protocol` — el contrato Cerebro↔Cuerpo del compositor.
//!
//! mirada se parte en dos procesos:
//!
//! - **El Cuerpo** (`mirada-compositor`, sobre `smithay`): habla Wayland
//!   con los clientes, posee el hardware (DRM/GPU/libinput) y compone las
//!   superficies reales. Los píxeles nunca salen de él.
//! - **El Cerebro** (una app GPUI sobre [`mirada-layout`]): decide *dónde*
//!   va cada ventana — pura aritmética de rectángulos— y orquesta el
//!   escritorio (layouts, atajos, focos).
//!
//! Este crate es el único lenguaje que comparten: un par de enums y un
//! marco de cable. No depende de Wayland, ni de `smithay`, ni de GPUI —
//! sólo de [`mirada-layout`] para reusar [`Rect`] y [`WindowId`].
//!
//! - El Cerebro emite [`BrainCommand`]; el Cuerpo los aplica.
//! - El Cuerpo emite [`BodyEvent`]; el Cerebro reacciona y recalcula.
//!
//! El cable es [`postcard`] con prefijo de longitud `u32` little-endian
//! (ver [`write_frame`] / [`read_frame`]).

#![forbid(unsafe_code)]

use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub use mirada_layout::geometry::Rect;
pub use mirada_layout::workspace::WindowId;
use mirada_layout::{LayoutMode, Workspace};

/// Identificador de una salida física (un monitor).
pub type OutputId = u32;

/// Dónde y cómo debe colocarse una ventana en pantalla.
///
/// Es la unidad de geometría que el Cerebro calcula y el Cuerpo aplica a
/// la superficie Wayland correspondiente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub id: WindowId,
    /// Rectángulo en píxeles de pantalla.
    pub rect: Rect,
    /// `false` la oculta sin destruirla (p. ej. en modo `Monocle`).
    pub visible: bool,
    /// `true` si esta ventana tiene el foco del teclado.
    pub focused: bool,
    /// `true` si flota (fuera del teselado): el Cuerpo la pinta encima.
    pub floating: bool,
    /// `true` si está en pantalla completa: cubre toda la salida.
    pub fullscreen: bool,
}

/// Una orden del Cerebro al Cuerpo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrainCommand {
    /// Geometría completa del escritorio: el Cuerpo mueve/redimensiona
    /// cada superficie y oculta las que falten en la lista.
    Place(Vec<WindowPlacement>),
    /// Pide el cierre ordenado de una ventana (`xdg_toplevel.close`).
    Close(WindowId),
    /// Mata al cliente de una ventana que no responde.
    Kill(WindowId),
    /// Registra los atajos globales que el Cuerpo debe interceptar y
    /// devolver como [`BodyEvent::Keybind`] en vez de pasarlos al cliente.
    GrabKeys(Vec<String>),
    /// Cambia el cursor del puntero al nombre dado (tema XCursor).
    SetCursor(String),
    /// Apaga el Cuerpo y libera el hardware.
    Shutdown,
}

/// Un hecho del Cuerpo que el Cerebro debe conocer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyEvent {
    /// Apareció un monitor (al arrancar o en caliente).
    OutputAdded { id: OutputId, width: i32, height: i32 },
    /// Desapareció un monitor.
    OutputRemoved { id: OutputId },
    /// Un cliente creó una ventana de nivel superior.
    WindowOpened { id: WindowId, app_id: String, title: String },
    /// Una ventana se cerró (por el cliente o tras un [`BrainCommand::Close`]).
    WindowClosed { id: WindowId },
    /// Una ventana cambió su título.
    WindowRetitled { id: WindowId, title: String },
    /// El usuario pulsó un atajo registrado con [`BrainCommand::GrabKeys`].
    Keybind(String),
    /// El puntero entró en una ventana — el Cerebro puede enfocar al pasar.
    PointerEntered { id: WindowId },
    /// Un cliente pidió pantalla completa para su ventana (`true`), o la
    /// soltó (`false`) — `xdg_toplevel.set_fullscreen`.
    FullscreenRequest { id: WindowId, fullscreen: bool },
}

/// Tamaño máximo de un marco, en bytes. Acota el búfer de [`read_frame`]
/// para que un prefijo de longitud corrupto no reserve gigabytes.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Escribe `value` como un marco: prefijo `u32` LE con la longitud + el
/// cuerpo serializado con `postcard`.
pub fn write_frame<W: Write, T: Serialize>(w: &mut W, value: &T) -> io::Result<()> {
    let body = postcard::to_stdvec(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "marco mayor que MAX_FRAME",
        ));
    }
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

/// Lee un marco escrito por [`write_frame`]. Devuelve `Ok(None)` en un
/// EOF limpio (el otro extremo cerró sin datos a medias).
pub fn read_frame<R: Read, T: DeserializeOwned>(r: &mut R) -> io::Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prefijo de longitud mayor que MAX_FRAME",
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let value = postcard::from_bytes(&body)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(value))
}

/// Traduce un [`Workspace`] de mirada-layout a la geometría de cable.
///
/// Es el puente del Cerebro: toma el estado abstracto (ventanas, foco,
/// modo) y la pantalla física, y produce el [`Vec<WindowPlacement>`] que
/// va dentro de un [`BrainCommand::Place`].
///
/// En modo [`LayoutMode::Monocle`] sólo la ventana enfocada queda
/// `visible`; en el resto de modos todas lo están.
pub fn placements(ws: &Workspace, screen: Rect) -> Vec<WindowPlacement> {
    let fullscreen = ws.fullscreen();
    let monocle = ws.params().mode == LayoutMode::Monocle;
    let focused = ws.focused();
    ws.layout(screen)
        .into_iter()
        .map(|(id, rect)| {
            let floating = ws.is_floating(id);
            let is_fs = fullscreen == Some(id);
            // Con una ventana en pantalla completa manda ella: ocupa toda
            // la salida, es la única visible y se lleva el foco.
            let (rect, visible, is_focused) = match fullscreen {
                Some(_) => (if is_fs { screen } else { rect }, is_fs, is_fs),
                None => {
                    let f = focused == Some(id);
                    // Una flotante siempre se ve; en `Monocle`, sólo la enfocada.
                    (rect, floating || !monocle || f, f)
                }
            };
            WindowPlacement {
                id,
                rect,
                visible,
                focused: is_focused,
                floating,
                fullscreen: is_fs,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirada_layout::LayoutParams;
    use std::io::Cursor;

    fn ws(mode: LayoutMode) -> Workspace {
        let mut w = Workspace::new(LayoutParams { mode, ..LayoutParams::default() });
        for id in [10, 20, 30] {
            w.add(id);
        }
        w
    }

    const SCREEN: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };

    #[test]
    fn frame_round_trips_a_brain_command() {
        let cmd = BrainCommand::Place(vec![WindowPlacement {
            id: 7,
            rect: Rect::new(0, 0, 800, 600),
            visible: true,
            focused: true,
            floating: false,
            fullscreen: false,
        }]);
        let mut buf = Vec::new();
        write_frame(&mut buf, &cmd).unwrap();
        let mut cur = Cursor::new(buf);
        let back: BrainCommand = read_frame(&mut cur).unwrap().unwrap();
        assert_eq!(back, cmd);
    }

    #[test]
    fn frame_round_trips_a_body_event() {
        let ev = BodyEvent::WindowOpened {
            id: 42,
            app_id: "org.brahman.shuma".into(),
            title: "shell".into(),
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &ev).unwrap();
        let mut cur = Cursor::new(buf);
        let back: BodyEvent = read_frame(&mut cur).unwrap().unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn several_frames_stream_in_order() {
        let evs = vec![
            BodyEvent::OutputAdded { id: 0, width: 2560, height: 1440 },
            BodyEvent::WindowOpened { id: 1, app_id: "a".into(), title: "t".into() },
            BodyEvent::Keybind("Super+Return".into()),
        ];
        let mut buf = Vec::new();
        for ev in &evs {
            write_frame(&mut buf, ev).unwrap();
        }
        let mut cur = Cursor::new(buf);
        for ev in &evs {
            let back: BodyEvent = read_frame(&mut cur).unwrap().unwrap();
            assert_eq!(&back, ev);
        }
        // Agotado el stream, un EOF limpio.
        assert!(read_frame::<_, BodyEvent>(&mut cur).unwrap().is_none());
    }

    #[test]
    fn empty_reader_is_a_clean_eof() {
        let mut cur = Cursor::new(Vec::new());
        assert!(read_frame::<_, BrainCommand>(&mut cur).unwrap().is_none());
    }

    #[test]
    fn an_oversized_length_prefix_is_rejected() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(u32::MAX).to_le_bytes());
        let mut cur = Cursor::new(buf);
        assert!(read_frame::<_, BrainCommand>(&mut cur).is_err());
    }

    #[test]
    fn placements_cover_every_window() {
        let p = placements(&ws(LayoutMode::Columns), SCREEN);
        assert_eq!(p.len(), 3);
        assert!(p.iter().all(|w| w.visible));
        // Sólo una enfocada — la última añadida.
        assert_eq!(p.iter().filter(|w| w.focused).count(), 1);
        assert!(p.iter().find(|w| w.id == 30).unwrap().focused);
    }

    #[test]
    fn monocle_keeps_only_the_focused_window_visible() {
        let p = placements(&ws(LayoutMode::Monocle), SCREEN);
        assert_eq!(p.len(), 3);
        assert_eq!(p.iter().filter(|w| w.visible).count(), 1);
        let shown = p.iter().find(|w| w.visible).unwrap();
        assert!(shown.focused);
        assert_eq!(shown.id, 30);
    }

    #[test]
    fn an_empty_workspace_places_nothing() {
        let empty = Workspace::new(LayoutParams::default());
        assert!(placements(&empty, SCREEN).is_empty());
    }

    #[test]
    fn a_floating_window_is_marked_and_stays_visible_in_monocle() {
        let mut w = ws(LayoutMode::Monocle); // Monocle oculta las no enfocadas
        w.set_floating(10, Some(Rect::new(0, 0, 200, 200)));
        let p = placements(&w, SCREEN);
        let f = p.iter().find(|x| x.id == 10).unwrap();
        assert!(f.floating);
        assert!(f.visible, "una flotante se ve aunque el modo sea Monocle");
        // Y conserva su rectángulo flotante.
        assert_eq!(f.rect, Rect::new(0, 0, 200, 200));
    }

    #[test]
    fn a_fullscreen_window_covers_the_screen_and_hides_the_rest() {
        let mut w = ws(LayoutMode::Columns);
        w.set_fullscreen(Some(20));
        let p = placements(&w, SCREEN);
        let fs = p.iter().find(|x| x.id == 20).unwrap();
        assert!(fs.fullscreen);
        assert!(fs.focused, "la ventana en pantalla completa se lleva el foco");
        assert_eq!(fs.rect, SCREEN);
        // El resto queda oculto.
        assert!(p.iter().filter(|x| x.id != 20).all(|x| !x.visible));
    }

    #[test]
    fn placements_fill_a_place_command_round_trip() {
        let cmd = BrainCommand::Place(placements(&ws(LayoutMode::Grid), SCREEN));
        let mut buf = Vec::new();
        write_frame(&mut buf, &cmd).unwrap();
        let mut cur = Cursor::new(buf);
        let back: BrainCommand = read_frame(&mut cur).unwrap().unwrap();
        assert_eq!(back, cmd);
    }
}
