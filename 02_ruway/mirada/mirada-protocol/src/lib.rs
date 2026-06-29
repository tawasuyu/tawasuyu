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
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use serde::{Deserialize, Serialize};

pub use mirada_layout::geometry::Rect;
pub use mirada_layout::workspace::WindowId;
pub use mirada_layout::{LayoutMode, LayoutParams};
use mirada_layout::Workspace;

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
    /// `true` si está **dormida** tras una capa de zoom (árbol fractal): el
    /// Cuerpo, además de ocultarla (`visible: false`), le suspende los frame
    /// callbacks para que el cliente quede inerte en vez de seguir pintando a
    /// ciegas. El `rect` es su hogar del nivel superior, al que vuelve al salir
    /// del zoom — no se redimensiona mientras duerme.
    pub suspended: bool,
    /// Divisor de frames: el Cuerpo le envía 1 de cada N `wl_surface.frame`
    /// callbacks, espaciando su ritmo de pintado. `1` = pleno ritmo (lo normal);
    /// `>1` = throttle de fondo (ver [`Config::background_frame_divisor`]). A
    /// diferencia de `suspended` (corte total para una ventana oculta), aquí la
    /// ventana **sigue visible y pintando**, sólo que más lento.
    pub frame_divisor: u32,
}

/// Lo que el host le pasa a un **plugin de layout** WASM: la lista ordenada de
/// ventanas teseladas (las que el `Desktop` dejó `visible && !floating &&
/// !fullscreen && !suspended`) y el área útil de la salida en píxeles. El plugin
/// devuelve un `Vec<(WindowId, Rect)>` que el host vuelca sobre los `rect` de
/// esas ventanas — sin tocar foco, visibilidad ni las flotantes/fullscreen.
///
/// Vive aquí, en el vocabulario común, para que host y guest lo compartan sin
/// que ninguno dependa del crate del otro.
///
/// No deriva `Eq` porque `LayoutParams::master_ratio` es `f32`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileInput {
    /// Ventanas teseladas, en el orden en que el `Desktop` las dispondría.
    pub ids: Vec<WindowId>,
    /// Área útil donde repartirlas (ya descontadas las franjas reservadas).
    pub work: Rect,
    /// Los parámetros de teselado vigentes del `Desktop` para esta salida
    /// (modo, fracción maestra, nº de maestras, gap). El plugin los honra para
    /// que los atajos del usuario (crecer maestra, etc.) sigan teniendo efecto.
    pub params: LayoutParams,
}

/// Parámetros de decoración de ventana que el Cerebro fija en el Cuerpo.
/// Hoy cubre el marco (grosor + colores). Los colores son RGBA en
/// `0..=255` — enteros para conservar `Eq` en [`BrainCommand`] y por ser
/// más naturales de escribir en la config que floats en `0..1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decorations {
    /// Grosor del marco en píxeles; `0` = ventanas sin marco.
    pub border_width: i32,
    /// Color RGBA del marco de la ventana enfocada.
    pub border_focus: [u8; 4],
    /// Color RGBA del marco de las ventanas sin foco.
    pub border_normal: [u8; 4],
    /// Alto de la barra de título en píxeles; `0` = sin barra de título (sólo
    /// se muestra el título de la ventana enfocada superpuesto, como antes).
    /// La franja se reserva arriba de cada ventana (no-shell): la superficie
    /// del cliente se achica y la barra se pinta encima.
    pub titlebar_height: i32,
    /// Pintar la barra de título con un **degradé** vertical (claro arriba →
    /// color base abajo) en vez de un color plano. `false` = barra sólida.
    #[serde(default)]
    pub titlebar_gradient: bool,
    /// Barra de título **sólo en las ventanas flotantes** (z-order). `true` =
    /// las teseladas no llevan barra (estilo tiling: el cuerpo entero es
    /// contenido), las flotantes sí (para agarrarlas/cerrarlas); `false` =
    /// todas las SSD llevan barra por igual (comportamiento histórico). No
    /// afecta a shell/fullscreen/greeter/CSD, que nunca llevan barra del
    /// servidor.
    #[serde(default)]
    pub titlebar_floating_only: bool,
    /// Marco con **bevel 3D** estilo Motif/CDE: en vez de un color plano, los
    /// lados superior e izquierdo se aclaran (luz) y los inferior y derecho se
    /// oscurecen (sombra), dando un relieve *levantado*. `false` (default) =
    /// marco plano de un solo color (comportamiento histórico). Pensado para
    /// looks retro con marcos gruesos: con `border_width` chico el efecto es
    /// sutil; gana con grosores de 4 px en adelante.
    #[serde(default)]
    pub border_bevel: bool,
    /// Color RGBA de la **barra de título** con foco, **desacoplado del marco**.
    /// `None` (default) = la barra hereda el color del marco (`border_focus`),
    /// el acoplamiento histórico. `Some` permite el clásico marco gris + barra
    /// de color, o barras claras (mac/Breeze), que de otro modo es imposible.
    #[serde(default)]
    pub titlebar_focus: Option<[u8; 4]>,
    /// Color RGBA de la barra de título **sin** foco. `None` = `border_normal`.
    #[serde(default)]
    pub titlebar_normal: Option<[u8; 4]>,
    /// Color RGBA del **texto y los botones** de la barra de título. `None`
    /// (default) = el claro histórico. Las barras claras (mac, Breeze) lo fijan
    /// oscuro para que el título y los íconos se lean.
    #[serde(default)]
    pub titlebar_text: Option<[u8; 4]>,
    /// Radio de las **esquinas redondeadas** de ventana en px; `0` (default) =
    /// rectas. Viaja en las decoraciones (no en la config sólo-Embedded) para
    /// que el redondeo también ande con el Cerebro enlazado. Caro: el render lo
    /// enmascara por ventana/frame, por eso default 0 (opt-in por vista).
    #[serde(default)]
    pub corner_radius: u8,
}

impl Default for Decorations {
    /// Los valores históricos del Cuerpo: marco de 2 px, azul al foco,
    /// gris discreto sin él, barra de título de 24 px.
    fn default() -> Self {
        Self {
            border_width: 2,
            border_focus: [92, 143, 235, 255],
            border_normal: [56, 56, 69, 255],
            titlebar_height: 24,
            titlebar_gradient: false,
            titlebar_floating_only: false,
            border_bevel: false,
            titlebar_focus: None,
            titlebar_normal: None,
            titlebar_text: None,
            corner_radius: 0,
        }
    }
}

/// Permisos de capacidad por ejecutable que el Cerebro fija en el Cuerpo.
///
/// El Cuerpo es **quien otorga el protocolo Wayland**: una capacidad sensible
/// (espiar el portapapeles vía `zwlr_data_control`, o inyectar pulsaciones
/// sintéticas vía `zwp_virtual_keyboard`) no se concede por una tabla que el
/// cliente pueda eludir, sino **no anunciando el global** a los clientes no
/// autorizados (frontera física, igual que el bitfield `Permisos` del kernel
/// wawa).
///
/// La identidad del cliente es su **ejecutable real**, resuelto por el Cuerpo
/// vía `SO_PEERCRED → /proc/<pid>/exe` — verdad del kernel, no falsificable; el
/// `app_id` es aserción del cliente y los clientes de `data_control` ni siquiera
/// tienen superficie/`app_id`. La postura es **permitir por defecto**: sólo se
/// deniega a los ejecutables que casen (por subcadena, sin distinguir
/// mayúsculas) con alguna entrada de la denylist.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permisos {
    /// Ejecutables a los que se les niega `zwlr_data_control` (el snoop del
    /// portapapeles). Casa por subcadena del path del ejecutable, sin distinguir
    /// mayúsculas. Vacía = nadie denegado (todos pueden bindear el global).
    #[serde(default)]
    pub clipboard_denylist: Vec<String>,
    /// Ejecutables a los que se les niega `zwp_virtual_keyboard` (la inyección
    /// de pulsaciones sintéticas — keylogger a la inversa). Casa por subcadena
    /// del path del ejecutable, sin distinguir mayúsculas. Vacía = nadie
    /// denegado (todos pueden bindear el global).
    #[serde(default)]
    pub virtual_input_denylist: Vec<String>,
    /// Ejecutables a los que se les niega `ext_foreign_toplevel_list` (el censo
    /// de ventanas: título + `app_id` de todo lo abierto — con qué banco operás,
    /// qué documento editás). Casa por subcadena del path del ejecutable, sin
    /// distinguir mayúsculas. Vacía = nadie denegado (todos pueden bindear el
    /// global).
    #[serde(default)]
    pub window_list_denylist: Vec<String>,
    /// Ejecutables a los que se les niega `zwlr_screencopy` (leer los píxeles
    /// de la pantalla — la capacidad más sensible de todas). Casa por
    /// subcadena del path del ejecutable, sin distinguir mayúsculas. Vacía =
    /// nadie denegado (todos pueden bindear el global).
    #[serde(default)]
    pub screencopy_denylist: Vec<String>,
    /// Ejecutables a los que se les niega `zwp_linux_dmabuf` (importar búferes
    /// de GPU compartidos: el cliente pinta directo en memoria de video y se la
    /// pasa al compositor sin copia). Negarlo no rompe la app —cae al camino
    /// `wl_shm` por software—, sólo le quita el atajo zero-copy. Casa por
    /// subcadena del path del ejecutable, sin distinguir mayúsculas. Vacía =
    /// nadie denegado (todos pueden bindear el global).
    #[serde(default)]
    pub dmabuf_denylist: Vec<String>,
}

impl Permisos {
    /// `true` si el ejecutable `exe` puede bindear `zwlr_data_control` (leer el
    /// portapapeles). Deniega sólo si `exe` contiene —sin distinguir
    /// mayúsculas— alguna entrada de la denylist. Denylist vacía ⇒ siempre
    /// permitido.
    pub fn clipboard_permitido(&self, exe: &str) -> bool {
        permitido(&self.clipboard_denylist, exe)
    }

    /// `true` si el ejecutable `exe` puede bindear `zwp_virtual_keyboard`
    /// (inyectar pulsaciones). Misma semántica de denylist por subcadena que
    /// [`Permisos::clipboard_permitido`]. Denylist vacía ⇒ siempre permitido.
    pub fn virtual_input_permitido(&self, exe: &str) -> bool {
        permitido(&self.virtual_input_denylist, exe)
    }

    /// `true` si el ejecutable `exe` puede bindear `ext_foreign_toplevel_list`
    /// (enumerar las ventanas abiertas). Misma semántica de denylist por
    /// subcadena que [`Permisos::clipboard_permitido`]. Denylist vacía ⇒
    /// siempre permitido.
    pub fn window_list_permitido(&self, exe: &str) -> bool {
        permitido(&self.window_list_denylist, exe)
    }

    /// `true` si el ejecutable `exe` puede bindear `zwlr_screencopy` (capturar
    /// los píxeles de la pantalla). Misma semántica de denylist por subcadena
    /// que [`Permisos::clipboard_permitido`]. Denylist vacía ⇒ siempre
    /// permitido.
    pub fn screencopy_permitido(&self, exe: &str) -> bool {
        permitido(&self.screencopy_denylist, exe)
    }

    /// `true` si el ejecutable `exe` puede bindear `zwp_linux_dmabuf` (importar
    /// búferes de GPU zero-copy). Misma semántica de denylist por subcadena que
    /// [`Permisos::clipboard_permitido`]. Denylist vacía ⇒ siempre permitido.
    pub fn dmabuf_permitido(&self, exe: &str) -> bool {
        permitido(&self.dmabuf_denylist, exe)
    }
}

/// Resuelve una denylist por subcadena, sin distinguir mayúsculas: permite
/// salvo que `exe` contenga alguna entrada. Lista vacía ⇒ siempre permitido.
fn permitido(denylist: &[String], exe: &str) -> bool {
    let exe = exe.to_lowercase();
    !denylist.iter().any(|d| exe.contains(&d.to_lowercase()))
}

/// Los efectos visuales que el Cerebro fija por ventana ([`BrainCommand::SetEffects`]).
///
/// Declarativo y extensible: un efecto nuevo (esquinas redondeadas, blur…) se
/// agrega como un campo aquí, sin tocar [`BrainCommand`]. Todos los campos son
/// `Eq`-safe (sin `f32`): la opacidad va en `u8`, como los colores de
/// [`Decorations`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowEffects {
    /// Opacidad de composición: `0` = transparente, `255` = opaca.
    pub opacity: u8,
    /// Pintar una sombra difusa detrás de la ventana.
    pub shadow: bool,
}

impl Default for WindowEffects {
    /// Sin efectos: opaca y sin sombra.
    fn default() -> Self {
        Self { opacity: 255, shadow: false }
    }
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
    /// Fija los parámetros de decoración de las ventanas (marco, …). El
    /// Cerebro lo envía al arrancar y tras recargar la config.
    SetDecorations(Decorations),
    /// Fija los permisos de capacidad por ejecutable (qué se le concede a
    /// quién): el snoop de portapapeles (`zwlr_data_control`), la inyección
    /// de teclas (`zwp_virtual_keyboard`), el censo de ventanas
    /// (`ext_foreign_toplevel_list`) y la captura de pantalla
    /// (`zwlr_screencopy`). El Cerebro lo envía al arrancar y tras recargar
    /// la config.
    SetCapabilities(Permisos),
    /// Lanza un programa como proceso hijo del Cuerpo — hereda su
    /// entorno, `WAYLAND_DISPLAY` incluido, así el cliente se conecta
    /// aquí. La cadena se pasa a `sh -c`.
    Spawn(String),
    /// Apaga el Cuerpo y libera el hardware.
    Shutdown,
    /// Bloquea la sesión activa: el Cuerpo compone el shell de credenciales
    /// (greeter en modo lock) encima de todo y le rutea el input hasta el
    /// desbloqueo. No-op si ya hay un shell de credenciales en pantalla.
    Lock,
    /// Cierra la sesión activa (FUS logout): el Cuerpo manda cerrar sus ventanas,
    /// la da de baja del roster de sesiones y pasa el foco a otra hosteada — o
    /// compone el login si no queda ninguna. No-op sin sesión activa.
    Logout,
    /// Estado de escritorios que el Cerebro **enlazado** empuja al Cuerpo para
    /// que su switcher Win+Tab (HUD + transición) funcione en modo DE: el
    /// escritorio activo, las cargas (nº de ventanas por escritorio), la duración
    /// del slide en ms (`0` = salto seco) y el **modo de transición** como slug
    /// (`"direct"`/`"hyprland"`/`"prezi"`/`"cube"` — `WorkspaceSwitchMode::slug`).
    /// Sin el slug, el Cuerpo no podría distinguir Cube/Prezi de Hyprland (sólo
    /// veía `slide_ms`) y esos modos quedaban inalcanzables en modo enlazado. En
    /// modo embebido el Cuerpo ya tiene estos datos y no recibe esto.
    SetWorkspaces { active: u32, loads: Vec<u32>, slide_ms: u32, switch_mode: String },
    /// Fija los **efectos visuales** ([`WindowEffects`]) de ciertas ventanas; las
    /// no listadas conservan los suyos. Es el canal Tier-2 declarativo: efectos
    /// nuevos se agregan como campos de [`WindowEffects`], sin tocar este enum.
    SetEffects(Vec<(WindowId, WindowEffects)>),
}

/// Un hecho del Cuerpo que el Cerebro debe conocer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyEvent {
    /// Apareció un monitor (al arrancar o en caliente).
    OutputAdded { id: OutputId, width: i32, height: i32 },
    /// Desapareció un monitor.
    OutputRemoved { id: OutputId },
    /// Cambió el tamaño físico de un monitor — se redimensionó la ventana
    /// anfitriona o el backend reportó otra resolución. El escritorio que
    /// muestra **no** cambia (a diferencia de quitar y volver a añadir).
    OutputResized { id: OutputId, width: i32, height: i32 },
    /// El marco (`pata`/shell) reservó —o liberó— franjas en los bordes de un
    /// monitor: las **zonas exclusivas** que el teselado debe esquivar, en
    /// píxeles desde cada borde. Cero en los cuatro = nada reservado (el área
    /// útil vuelve a ser el monitor entero). A diferencia de `OutputResized`,
    /// no cambia el tamaño físico: sólo el área teselada dentro de él, así que
    /// soporta barras en cualquier borde (top/bottom/left/right) a la vez.
    OutputReserved {
        id: OutputId,
        top: i32,
        bottom: i32,
        left: i32,
        right: i32,
    },
    /// Un cliente creó una ventana de nivel superior.
    WindowOpened { id: WindowId, app_id: String, title: String },
    /// El **linaje de proceso** de una ventana recién abierta: su PID y la
    /// cadena de PIDs ancestros (del padre inmediato hacia la raíz, acotada). Lo
    /// emite el Cuerpo justo tras [`WindowOpened`](BodyEvent::WindowOpened),
    /// **best-effort**: el Cerebro lo usa para agrupar por *constelación* (linaje
    /// de actividad). Evento aparte —no campo de `WindowOpened`— para no romper la
    /// simulación ni los 20+ sitios que ya construyen ese evento; si el Cuerpo no
    /// puede averiguar el PID (backend anidado, sin credenciales), simplemente no
    /// lo emite.
    WindowLineage { id: WindowId, pid: u32, ancestors: Vec<u32> },
    /// Una ventana se cerró (por el cliente o tras un [`BrainCommand::Close`]).
    WindowClosed { id: WindowId },
    /// Una ventana cambió su título.
    WindowRetitled { id: WindowId, title: String },
    /// El usuario pulsó un atajo registrado con [`BrainCommand::GrabKeys`].
    Keybind(String),
    /// El puntero entró en una ventana — el Cerebro puede enfocar al pasar
    /// (foco-sigue-ratón, si la config lo habilita).
    PointerEntered { id: WindowId },
    /// El usuario hizo click (botón primario) sobre una ventana — el
    /// Cerebro la enfoca, esté donde esté, **sin** depender del
    /// foco-sigue-ratón. Es el camino del foco-al-click.
    Clicked { id: WindowId },
    /// Arrastre interactivo de una ventana **teselada** sobre el punto
    /// `(x, y)` de pantalla: el Cerebro la intercambia con la ventana
    /// teselada que haya ahí (reordena el stack), conservándola teselada.
    /// El arrastre de una flotante usa [`BodyEvent::WindowFloatTo`] en su
    /// lugar — moverla, no intercambiarla.
    WindowDragged { id: WindowId, x: i32, y: i32 },
    /// Un cliente pidió pantalla completa para su ventana (`true`), o la
    /// soltó (`false`) — `xdg_toplevel.set_fullscreen`.
    FullscreenRequest { id: WindowId, fullscreen: bool },
    /// El usuario arrastró una ventana con el ratón a un rectángulo nuevo
    /// (mover o redimensionar interactivos). El Cerebro la hace flotar
    /// ahí; si estaba teselada, deja de estarlo.
    WindowFloatTo { id: WindowId, rect: Rect },
    /// El switcher Win+Tab del Cuerpo confirmó un salto a un escritorio
    /// (modo **enlazado**): el Cerebro externo lo aplica. En modo embebido el
    /// Cuerpo cambia el escritorio él mismo y no emite esto.
    SwitchWorkspace(u32),
    /// El Cuerpo movió el **origen global** de un monitor: `(x, y)` es su
    /// esquina superior-izquierda en el espacio compuesto. Lo emite el backend
    /// cada vez que recalcula la disposición de salidas (arranque, hotplug,
    /// reordenamiento por config), porque el Cuerpo —que conoce nombres,
    /// `order` y dirección— es la **fuente única** de la geometría. Sin esto el
    /// Cerebro la reconstruía por su cuenta (fila por orden de aparición), y al
    /// diferir del backend las ventanas maximizadas/teseladas aterrizaban en el
    /// monitor equivocado o se desbordaban. El alto/ancho siguen llegando por
    /// [`OutputAdded`](BodyEvent::OutputAdded)/[`OutputResized`](BodyEvent::OutputResized);
    /// esto fija sólo la posición.
    OutputMoved { id: OutputId, x: i32, y: i32 },
}

/// Tamaño máximo de un marco, en bytes. Acota el búfer de [`read_frame`]
/// para que un prefijo de longitud corrupto no reserve gigabytes.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Escribe `value` como un marco: prefijo `u32` LE con la longitud + el
/// cuerpo serializado con `postcard`.
#[cfg(feature = "framing")]
pub fn write_frame<W: std::io::Write, T: Serialize>(w: &mut W, value: &T) -> std::io::Result<()> {
    use std::io;
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
#[cfg(feature = "framing")]
pub fn read_frame<R: std::io::Read, T: serde::de::DeserializeOwned>(
    r: &mut R,
) -> std::io::Result<Option<T>> {
    use std::io;
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
///
/// Con zoom activo (árbol fractal) añade, tras las visibles, las ventanas
/// **dormidas** ([`Workspace::dormant`]) marcadas `suspended` — fuera de vista
/// pero listadas explícitamente para que el Cuerpo les corte los frames en vez
/// de limitarse a ocultarlas por omisión.
pub fn placements(ws: &Workspace, screen: Rect) -> Vec<WindowPlacement> {
    let fullscreen = ws.fullscreen();
    let monocle = ws.params().mode == LayoutMode::Monocle;
    let focused = ws.focused();
    let mut out: Vec<WindowPlacement> = ws
        .layout(screen)
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
                suspended: false,
                // Throttle apagado por defecto; la política de fondo (si la hay)
                // la aplica el Cerebro en `relayout`, que conoce el foco global.
                frame_divisor: 1,
            }
        })
        .collect();
    // Capas profundas dormidas: ocultas, sin foco y con los frames suspendidos.
    for (id, rect) in ws.dormant(screen) {
        out.push(WindowPlacement {
            id,
            rect,
            visible: false,
            focused: false,
            floating: false,
            fullscreen: false,
            suspended: true,
            // Dormida: los frames ya están cortados del todo; el divisor da igual.
            frame_divisor: 1,
        });
    }
    out
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
            suspended: false,
            frame_divisor: 1,
        }]);
        let mut buf = Vec::new();
        write_frame(&mut buf, &cmd).unwrap();
        let mut cur = Cursor::new(buf);
        let back: BrainCommand = read_frame(&mut cur).unwrap().unwrap();
        assert_eq!(back, cmd);
    }

    #[test]
    fn frame_round_trips_set_capabilities() {
        let cmd = BrainCommand::SetCapabilities(Permisos {
            clipboard_denylist: vec!["wl-paste".into(), "/usr/bin/sneaky".into()],
            virtual_input_denylist: vec!["wtype".into()],
            window_list_denylist: vec!["lswt".into()],
            screencopy_denylist: vec!["grim".into()],
            dmabuf_denylist: vec!["spyware".into()],
        });
        let mut buf = Vec::new();
        write_frame(&mut buf, &cmd).unwrap();
        let mut cur = Cursor::new(buf);
        let back: BrainCommand = read_frame(&mut cur).unwrap().unwrap();
        assert_eq!(back, cmd);
    }

    #[test]
    fn permisos_vacios_permiten_a_todos() {
        let p = Permisos::default();
        assert!(p.clipboard_permitido("/usr/bin/wl-paste"));
        assert!(p.clipboard_permitido("cualquiera"));
        assert!(p.virtual_input_permitido("/usr/bin/wtype"));
        assert!(p.window_list_permitido("/usr/bin/lswt"));
        assert!(p.screencopy_permitido("/usr/bin/grim"));
        assert!(p.dmabuf_permitido("/usr/bin/firefox"));
    }

    #[test]
    fn denylist_niega_por_subcadena_sin_distinguir_mayusculas() {
        let p = Permisos {
            clipboard_denylist: vec!["wl-paste".into()],
            virtual_input_denylist: vec!["wtype".into()],
            window_list_denylist: vec!["lswt".into()],
            screencopy_denylist: vec!["grim".into()],
            dmabuf_denylist: vec!["spyware".into()],
        };
        // Casa por subcadena: el path completo contiene el binario denegado.
        assert!(!p.clipboard_permitido("/usr/bin/wl-paste"));
        // Sin distinguir mayúsculas.
        assert!(!p.clipboard_permitido("/opt/WL-Paste"));
        // No casa lo no listado.
        assert!(p.clipboard_permitido("/usr/bin/wl-copy"));
        // Las denylists son independientes: wtype inyecta pero puede leer
        // el portapapeles; wl-paste lee pero puede inyectar; lswt sólo
        // pierde el censo de ventanas.
        assert!(!p.virtual_input_permitido("/usr/bin/wtype"));
        assert!(p.virtual_input_permitido("/usr/bin/wl-paste"));
        assert!(p.clipboard_permitido("/usr/bin/wtype"));
        assert!(!p.window_list_permitido("/usr/bin/lswt"));
        assert!(p.window_list_permitido("/usr/bin/wtype"));
        assert!(p.virtual_input_permitido("/usr/bin/lswt"));
        assert!(!p.screencopy_permitido("/usr/bin/grim"));
        assert!(p.screencopy_permitido("/usr/bin/lswt"));
        assert!(p.window_list_permitido("/usr/bin/grim"));
        // dmabuf es otra denylist independiente: niega «spyware», permite el resto.
        assert!(!p.dmabuf_permitido("/opt/spyware/bin/leak"));
        assert!(p.dmabuf_permitido("/usr/bin/firefox"));
        assert!(p.screencopy_permitido("/opt/spyware/bin/leak")); // no toca screencopy
    }

    #[test]
    fn frame_round_trips_set_effects() {
        let cmd = BrainCommand::SetEffects(vec![
            (7, WindowEffects { opacity: 180, shadow: false }),
            (9, WindowEffects { opacity: 255, shadow: true }),
        ]);
        let mut buf = Vec::new();
        write_frame(&mut buf, &cmd).unwrap();
        let back: BrainCommand = read_frame(&mut Cursor::new(buf)).unwrap().unwrap();
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
    fn frame_round_trips_a_window_lineage_event() {
        let ev = BodyEvent::WindowLineage { id: 9, pid: 4242, ancestors: vec![4200, 1, 0] };
        let mut buf = Vec::new();
        write_frame(&mut buf, &ev).unwrap();
        let back: BodyEvent = read_frame(&mut Cursor::new(buf)).unwrap().unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn frame_round_trips_the_new_body_events() {
        for ev in [
            BodyEvent::Clicked { id: 7 },
            BodyEvent::WindowDragged { id: 3, x: 640, y: -12 },
        ] {
            let mut buf = Vec::new();
            write_frame(&mut buf, &ev).unwrap();
            let back: BodyEvent = read_frame(&mut Cursor::new(buf)).unwrap().unwrap();
            assert_eq!(back, ev);
        }
    }

    #[test]
    fn frame_round_trips_a_set_decorations_command() {
        let cmd = BrainCommand::SetDecorations(Decorations {
            border_width: 3,
            border_focus: [10, 20, 30, 255],
            border_normal: [1, 2, 3, 4],
            titlebar_height: 24,
            titlebar_gradient: true,
            titlebar_floating_only: false,
            border_bevel: true,
            titlebar_focus: Some([0, 0, 128, 255]),
            titlebar_normal: None,
            titlebar_text: Some([20, 20, 20, 255]),
            corner_radius: 12,
        });
        let mut buf = Vec::new();
        write_frame(&mut buf, &cmd).unwrap();
        let back: BrainCommand = read_frame(&mut Cursor::new(buf)).unwrap().unwrap();
        assert_eq!(back, cmd);
    }

    #[test]
    fn default_decorations_are_the_historic_values() {
        let d = Decorations::default();
        assert_eq!(d.border_width, 2);
        assert_eq!(d.border_focus, [92, 143, 235, 255]);
        assert_eq!(d.border_normal, [56, 56, 69, 255]);
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
    fn zoomed_layers_are_listed_dormant_not_omitted() {
        // Tres en columnas; agrupo {20,30} y entro en el grupo.
        let mut w = Workspace::new(LayoutParams {
            mode: LayoutMode::Columns,
            gap: 0,
            ..LayoutParams::default()
        });
        for id in [10, 20, 30] {
            w.add(id);
        }
        w.group(&[20, 30]);
        w.focus_window(20);
        w.zoom_in();
        let p = placements(&w, SCREEN);
        // Las tres siguen en la lista (la 10 no se omite: se marca dormida).
        assert_eq!(p.len(), 3);
        let ten = p.iter().find(|x| x.id == 10).unwrap();
        assert!(ten.suspended, "la capa profunda fuera de vista duerme");
        assert!(!ten.visible);
        assert!(!ten.focused);
        // Las en vista no están suspendidas y sí visibles.
        for id in [20, 30] {
            let v = p.iter().find(|x| x.id == id).unwrap();
            assert!(!v.suspended && v.visible);
        }
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
