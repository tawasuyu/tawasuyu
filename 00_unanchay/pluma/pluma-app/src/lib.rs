//! `pluma-app` como biblioteca: expone los módulos internos del editor
//! multilienzo para que el binario (`src/main.rs`) y los `examples/` (el
//! showreel headless) compartan exactamente la misma `vista()` y el mismo
//! `Model`. El producto sigue siendo el binario; la biblioteca sólo existe
//! para no duplicar la vista en el reel.

pub(crate) mod clipboard;
pub mod dump;
pub mod init;
pub mod model;
pub mod showreel;
pub mod update;
pub(crate) mod util;
pub mod view;
