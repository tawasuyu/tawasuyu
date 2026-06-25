//! ente-kernel: primitivas de Linux que el Init usa pero que son reusables
//! desde tools/tests/sub-supervisores. Sin estado global. Cada función es
//! independiente y se puede testear de forma aislada.

pub mod sigchld;
pub mod surface;
pub mod uevent;
pub mod watchdog;

pub use sigchld::spawn_sigchld_stream;
pub use surface::{become_child_subreaper, bootstrap_kernel_surface};
pub use uevent::{spawn_uevent_stream, UAction, UEvent};
pub use watchdog::Watchdog;
