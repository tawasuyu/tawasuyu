//! Acciones de escritorio y su mapa de teclas por defecto.
//!
//! Una [`DesktopAction`] es una orden de alto nivel del usuario, ya
//! desligada de la tecla concreta: el [`Desktop`](crate::Desktop) las
//! aplica sin saber qué combinación las disparó.
//!
//! Cada acción tiene una **forma textual** estable ([`Display`] /
//! [`FromStr`]) — `"focus-next"`, `"layout:grid"`, `"workspace:3"` — que
//! es el vocabulario del keymap configurable en RON (ver [`crate::keymap`]).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use mirada_layout::{LayoutMode, WindowId};

/// Número de escritorios virtuales que mantiene el `Desktop`.
pub const WORKSPACE_COUNT: usize = 9;

/// **Lupa** (zoom de pantalla completa) — porcentaje mínimo: `100` = 1.0×, sin
/// zoom. Es el suelo de [`DesktopAction::MagnifyOut`].
pub const MAGNIFY_MIN_PCT: u16 = 100;
/// **Lupa** — porcentaje máximo: `800` = 8.0×. Techo de [`DesktopAction::MagnifyIn`].
pub const MAGNIFY_MAX_PCT: u16 = 800;
/// **Lupa** — paso de cada [`MagnifyIn`](DesktopAction::MagnifyIn) /
/// [`MagnifyOut`](DesktopAction::MagnifyOut): `50` % (1.5×, 2.0×, 2.5×…).
pub const MAGNIFY_STEP_PCT: u16 = 50;

/// Una dirección cardinal en pantalla — para el foco (y, a futuro, el
/// movimiento) espacial entre ventanas teseladas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    /// El sufijo textual de la dirección — `"left"`, `"right"`, …
    fn slug(self) -> &'static str {
        match self {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }

    /// La dirección desde su sufijo, o `None` si no calza.
    fn from_slug(s: &str) -> Option<Direction> {
        Some(match s {
            "left" => Direction::Left,
            "right" => Direction::Right,
            "up" => Direction::Up,
            "down" => Direction::Down,
            _ => return None,
        })
    }
}

/// Una orden de escritorio de alto nivel.
///
/// Es serializable (`postcard`) para viajar por el API de control
/// ([`crate::ctl`]) y tiene una forma textual estable ([`Display`] /
/// [`FromStr`]) para el keymap y `mirada-ctl`.
///
/// No es `Copy`: [`Spawn`](DesktopAction::Spawn) lleva su comando.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesktopAction {
    /// Mueve el foco a la ventana siguiente del escritorio activo.
    FocusNext,
    /// Mueve el foco a la ventana anterior.
    FocusPrev,
    /// Mueve el foco a la ventana teselada más cercana en una dirección
    /// cardinal (espacial, no cíclico) — el `Super+flechas` clásico.
    FocusDir(Direction),
    /// Enfoca una ventana concreta por su id; si está en otro escritorio,
    /// salta a él. Para clics de taskbar o `mirada-ctl focus-window`.
    FocusWindow(WindowId),
    /// Intercambia la ventana enfocada con su vecina teselada en una
    /// dirección cardinal (mueve la ventana por geometría) — Super+Shift+flechas.
    MoveDir(Direction),
    /// Adelanta la ventana enfocada en el orden de teselado.
    MoveForward,
    /// Atrasa la ventana enfocada en el orden de teselado.
    MoveBackward,
    /// Cierra la ventana enfocada (cierre ordenado).
    CloseFocused,
    /// Cierra una ventana concreta por su id (cierre ordenado), sin tener que
    /// enfocarla antes. Para el clic derecho de un taskbar o
    /// `mirada-ctl close-window`.
    CloseWindow(WindowId),
    /// Alterna entre flotante y teselada la ventana enfocada.
    ToggleFloat,
    /// Alterna el escritorio entero entre teselado y flotante: si queda
    /// alguna teselada, las hace flotar todas (en cascada); si ya están
    /// todas flotando, las devuelve al teselado.
    ToggleTiling,
    /// Alterna pantalla completa en la ventana enfocada.
    ToggleFullscreen,
    /// Alterna **maximizar** la ventana enfocada: la hace flotar ocupando toda
    /// el área de trabajo (conservando la barra de título, así se puede
    /// restaurar) o, si ya está así, la devuelve al teselado. A diferencia de
    /// [`ToggleFullscreen`], no oculta el chrome ni "se apropia" del escritorio.
    ToggleMaximize,
    /// Guarda la ventana enfocada en el scratchpad (la oculta). Es el
    /// **escritorio especial por defecto** (sin nombre) — equivale a
    /// `MoveToSpecialWorkspace("")`.
    SendToScratchpad,
    /// Invoca u oculta la ventana del scratchpad — aparece flotando. Equivale a
    /// `ToggleSpecialWorkspace("")`.
    ToggleScratchpad,
    /// Manda la ventana enfocada a un **escritorio especial con nombre**
    /// (estilo Hyprland `movetoworkspace special:nombre`): se oculta del
    /// escritorio normal y queda apartada en ese especial.
    MoveToSpecialWorkspace(String),
    /// Muestra/oculta un escritorio especial con nombre como overlay flotante
    /// sobre el activo (Hyprland `togglespecialworkspace nombre`): si alguna de
    /// sus ventanas está a la vista, las guarda todas; si no, las trae todas.
    ToggleSpecialWorkspace(String),
    /// **Membresía persistente**: declara que las ventanas de `app_id` pertenecen
    /// al especial `special`. Distinto de `MoveToSpecialWorkspace` (que aparta la
    /// enfocada): acá la ventana **nace visible** pero queda etiquetada, para que
    /// [`StashSpecial`](Self::StashSpecial)/[`SummonSpecial`](Self::SummonSpecial)
    /// la oculten/traigan junto con sus compañeras. Lo usa `pacha` para agrupar
    /// las ventanas de un contexto de usuario sin depender del foco. Las ya
    /// abiertas con ese `app_id` también quedan etiquetadas.
    PlaceAppInSpecial { app_id: String, special: String },
    /// Oculta **todas** las ventanas etiquetadas en el especial `special`
    /// (estén donde estén), apartándolas. Lo usa `pacha` al mandar un contexto a
    /// background.
    StashSpecial(String),
    /// Trae **teselado** (no flotante) al escritorio activo todo lo apartado del
    /// especial `special`. Lo usa `pacha` al volver a un contexto.
    SummonSpecial(String),
    /// Despliega/oculta la terminal dropdown estilo *quake* — un toplevel
    /// real anclado arriba a todo el ancho, con foco de teclado normal. La
    /// crea perezosamente la primera vez (patrón pypr).
    ToggleDropterm,
    /// Pasa al siguiente modo de teselado.
    CycleLayout,
    /// Fija un modo de teselado concreto.
    SetLayout(LayoutMode),
    /// Agranda el área de la ventana maestra (`MasterStack`/`CenteredMaster`).
    GrowMaster,
    /// Encoge el área de la ventana maestra.
    ShrinkMaster,
    /// Mete una ventana más en el área maestra (`nmaster`).
    IncMaster,
    /// Saca una ventana del área maestra.
    DecMaster,
    /// Lleva la ventana enfocada al puesto maestro (la inserta al frente,
    /// desplazando al resto).
    PromoteToMaster,
    /// Intercambia la ventana enfocada con la maestra (sólo esas dos; el
    /// resto del orden queda igual). El foco acompaña a la ventana.
    SwapMaster,
    /// Pliega la pila (las ventanas teseladas que no son maestras) en un
    /// sub-espacio anidado — el árbol fractal. Luego se puede entrar en él con
    /// [`ZoomIn`](DesktopAction::ZoomIn).
    GroupStack,
    /// Pliega la **constelación** de la ventana enfocada (su familia por linaje
    /// de proceso: la terminal y lo que lanzó, …) en un sub-espacio. Como
    /// [`GroupStack`](DesktopAction::GroupStack) pero el criterio es la actividad,
    /// no la posición en la pila.
    GroupConstellation,
    /// Deshace toda la agrupación del escritorio activo: vuelve al teselado plano.
    Ungroup,
    /// Entra ("zoom in") en el sub-espacio que contiene la ventana enfocada:
    /// ese sub-espacio absorbe la pantalla y el resto se aparta.
    ZoomIn,
    /// Sale ("zoom out") un nivel hacia el espacio contenedor.
    ZoomOut,
    /// **Lupa**: acerca el zoom de pantalla completa un paso ([`MAGNIFY_STEP_PCT`]),
    /// hasta [`MAGNIFY_MAX_PCT`]. Accesibilidad para hipermétropes — agranda TODA
    /// la pantalla alrededor del puntero (no es el zoom-Z de ventanas: ver
    /// [`ZoomIn`](DesktopAction::ZoomIn)).
    MagnifyIn,
    /// **Lupa**: aleja el zoom de pantalla completa un paso, hasta apagarlo
    /// ([`MAGNIFY_MIN_PCT`] = 100 % = 1.0×).
    MagnifyOut,
    /// **Lupa**: apaga el zoom de pantalla completa (vuelve a 1.0×).
    MagnifyReset,
    /// **Lupa**: fija el factor de zoom de pantalla completa en un porcentaje
    /// exacto (acotado a `[MAGNIFY_MIN_PCT, MAGNIFY_MAX_PCT]`). Para `mirada-ctl
    /// magnify 200` y el control de pata.
    MagnifySet(u16),
    /// **Grabación de pantalla**: alterna grabar/parar. El Cuerpo decide según su
    /// estado y arma un default (ruta con timestamp) al arrancar. Es lo que va al
    /// atajo de teclado, que no carga parámetros.
    RecordToggle,
    /// **Grabación de pantalla**: arranca con parámetros explícitos (ruta, códec,
    /// audio, fps). La construye `mirada-ctl record start …` y viaja tipada por el
    /// socket de control (su forma textual de keymap usa los defaults).
    RecordStart(mirada_protocol::RecordSpec),
    /// **Grabación de pantalla**: detiene la grabación en curso (no-op si no hay).
    RecordStop,
    /// Salta el foco a la **siguiente constelación** (familia de ventanas por
    /// linaje de proceso) del escritorio activo — el "alt-tab" por actividad, no
    /// por ventana suelta.
    FocusConstellationNext,
    /// Salta el foco a la constelación anterior.
    FocusConstellationPrev,
    /// Cicla al **siguiente escritorio** (Win+Tab). Relativo y con wrap. El
    /// estilo de transición lo decide `Config::workspace_switch_mode`.
    WorkspaceNext,
    /// Cicla al escritorio anterior (Win+Shift+Tab).
    WorkspacePrev,
    /// Activa el escritorio virtual `n` (índice 0-based).
    SwitchWorkspace(usize),
    /// Manda la ventana enfocada al escritorio virtual `n` (queda donde está).
    SendToWorkspace(usize),
    /// Manda la ventana enfocada al escritorio `n` y salta con ella allí.
    MoveToWorkspace(usize),
    /// Mueve el foco a la siguiente salida (monitor).
    FocusOutputNext,
    /// Mueve el foco a la salida (monitor) vecina en una dirección cardinal.
    FocusOutputDir(Direction),
    /// Manda la ventana enfocada a la salida vecina en una dirección — pasa
    /// al escritorio que muestra esa salida.
    SendToOutputDir(Direction),
    /// Redimensiona la ventana flotante enfocada hacia una dirección
    /// (derecha/abajo agrandan; izquierda/arriba achican), por `float_step`
    /// px. No hace nada sobre una teselada.
    ResizeFloatDir(Direction),
    /// Lanza un programa — abre una terminal, un navegador, lo que sea.
    /// El comando se pasa a `sh -c` en el Cuerpo.
    Spawn(String),
    /// Apaga el compositor.
    Quit,
    /// Bloquea la sesión activa: el Cuerpo compone el shell de credenciales
    /// (mirada-greeter en modo lock) encima y le rutea el input hasta que el
    /// usuario desbloquee.
    Lock,
    /// Cierra la sesión activa (FUS logout): el Cuerpo manda cerrar sus ventanas,
    /// la da de baja del roster y pasa a otra sesión hosteada — o al login si no
    /// queda ninguna.
    Logout,
}

/// El nombre RON-seguro de un modo de teselado (sin guiones problemáticos
/// para identificadores: aquí van como valor de cadena, no de enum). Público
/// para que `mirada-ctl`/`pata` reporten el layout activo por su slug.
pub fn layout_slug(mode: LayoutMode) -> &'static str {
    match mode {
        LayoutMode::MasterStack => "master-stack",
        LayoutMode::Monocle => "monocle",
        LayoutMode::Grid => "grid",
        LayoutMode::Columns => "columns",
        LayoutMode::Rows => "rows",
        LayoutMode::CenteredMaster => "centered-master",
        LayoutMode::Spiral => "spiral",
    }
}

/// Modo de teselado desde su `slug`.
pub(crate) fn layout_from_slug(slug: &str) -> Option<LayoutMode> {
    Some(match slug {
        "master-stack" => LayoutMode::MasterStack,
        "monocle" => LayoutMode::Monocle,
        "grid" => LayoutMode::Grid,
        "columns" => LayoutMode::Columns,
        "rows" => LayoutMode::Rows,
        "centered-master" => LayoutMode::CenteredMaster,
        "spiral" => LayoutMode::Spiral,
        _ => return None,
    })
}

impl fmt::Display for DesktopAction {
    /// La forma textual estable de la acción — el vocabulario del keymap.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DesktopAction::FocusNext => f.write_str("focus-next"),
            DesktopAction::FocusPrev => f.write_str("focus-prev"),
            DesktopAction::FocusDir(d) => write!(f, "focus-{}", d.slug()),
            DesktopAction::MoveDir(d) => write!(f, "move-{}", d.slug()),
            DesktopAction::FocusWindow(id) => write!(f, "focus-window:{id}"),
            DesktopAction::MoveForward => f.write_str("move-forward"),
            DesktopAction::MoveBackward => f.write_str("move-backward"),
            DesktopAction::CloseFocused => f.write_str("close-focused"),
            DesktopAction::CloseWindow(id) => write!(f, "close-window:{id}"),
            DesktopAction::ToggleFloat => f.write_str("toggle-float"),
            DesktopAction::ToggleTiling => f.write_str("toggle-tiling"),
            DesktopAction::ToggleFullscreen => f.write_str("toggle-fullscreen"),
            DesktopAction::ToggleMaximize => f.write_str("toggle-maximize"),
            DesktopAction::SendToScratchpad => f.write_str("send-to-scratchpad"),
            DesktopAction::ToggleScratchpad => f.write_str("toggle-scratchpad"),
            DesktopAction::MoveToSpecialWorkspace(name) => write!(f, "move-to-special:{name}"),
            DesktopAction::ToggleSpecialWorkspace(name) => write!(f, "toggle-special:{name}"),
            DesktopAction::PlaceAppInSpecial { app_id, special } => {
                write!(f, "place-app-special:{app_id}:{special}")
            }
            DesktopAction::StashSpecial(name) => write!(f, "stash-special:{name}"),
            DesktopAction::SummonSpecial(name) => write!(f, "summon-special:{name}"),
            DesktopAction::ToggleDropterm => f.write_str("toggle-dropterm"),
            DesktopAction::CycleLayout => f.write_str("cycle-layout"),
            DesktopAction::SetLayout(m) => write!(f, "layout:{}", layout_slug(*m)),
            DesktopAction::GrowMaster => f.write_str("grow-master"),
            DesktopAction::ShrinkMaster => f.write_str("shrink-master"),
            DesktopAction::IncMaster => f.write_str("inc-master"),
            DesktopAction::DecMaster => f.write_str("dec-master"),
            DesktopAction::PromoteToMaster => f.write_str("promote-to-master"),
            DesktopAction::SwapMaster => f.write_str("swap-master"),
            DesktopAction::GroupStack => f.write_str("group-stack"),
            DesktopAction::GroupConstellation => f.write_str("group-constellation"),
            DesktopAction::Ungroup => f.write_str("ungroup"),
            DesktopAction::ZoomIn => f.write_str("zoom-in"),
            DesktopAction::ZoomOut => f.write_str("zoom-out"),
            DesktopAction::MagnifyIn => f.write_str("magnify-in"),
            DesktopAction::MagnifyOut => f.write_str("magnify-out"),
            DesktopAction::MagnifyReset => f.write_str("magnify-reset"),
            DesktopAction::MagnifySet(pct) => write!(f, "magnify:{pct}"),
            // RecordStart lleva un spec que no cabe en la forma textual del
            // keymap; ahí se escribe «record-start» y al parsear usa los defaults
            // (el spec rico viaja tipado por el socket, no por texto).
            DesktopAction::RecordToggle => f.write_str("record-toggle"),
            DesktopAction::RecordStart(_) => f.write_str("record-start"),
            DesktopAction::RecordStop => f.write_str("record-stop"),
            DesktopAction::FocusConstellationNext => f.write_str("focus-constellation-next"),
            DesktopAction::FocusConstellationPrev => f.write_str("focus-constellation-prev"),
            DesktopAction::WorkspaceNext => f.write_str("workspace-next"),
            DesktopAction::WorkspacePrev => f.write_str("workspace-prev"),
            // Los escritorios se numeran 1-based de cara al usuario.
            DesktopAction::SwitchWorkspace(n) => write!(f, "workspace:{}", n + 1),
            DesktopAction::SendToWorkspace(n) => write!(f, "send-to-workspace:{}", n + 1),
            DesktopAction::MoveToWorkspace(n) => write!(f, "move-to-workspace:{}", n + 1),
            DesktopAction::FocusOutputNext => f.write_str("focus-output-next"),
            DesktopAction::FocusOutputDir(d) => write!(f, "focus-output-{}", d.slug()),
            DesktopAction::SendToOutputDir(d) => write!(f, "send-to-output-{}", d.slug()),
            DesktopAction::ResizeFloatDir(d) => write!(f, "resize-float-{}", d.slug()),
            DesktopAction::Spawn(cmd) => write!(f, "spawn:{cmd}"),
            DesktopAction::Quit => f.write_str("quit"),
            DesktopAction::Lock => f.write_str("lock"),
            DesktopAction::Logout => f.write_str("logout"),
        }
    }
}

impl FromStr for DesktopAction {
    /// Mensaje de error ya formateado, listo para mostrar al usuario.
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        Ok(match s {
            "focus-next" => Self::FocusNext,
            "focus-prev" => Self::FocusPrev,
            "move-forward" => Self::MoveForward,
            "move-backward" => Self::MoveBackward,
            "close-focused" => Self::CloseFocused,
            "toggle-float" => Self::ToggleFloat,
            "toggle-tiling" => Self::ToggleTiling,
            "toggle-fullscreen" => Self::ToggleFullscreen,
            "toggle-maximize" => Self::ToggleMaximize,
            "send-to-scratchpad" => Self::SendToScratchpad,
            "toggle-scratchpad" => Self::ToggleScratchpad,
            "toggle-dropterm" => Self::ToggleDropterm,
            "cycle-layout" => Self::CycleLayout,
            "grow-master" => Self::GrowMaster,
            "shrink-master" => Self::ShrinkMaster,
            "inc-master" => Self::IncMaster,
            "dec-master" => Self::DecMaster,
            "promote-to-master" => Self::PromoteToMaster,
            "swap-master" => Self::SwapMaster,
            "group-stack" => Self::GroupStack,
            "group-constellation" => Self::GroupConstellation,
            "ungroup" => Self::Ungroup,
            "zoom-in" => Self::ZoomIn,
            "zoom-out" => Self::ZoomOut,
            "magnify-in" => Self::MagnifyIn,
            "magnify-out" => Self::MagnifyOut,
            "magnify-reset" => Self::MagnifyReset,
            "record-toggle" => Self::RecordToggle,
            "record-start" => Self::RecordStart(mirada_protocol::RecordSpec::default()),
            "record-stop" => Self::RecordStop,
            "focus-constellation-next" => Self::FocusConstellationNext,
            "focus-constellation-prev" => Self::FocusConstellationPrev,
            "workspace-next" => Self::WorkspaceNext,
            "workspace-prev" => Self::WorkspacePrev,
            "focus-output-next" => Self::FocusOutputNext,
            "quit" => Self::Quit,
            "lock" => Self::Lock,
            "logout" => Self::Logout,
            _ => {
                if let Some(slug) = s.strip_prefix("layout:") {
                    Self::SetLayout(
                        layout_from_slug(slug)
                            .ok_or_else(|| format!("modo de teselado desconocido: '{slug}'"))?,
                    )
                } else if let Some(rest) = s.strip_prefix("place-app-special:") {
                    // `place-app-special:<app_id>:<special>` (mirada-ctl une args con ':').
                    let (app_id, special) = rest
                        .split_once(':')
                        .ok_or_else(|| "place-app-special requiere <app_id> <special>".to_string())?;
                    Self::PlaceAppInSpecial {
                        app_id: app_id.trim().to_string(),
                        special: special.trim().to_string(),
                    }
                } else if let Some(name) = s.strip_prefix("stash-special:") {
                    Self::StashSpecial(name.trim().to_string())
                } else if let Some(name) = s.strip_prefix("summon-special:") {
                    Self::SummonSpecial(name.trim().to_string())
                } else if let Some(name) = s.strip_prefix("move-to-special:") {
                    Self::MoveToSpecialWorkspace(name.trim().to_string())
                } else if let Some(name) = s.strip_prefix("toggle-special:") {
                    Self::ToggleSpecialWorkspace(name.trim().to_string())
                } else if let Some(d) = s.strip_prefix("focus-").and_then(Direction::from_slug) {
                    Self::FocusDir(d)
                } else if let Some(d) = s.strip_prefix("focus-output-").and_then(Direction::from_slug) {
                    Self::FocusOutputDir(d)
                } else if let Some(d) = s.strip_prefix("send-to-output-").and_then(Direction::from_slug) {
                    Self::SendToOutputDir(d)
                } else if let Some(d) = s.strip_prefix("resize-float-").and_then(Direction::from_slug) {
                    Self::ResizeFloatDir(d)
                } else if let Some(d) = s.strip_prefix("move-").and_then(Direction::from_slug) {
                    Self::MoveDir(d)
                } else if let Some(id) = s.strip_prefix("focus-window:") {
                    Self::FocusWindow(
                        id.trim()
                            .parse()
                            .map_err(|_| format!("id de ventana inválido: '{id}'"))?,
                    )
                } else if let Some(id) = s.strip_prefix("close-window:") {
                    Self::CloseWindow(
                        id.trim()
                            .parse()
                            .map_err(|_| format!("id de ventana inválido: '{id}'"))?,
                    )
                } else if let Some(n) = s.strip_prefix("send-to-workspace:") {
                    Self::SendToWorkspace(parse_workspace(n)?)
                } else if let Some(n) = s.strip_prefix("move-to-workspace:") {
                    Self::MoveToWorkspace(parse_workspace(n)?)
                } else if let Some(n) = s.strip_prefix("workspace:") {
                    Self::SwitchWorkspace(parse_workspace(n)?)
                } else if let Some(pct) = s.strip_prefix("magnify:") {
                    let pct: u16 = pct
                        .trim()
                        .parse()
                        .map_err(|_| format!("factor de lupa inválido: '{pct}'"))?;
                    Self::MagnifySet(pct.clamp(MAGNIFY_MIN_PCT, MAGNIFY_MAX_PCT))
                } else if let Some(cmd) = s.strip_prefix("spawn:") {
                    let cmd = cmd.trim();
                    if cmd.is_empty() {
                        return Err("spawn: necesita un comando".into());
                    }
                    Self::Spawn(cmd.to_string())
                } else {
                    return Err(format!("acción desconocida: '{s}'"));
                }
            }
        })
    }
}

/// Parsea el número de escritorio del keymap (1-based) a índice (0-based),
/// acotado a [`WORKSPACE_COUNT`].
fn parse_workspace(s: &str) -> Result<usize, String> {
    let n: usize = s
        .trim()
        .parse()
        .map_err(|_| format!("número de escritorio inválido: '{s}'"))?;
    if (1..=WORKSPACE_COUNT).contains(&n) {
        Ok(n - 1)
    } else {
        Err(format!("escritorio fuera de rango (1..={WORKSPACE_COUNT}): {n}"))
    }
}

/// Mapa de teclas por defecto, estilo *tiling WM* (modificador `Super`).
///
/// Las cadenas deben coincidir literalmente con las que el Cuerpo emite
/// en [`BodyEvent::Keybind`](mirada_protocol::BodyEvent::Keybind); son
/// también las que se registran con
/// [`BrainCommand::GrabKeys`](mirada_protocol::BrainCommand::GrabKeys).
pub fn default_keymap() -> Vec<(String, DesktopAction)> {
    let mut map = vec![
        ("Super+j".into(), DesktopAction::FocusNext),
        ("Super+k".into(), DesktopAction::FocusPrev),
        // Alt+Tab clásico — el cycler estándar que cualquier usuario espera.
        // Hoy es un ciclado simple sobre el workspace activo; un HUD con
        // miniaturas queda como evolución del overview de mirada.
        ("Alt+Tab".into(), DesktopAction::FocusNext),
        ("Alt+Shift+Tab".into(), DesktopAction::FocusPrev),
        // Foco espacial estilo i3/sway — el clásico Super+flechas.
        ("Super+Left".into(), DesktopAction::FocusDir(Direction::Left)),
        ("Super+Right".into(), DesktopAction::FocusDir(Direction::Right)),
        ("Super+Up".into(), DesktopAction::FocusDir(Direction::Up)),
        ("Super+Down".into(), DesktopAction::FocusDir(Direction::Down)),
        // Mover la ventana enfocada por geometría — Super+Shift+flechas.
        ("Super+Shift+Left".into(), DesktopAction::MoveDir(Direction::Left)),
        ("Super+Shift+Right".into(), DesktopAction::MoveDir(Direction::Right)),
        ("Super+Shift+Up".into(), DesktopAction::MoveDir(Direction::Up)),
        ("Super+Shift+Down".into(), DesktopAction::MoveDir(Direction::Down)),
        ("Super+Shift+j".into(), DesktopAction::MoveForward),
        ("Super+Shift+k".into(), DesktopAction::MoveBackward),
        ("Super+q".into(), DesktopAction::CloseFocused),
        ("Super+f".into(), DesktopAction::ToggleFloat),
        ("Super+Shift+f".into(), DesktopAction::ToggleFullscreen),
        // La tecla «quake» clásica baja la terminal dropdown.
        ("Super+`".into(), DesktopAction::ToggleDropterm),
        // Scratchpad genérico: enviar la enfocada / invocar la guardada.
        // (Shift+` produce «~» tras canonizar, así que ése es el combo.)
        ("Super+Shift+s".into(), DesktopAction::SendToScratchpad),
        ("Super+Shift+~".into(), DesktopAction::ToggleScratchpad),
        ("Super+space".into(), DesktopAction::CycleLayout),
        ("Super+Shift+space".into(), DesktopAction::ToggleTiling),
        ("Super+t".into(), DesktopAction::SetLayout(LayoutMode::MasterStack)),
        ("Super+m".into(), DesktopAction::SetLayout(LayoutMode::Monocle)),
        ("Super+g".into(), DesktopAction::SetLayout(LayoutMode::Grid)),
        ("Super+c".into(), DesktopAction::SetLayout(LayoutMode::Columns)),
        ("Super+r".into(), DesktopAction::SetLayout(LayoutMode::Rows)),
        ("Super+d".into(), DesktopAction::SetLayout(LayoutMode::CenteredMaster)),
        ("Super+s".into(), DesktopAction::SetLayout(LayoutMode::Spiral)),
        ("Super+h".into(), DesktopAction::ShrinkMaster),
        ("Super+l".into(), DesktopAction::GrowMaster),
        ("Super+o".into(), DesktopAction::FocusOutputNext),
        ("Super+Return".into(), DesktopAction::PromoteToMaster),
        // Árbol fractal: plegar la pila en un sub-espacio y entrar/salir de él.
        ("Super+a".into(), DesktopAction::GroupStack),
        ("Super+Shift+a".into(), DesktopAction::Ungroup),
        // Plegar por constelación (familia de actividad) en vez de por pila.
        ("Super+Shift+c".into(), DesktopAction::GroupConstellation),
        // Alt-tab por constelación: saltar entre familias de actividad.
        // Win+Tab cicla escritorios (el comportamiento esperado). La constelación
        // (alt-tab por actividad) queda sin atajo default — accesible vía perfiles
        // — porque Super+Tab es lo que la gente espera para escritorios.
        ("Super+Tab".into(), DesktopAction::WorkspaceNext),
        ("Super+Shift+Tab".into(), DesktopAction::WorkspacePrev),
        ("Super+i".into(), DesktopAction::ZoomIn),
        ("Super+u".into(), DesktopAction::ZoomOut),
        // Lupa (zoom de pantalla completa, accesibilidad): Super++ acerca,
        // Super+- aleja, Super+0 la apaga. Las teclas «+/-/0» son lo universal
        // para zoom; no chocan con nada del default (los escritorios son 1..9).
        ("Super+equal".into(), DesktopAction::MagnifyIn),
        ("Super+minus".into(), DesktopAction::MagnifyOut),
        ("Super+0".into(), DesktopAction::MagnifyReset),
        // Grabar pantalla (screencast): Super+Shift+R alterna grabar/parar.
        ("Super+Shift+r".into(), DesktopAction::RecordToggle),
        ("Super+Shift+Return".into(), DesktopAction::Spawn("foot".into())),
        ("Super+p".into(), DesktopAction::Spawn("foot -e mirada-launcher".into())),
        // Panel de control unificado (wawa-panel): ajustes de mirada/pata/sistema
        // —cada app una pestaña—, incluida la geometría del Prezi. Super+Shift+p.
        ("Super+Shift+p".into(), DesktopAction::Spawn("wawa-panel".into())),
        ("Super+,".into(), DesktopAction::IncMaster),
        ("Super+.".into(), DesktopAction::DecMaster),
        // Bloquear la sesión: Super+Escape (Super+l ya es GrowMaster).
        ("Super+Escape".into(), DesktopAction::Lock),
        // Cerrar la sesión (FUS logout): Super+Shift+Escape.
        ("Super+Shift+Escape".into(), DesktopAction::Logout),
        ("Super+Shift+e".into(), DesktopAction::Quit),
        // Captura de pantalla (usa el `zwlr_screencopy` del compositor vía grim),
        // convención GNOME: `Print` saca la pantalla entera a un archivo;
        // `Shift+Print` selecciona una región (slurp) y la copia al portapapeles.
        (
            "Print".into(),
            DesktopAction::Spawn(
                "grim \"$HOME/$(date +%Y-%m-%d-%H%M%S)-captura.png\"".into(),
            ),
        ),
        (
            "Shift+Print".into(),
            DesktopAction::Spawn("grim -g \"$(slurp)\" - | wl-copy".into()),
        ),
    ];
    // Un escritorio por dígito: `Super+1`..`Super+9` lo activan,
    // `Super+Shift+1`.. mandan la ventana enfocada allí.
    for n in 0..WORKSPACE_COUNT {
        map.push((format!("Super+{}", n + 1), DesktopAction::SwitchWorkspace(n)));
        map.push((
            format!("Super+Shift+{}", n + 1),
            DesktopAction::SendToWorkspace(n),
        ));
    }
    map
}

// ===================================================================
// Presets de atajos (perfiles de fábrica)
// ===================================================================
//
// Cada preset es un mapa completo, autocontenido, que reproduce el
// *muscle memory* de un WM conocido traducido al vocabulario de
// [`DesktopAction`] de mirada. mirada no tiene todos los conceptos de
// cada WM (p. ej. el `dwindle` de Hyprland o el árbol de splits de i3),
// así que algunas teclas se mapean a la acción más cercana — se anota
// con un comentario `≈` dónde la equivalencia es aproximada.
//
// Los perfiles se gestionan en [`crate::profiles`]: el usuario los
// duplica, edita, borra y conmuta. `dwm` es el preset por defecto
// ([`default_keymap`]) — el que históricamente trae mirada.

/// Los nombres de los presets de fábrica, en orden de presentación. `mirada`
/// es el nativo (el keymap actual) y encabeza la lista.
pub const PRESET_NAMES: [&str; 6] = ["mirada", "dwm", "i3", "hyprland", "windows", "mac"];

/// El keymap de un preset de fábrica por nombre, o `None` si no existe.
pub fn preset_keymap(name: &str) -> Option<Vec<(String, DesktopAction)>> {
    Some(match name {
        "mirada" => mirada_keymap(),
        "dwm" => dwm_keymap(),
        "i3" => i3_keymap(),
        "hyprland" => hyprland_keymap(),
        "windows" => windows_keymap(),
        "mac" => mac_keymap(),
        _ => return None,
    })
}

/// Preset **mirada** — el keymap **nativo y actual** del compositor: es
/// exactamente [`default_keymap`]. La vista `mirada` lo usa; es el default.
pub fn mirada_keymap() -> Vec<(String, DesktopAction)> {
    default_keymap()
}

/// Preset **dwm** — la herencia dwm de mirada: `Super` como modificador, foco
/// cíclico por la pila (`Super+j/k`), maestra+pila, zoom con `Super+Return`,
/// terminal con `Super+Shift+Return`. Hoy coincide con [`default_keymap`].
pub fn dwm_keymap() -> Vec<(String, DesktopAction)> {
    default_keymap()
}

/// Preset **windows** — escritorio apilado estilo Windows (vistas `windows-xp`
/// y `kde`): `Alt+Tab` cicla ventanas, `Alt+F4` cierra, `Super+E` el explorador,
/// `Super+R` el lanzador, `Super+D` muestra el escritorio (todo flotante),
/// `Super+↑` maximiza, `Super+←/→/↓` acomoda (snap).
pub fn windows_keymap() -> Vec<(String, DesktopAction)> {
    use DesktopAction::*;
    use Direction::{Down, Left, Right};
    let mut map = vec![
        ("Alt+Tab".into(), FocusNext),
        ("Alt+Shift+Tab".into(), FocusPrev),
        ("Alt+F4".into(), CloseFocused),
        ("Super+e".into(), Spawn("nada".into())),
        ("Super+r".into(), Spawn("foot -e mirada-launcher".into())),
        ("Super+d".into(), ToggleTiling), // «mostrar escritorio» ≈ todo flotante
        ("Super+Up".into(), ToggleFullscreen),
        ("Super+Left".into(), MoveDir(Left)),
        ("Super+Right".into(), MoveDir(Right)),
        ("Super+Down".into(), MoveDir(Down)),
        ("F11".into(), ToggleFullscreen),
        ("Super+grave".into(), ToggleDropterm),
    ];
    for n in 0..WORKSPACE_COUNT {
        map.push((format!("Super+{}", n + 1), SwitchWorkspace(n)));
        map.push((format!("Super+Shift+{}", n + 1), SendToWorkspace(n)));
    }
    map
}

/// Preset **mac** — atajos estilo macOS (vista `mac`), con `Super` como ⌘:
/// `Super+Q`/`Super+W` cierran, `Super+Tab` cicla, `Super+space` el lanzador
/// (Spotlight), `Super+Ctrl+F` pantalla completa, `Super+M`/`Super+H` esconden
/// (scratchpad ≈ minimizar/ocultar).
pub fn mac_keymap() -> Vec<(String, DesktopAction)> {
    use DesktopAction::*;
    let mut map = vec![
        ("Super+q".into(), CloseFocused),
        ("Super+w".into(), CloseFocused),
        ("Super+Tab".into(), FocusNext),
        ("Super+Shift+Tab".into(), FocusPrev),
        ("Super+grave".into(), FocusNext), // ⌘` : ventanas de la misma app
        ("Super+space".into(), Spawn("foot -e mirada-launcher".into())), // Spotlight
        ("Super+Ctrl+f".into(), ToggleFullscreen),
        ("Super+m".into(), SendToScratchpad), // ⌘M : minimizar ≈ esconder
        ("Super+h".into(), SendToScratchpad), // ⌘H : ocultar
        ("Super+n".into(), Spawn("foot".into())), // ⌘N : nueva ventana ≈ terminal
        // Captura: `Print` pantalla entera → archivo, `Shift+Print` región →
        // portapapeles (⌘⇧3/4 chocan con los escritorios Super+Shift+N de abajo).
        (
            "Print".into(),
            Spawn("grim \"$HOME/$(date +%Y-%m-%d-%H%M%S)-captura.png\"".into()),
        ),
        (
            "Shift+Print".into(),
            Spawn("grim -g \"$(slurp)\" - | wl-copy".into()),
        ),
    ];
    for n in 0..WORKSPACE_COUNT {
        map.push((format!("Super+{}", n + 1), SwitchWorkspace(n)));
        map.push((format!("Super+Shift+{}", n + 1), SendToWorkspace(n)));
    }
    map
}

/// Preset **i3 / sway** — foco y movimiento vim `h/j/k/l` (la convención
/// moderna de sway) y por flechas; `Super+Return` abre terminal,
/// `Super+Shift+q` cierra, `Super+d` lanza el menú, `Super+f` pantalla
/// completa, `Super+Shift+space` flota, `Super+Shift+e` sale. Los modos de
/// layout de i3 (split/stacking/tabbed) se mapean a los teselados de mirada.
pub fn i3_keymap() -> Vec<(String, DesktopAction)> {
    use DesktopAction::*;
    use Direction::{Down, Left, Right, Up};
    let mut map = vec![
        // --- Apps y sistema ---
        ("Super+Return".into(), Spawn("foot".into())),
        ("Super+Shift+q".into(), CloseFocused),
        ("Super+d".into(), Spawn("foot -e mirada-launcher".into())),
        ("Super+Shift+e".into(), Quit),
        ("Super+Shift+s".into(), SendToScratchpad),
        ("Super+Shift+grave".into(), ToggleScratchpad), // ~ : muestra el scratchpad
        // --- Foco (h/j/k/l estilo sway + flechas) ---
        ("Super+h".into(), FocusDir(Left)),
        ("Super+j".into(), FocusDir(Down)),
        ("Super+k".into(), FocusDir(Up)),
        ("Super+l".into(), FocusDir(Right)),
        ("Super+Left".into(), FocusDir(Left)),
        ("Super+Down".into(), FocusDir(Down)),
        ("Super+Up".into(), FocusDir(Up)),
        ("Super+Right".into(), FocusDir(Right)),
        // --- Mover la ventana ---
        ("Super+Shift+h".into(), MoveDir(Left)),
        ("Super+Shift+j".into(), MoveDir(Down)),
        ("Super+Shift+k".into(), MoveDir(Up)),
        ("Super+Shift+l".into(), MoveDir(Right)),
        ("Super+Shift+Left".into(), MoveDir(Left)),
        ("Super+Shift+Down".into(), MoveDir(Down)),
        ("Super+Shift+Up".into(), MoveDir(Up)),
        ("Super+Shift+Right".into(), MoveDir(Right)),
        // --- Estado y layout ---
        ("Super+f".into(), ToggleFullscreen),
        ("Super+Shift+space".into(), ToggleFloat),
        ("Super+space".into(), CycleLayout),
        ("Super+e".into(), CycleLayout), // i3: «layout toggle split»
        ("Super+s".into(), SetLayout(LayoutMode::Spiral)), // i3 «stacking» ≈ espiral
        ("Super+w".into(), SetLayout(LayoutMode::Grid)), // i3 «tabbed» ≈ grilla
        ("Super+t".into(), SetLayout(LayoutMode::MasterStack)),
        ("Super+a".into(), ZoomOut), // i3 «focus parent» ≈ subir un nivel
        // --- Maestra (i3 usa un modo resize; aquí teclas directas) ---
        ("Super+minus".into(), ShrinkMaster),
        ("Super+equal".into(), GrowMaster),
    ];
    for n in 0..WORKSPACE_COUNT {
        map.push((format!("Super+{}", n + 1), SwitchWorkspace(n)));
        map.push((format!("Super+Shift+{}", n + 1), SendToWorkspace(n)));
    }
    map
}

/// Preset **Hyprland** — su identidad propia: `Super+Q` abre terminal,
/// `Super+C` cierra, `Super+M` sale de la sesión, `Super+E` el gestor de
/// archivos, `Super+V` flota, `Super+R` el menú, flechas mueven el foco,
/// `Super+S` el «special workspace» (scratchpad). Manda la ventana al
/// escritorio **siguiéndola** (`movetoworkspace`).
pub fn hyprland_keymap() -> Vec<(String, DesktopAction)> {
    use DesktopAction::*;
    use Direction::{Down, Left, Right, Up};
    let mut map = vec![
        ("Super+q".into(), Spawn("foot".into())), // Q = terminal
        ("Super+c".into(), CloseFocused),         // C = killactive
        ("Super+m".into(), Quit),                 // M = exit
        ("Super+e".into(), Spawn("nada".into())), // E = gestor de archivos
        ("Super+v".into(), ToggleFloat),          // V = togglefloating
        ("Super+r".into(), Spawn("foot -e mirada-launcher".into())), // R = menú
        ("Super+p".into(), PromoteToMaster),      // P = pseudo ≈ promover a maestra
        ("Super+j".into(), CycleLayout),          // J = togglesplit ≈ ciclar layout
        ("Super+f".into(), ToggleFullscreen),
        ("Super+Left".into(), FocusDir(Left)),
        ("Super+Right".into(), FocusDir(Right)),
        ("Super+Up".into(), FocusDir(Up)),
        ("Super+Down".into(), FocusDir(Down)),
        ("Super+Shift+Left".into(), MoveDir(Left)),
        ("Super+Shift+Right".into(), MoveDir(Right)),
        ("Super+Shift+Up".into(), MoveDir(Up)),
        ("Super+Shift+Down".into(), MoveDir(Down)),
        ("Super+s".into(), ToggleScratchpad), // S = special workspace
        ("Super+Shift+s".into(), SendToScratchpad), // Shift+S = mover al special
        ("Super+grave".into(), ToggleDropterm),
    ];
    for n in 0..WORKSPACE_COUNT {
        map.push((format!("Super+{}", n + 1), SwitchWorkspace(n)));
        // Hyprland: `movetoworkspace` salta con la ventana al escritorio.
        map.push((format!("Super+Shift+{}", n + 1), MoveToWorkspace(n)));
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_has_no_duplicate_bindings() {
        let map = default_keymap();
        let mut keys: Vec<_> = map.iter().map(|(k, _)| k.clone()).collect();
        keys.sort();
        let unique = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), unique, "hay un atajo repetido");
    }

    #[test]
    fn keymap_covers_every_virtual_workspace() {
        let map = default_keymap();
        for n in 0..WORKSPACE_COUNT {
            assert!(map
                .iter()
                .any(|(_, a)| a == &DesktopAction::SwitchWorkspace(n)));
            assert!(map
                .iter()
                .any(|(_, a)| a == &DesktopAction::SendToWorkspace(n)));
        }
    }

    #[test]
    fn every_default_action_round_trips_through_its_text_form() {
        for (_, action) in default_keymap() {
            let text = action.to_string();
            let back: DesktopAction = text.parse().unwrap();
            assert_eq!(action, back, "no redondea: {text}");
        }
    }

    #[test]
    fn every_layout_mode_round_trips() {
        for mode in LayoutMode::ALL {
            let a = DesktopAction::SetLayout(mode);
            assert_eq!(a, a.to_string().parse().unwrap());
        }
    }

    #[test]
    fn workspace_actions_are_one_based_in_text() {
        assert_eq!(DesktopAction::SwitchWorkspace(0).to_string(), "workspace:1");
        assert_eq!(
            "workspace:1".parse::<DesktopAction>().unwrap(),
            DesktopAction::SwitchWorkspace(0)
        );
        assert_eq!(
            "send-to-workspace:9".parse::<DesktopAction>().unwrap(),
            DesktopAction::SendToWorkspace(8)
        );
    }

    #[test]
    fn out_of_range_or_unknown_actions_are_rejected() {
        assert!("workspace:0".parse::<DesktopAction>().is_err());
        assert!("workspace:99".parse::<DesktopAction>().is_err());
        assert!("layout:fractal".parse::<DesktopAction>().is_err());
        assert!("focus-window:abc".parse::<DesktopAction>().is_err());
        assert!("teleport".parse::<DesktopAction>().is_err());
    }

    #[test]
    fn directional_actions_round_trip_in_every_direction() {
        for d in [Direction::Left, Direction::Right, Direction::Up, Direction::Down] {
            for a in [
                DesktopAction::FocusDir(d),
                DesktopAction::MoveDir(d),
                DesktopAction::FocusOutputDir(d),
                DesktopAction::SendToOutputDir(d),
                DesktopAction::ResizeFloatDir(d),
            ] {
                let text = a.to_string();
                assert_eq!(text.parse::<DesktopAction>().unwrap(), a, "no redondea: {text}");
            }
        }
    }

    #[test]
    fn focus_window_round_trips_with_its_id() {
        let a = DesktopAction::FocusWindow(42);
        assert_eq!(a.to_string(), "focus-window:42");
        assert_eq!("focus-window:42".parse::<DesktopAction>().unwrap(), a);
    }

    #[test]
    fn close_window_round_trips_with_its_id() {
        let a = DesktopAction::CloseWindow(7);
        assert_eq!(a.to_string(), "close-window:7");
        assert_eq!("close-window:7".parse::<DesktopAction>().unwrap(), a);
        // id inválido → error, no panic.
        assert!("close-window:abc".parse::<DesktopAction>().is_err());
    }

    #[test]
    fn spawn_round_trips_keeping_the_whole_command() {
        let a = DesktopAction::Spawn("foot --title diario".into());
        assert_eq!(a.to_string(), "spawn:foot --title diario");
        assert_eq!(a.to_string().parse::<DesktopAction>().unwrap(), a);
    }

    #[test]
    fn spawn_without_a_command_is_rejected() {
        assert!("spawn:".parse::<DesktopAction>().is_err());
        assert!("spawn:   ".parse::<DesktopAction>().is_err());
    }

    #[test]
    fn every_preset_resolves_and_has_no_duplicate_bindings() {
        for name in PRESET_NAMES {
            let map = preset_keymap(name).unwrap_or_else(|| panic!("preset {name} no resuelve"));
            assert!(!map.is_empty(), "preset {name} vacío");
            let mut keys: Vec<_> = map.iter().map(|(k, _)| k.clone()).collect();
            keys.sort();
            let unique = keys.len();
            keys.dedup();
            assert_eq!(keys.len(), unique, "preset {name} tiene un atajo repetido");
        }
    }

    #[test]
    fn every_preset_binding_round_trips_through_its_text_form() {
        for name in PRESET_NAMES {
            for (_, action) in preset_keymap(name).unwrap() {
                let text = action.to_string();
                assert_eq!(text.parse::<DesktopAction>().unwrap(), action, "{name}: {text}");
            }
        }
    }

    #[test]
    fn every_preset_covers_the_nine_workspaces() {
        for name in PRESET_NAMES {
            let map = preset_keymap(name).unwrap();
            for n in 0..WORKSPACE_COUNT {
                assert!(
                    map.iter().any(|(_, a)| a == &DesktopAction::SwitchWorkspace(n)),
                    "preset {name} no cubre el escritorio {n}"
                );
            }
        }
    }

    #[test]
    fn presets_keep_their_identity_keys() {
        // Hyprland: Super+Q abre terminal, no cierra.
        let hypr = preset_keymap("hyprland").unwrap();
        assert!(hypr
            .iter()
            .any(|(k, a)| k == "Super+q" && matches!(a, DesktopAction::Spawn(_))));
        assert!(hypr
            .iter()
            .any(|(k, a)| k == "Super+m" && a == &DesktopAction::Quit));
        // i3: Super+Return abre terminal, Super+Shift+q cierra.
        let i3 = preset_keymap("i3").unwrap();
        assert!(i3
            .iter()
            .any(|(k, a)| k == "Super+Return" && matches!(a, DesktopAction::Spawn(_))));
        assert!(i3
            .iter()
            .any(|(k, a)| k == "Super+Shift+q" && a == &DesktopAction::CloseFocused));
    }
}
