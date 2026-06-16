//! Re-export de la lectura de `/proc`, que ahora vive en el core agnóstico
//! `sandokan-sysmon-core::procfs` (Regla 2): barrido de procesos, jiffies y
//! envío de señales no tocan UI. El frontend la sigue viendo como
//! `super::procfs::*`; sólo el ruteo (spawn del scan, manejo de `Msg`,
//! derivación contra el `Model`) vive en este crate.

pub use sandokan_sysmon_core::procfs::*;
