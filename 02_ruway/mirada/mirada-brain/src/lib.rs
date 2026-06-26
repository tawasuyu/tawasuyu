//! `mirada-brain` — el orquestador de escritorio del compositor.
//!
//! Es el "Cerebro" de mirada sin pantalla: mantiene el estado del
//! escritorio (salidas, escritorios virtuales, ventanas, foco), consume
//! los [`BodyEvent`]s que reporta el Cuerpo y produce los
//! [`BrainCommand`]s que el Cuerpo aplica.
//!
//! Es agnóstico de GPUI y de `smithay`: una app GPUI sólo lo *envuelve*
//! para pintar un HUD y para mover los bytes por el cable de
//! [`mirada_protocol`]. Toda la lógica vive aquí y es determinista —
//! la misma secuencia de eventos da siempre el mismo estado.
//!
//! - [`action`] — las acciones de escritorio y el mapa de teclas.
//! - [`config`] — la [`Config`] general del WM (dropterm, teselado, foco).
//! - [`desktop`] — el [`Desktop`]: el estado y el bucle `evento → comandos`.
//! - [`keymap`] — el [`Keymap`] configurable en RON, recargable en caliente.
//! - [`rules`] — las [`Rules`] de ventana (escritorio/flotante por `app_id`).
//! - [`session`] — la [`DesktopState`]: la forma del escritorio entre arranques.
//! - [`ctl`] — el API de control externo (`mirada-ctl`, taskbars, scripts).

#![forbid(unsafe_code)]

pub mod action;
pub mod activity;
pub mod config;
pub mod ctl;
pub mod desktop;
pub mod git_branch;
pub mod keymap;
pub mod permisos;
pub mod profiles;
pub mod rules;
pub mod session;
/// `impl allichay::Configurable for Config` — vuelve la config editable por UI.
pub mod settings;
pub mod vistas;
pub mod watch;

pub use action::{
    default_keymap, dwm_keymap, hyprland_keymap, i3_keymap, layout_slug, preset_keymap,
    DesktopAction, PRESET_NAMES, WORKSPACE_COUNT,
};
pub use activity::{ActivityGraph, Lineage};
pub use config::{
    default_root_menu, default_zones, waypipe_ssh_command, Config, MenuEntry, OutputOverride,
    OverviewPlace, StartupApp, WorkspaceSwitchMode, ZoneCfg,
    DROPTERM_APP_ID,
};
pub use ctl::{CtlConn, CtlReply, CtlRequest, CtlServer, WindowLine, WorkspacesState};
pub use desktop::{Desktop, Output, WindowInfo};
pub use git_branch::{BranchSwitch, GitBranchWatch};
pub use keymap::{Keymap, KeymapError, KeymapWatch};
pub use permisos::Permisos;
pub use profiles::{KeymapProfiles, ProfileError};
pub use rules::{Rule, RuleOutcome, Rules};
pub use vistas::{Vista, VISTA_NAMES};
pub use session::{DesktopState, SESSION_VERSION};
pub use watch::FileWatch;

pub use mirada_layout::{
    disponer, envolvente, wallpaper_dst_rect, Disposicion, LayoutMode, LayoutParams, Rect,
    WallpaperFit, WindowId, Workspace, ZoneFrac,
};
pub use mirada_protocol::{BodyEvent, BrainCommand, Decorations, OutputId, WindowPlacement};
