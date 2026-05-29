//! media-source-av1 — decode **AV1 nativo** (puro-Rust) del dominio media.
//!
//! AV1 + Opus son el formato de medios NATIVO de gioser (PLAN.md
//! §6.quinquies): sin C, sin patentes, compila a WASM, corre igual en
//! wawa. Este crate cubre el video AV1 sobre contenedor **IVF** en tres
//! capas puro-Rust + una de decode:
//!
//! - [`ivf`]: demuxer IVF (cabecera + temporal units). Puro-Rust, sin
//!   dependencias — sirve sin el feature `decode`.
//! - [`obu`]: splitter de OBUs (inspección de bitstream, LEB128). Idem.
//! - [`Av1VideoSource`] (feature `decode`, on por defecto): demuxea +
//!   decodifica con `rav1d` (port puro-Rust de dav1d) y entrega frames
//!   RGBA como [`media_core::FrameSource`].
//!
//! Los códecs ajenos (H.264/H.265/AAC, o cualquier contenedor que no sea
//! AV1-en-IVF) entran por `shared/foreign-av` (puente ffmpeg), que además
//! sabe transcodificar a AV1 al importar. La división nativo/foreign es
//! la regla dura #4 aplicada al video.
//!
//! ## Audio nativo (pendiente)
//!
//! El audio nativo de gioser es **Opus**, pero no hay decoder Opus
//! puro-Rust maduro hoy (symphonia trae mp3/vorbis/flac/aac pero NO
//! Opus). Hasta que lo haya, el audio de un contenedor AV1+Opus se saca
//! por `shared/foreign-av`. Este crate cubre sólo el video.

pub mod ivf;
pub mod obu;

#[cfg(feature = "decode")]
mod decode;

#[cfg(feature = "decode")]
pub use decode::Av1VideoSource;

pub use ivf::{IvfHeader, IvfReader, TemporalUnit};
pub use obu::{read_leb128, split_obus, Obu, ObuKind};
