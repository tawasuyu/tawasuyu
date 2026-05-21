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
//! - [`desktop`] — el [`Desktop`]: el estado y el bucle `evento → comandos`.
//! - [`keymap`] — el [`Keymap`] configurable en RON, recargable en caliente.
//! - [`rules`] — las [`Rules`] de ventana (escritorio/flotante por `app_id`).
//! - [`ctl`] — el API de control externo (`mirada-ctl`, taskbars, scripts).

#![forbid(unsafe_code)]

pub mod action;
pub mod ctl;
pub mod desktop;
pub mod keymap;
pub mod rules;

pub use action::{default_keymap, DesktopAction, WORKSPACE_COUNT};
pub use ctl::{CtlConn, CtlReply, CtlRequest, CtlServer, WindowLine};
pub use desktop::{Desktop, WindowInfo};
pub use keymap::{Keymap, KeymapError, KeymapWatch};
pub use rules::{Rule, RuleOutcome, Rules};

pub use mirada_layout::{LayoutMode, LayoutParams, Rect, WindowId, Workspace};
pub use mirada_protocol::{BodyEvent, BrainCommand, OutputId, WindowPlacement};
