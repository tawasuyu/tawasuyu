//! Re-export de las operaciones de archivo del shell, que ahora viven en el core
//! agnóstico `nahual-shell-core::ops` (Regla 2): la cola y la ejecución por
//! `SourceMut` no tocan UI ni `Handle`. El frontend las sigue viendo como
//! `crate::ops::*` sin cambios; sólo el ruteo (encolar/lanzar worker) vive acá.

pub use nahual_shell_core::ops::*;
