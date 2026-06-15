//! media-core — productores de video y audio del dominio.
//!
//! Dos traits gemelos:
//!
//! - [`FrameSource`]: entrega bytes RGBA con un tamaño. Lo consume
//!   `llimphi-surface` para subirlo a una textura GPU.
//! - [`AudioSource`]: rellena un buffer de samples `f32` intercalados
//!   por canal a una sample rate dada. Lo consume un sink (cpal, JACK,
//!   wawa) que se encarga del realtime.
//!
//! Ambos vienen con una implementación procedural de referencia:
//! [`TestCard`] (gradiente animado + círculo rebotando) para video y
//! [`ToneSource`] (senoide configurable, default A4) para audio. Son
//! los "test patterns" del dominio: validan los pipelines completos
//! sin meter decoders externos.
//!
//! El crate es `std` y no tiene dependencias — la idea es que el
//! núcleo del dominio sea liviano y los backends pesados (ffmpeg,
//! gstreamer, v4l2, cpal…) vivan en crates `media-source-*` o
//! `media-audio-*` que impl los traits.

// ── Submódulos del núcleo (nuevos — partición de lib.rs monolítico) ──────────
pub mod audio;
pub mod frame;
pub mod spectrum;
pub mod subtitles;
pub mod transport;

// ── Submódulos de dominio existentes ─────────────────────────────────────────
pub mod chapters;
pub mod channels;
pub mod color;
pub mod config;
pub mod control;
pub mod dynamics;
pub mod eq;
pub mod fade;
pub mod layout;
pub mod library;
pub mod loudness;
pub mod metadata;
pub mod osd;
pub mod playlist;
pub mod profile;
pub mod seek;
pub mod sync;
pub mod toolbar;
pub mod thumbnail;
pub mod tracks;
pub mod transform;
pub mod viewport;
pub mod waveform;

// ── Re-exportaciones para mantener la API pública intacta ────────────────────

// Traits principales de producción
pub use audio::AudioSource;
pub use frame::{FrameSource, Seekable};

// Implementaciones de referencia (test patterns)
pub use audio::{AudioProbe, ProbedAudioSource, ToneSource};
pub use frame::TestCard;

// Controles de transporte
pub use transport::{
    MixerAudio, Pause, PausableAudio, PausableVideo, VideoSwitch, VideoSwitcher, Volume,
    VolumeAudio,
};

// Análisis espectral y niveles
pub use spectrum::{Levels, Spectrum, Waterfall};

// Subtítulos
pub use subtitles::{
    AssColor, StyleSheet, SubAlign, SubtitleCue, SubtitleStyle, SubtitleTrack,
};
