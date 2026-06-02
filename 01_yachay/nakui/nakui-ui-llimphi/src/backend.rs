//! Backend de Nakui — re-exporta `nakui-backend` (regla #2: el motor
//! WAL/snapshot/compaction + impl `MetaBackend` vive en un core agnóstico
//! de GUI, no en el frontend). Este `mod backend` se mantiene como fachada
//! para que `crate::backend::X` siga resolviendo sin tocar los callers.

pub use nakui_backend::*;
