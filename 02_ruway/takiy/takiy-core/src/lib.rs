//! `takiy-core` — teoría musical y modelo de partitura.
//!
//! La base agnóstica de takiy (composición musical asistida): nada de
//! síntesis de audio, nada de IA, nada de UI — sólo los tipos puros que
//! todo lo demás comparte.
//!
//! - [`pitch`] — alturas MIDI, clases de altura, frecuencias.
//! - [`scale`] — escalas como raíz + patrón de semitonos.
//! - [`chord`] — acordes como raíz + cualidad armónica.
//! - [`score`] — `ScoreNote`, `Track` y un `Score` multipista con tempo.
//!
//! El tiempo se mide en pulsos: una partitura es independiente del tempo
//! hasta reproducirla. La síntesis (`takiy-synth`) y la asistencia por
//! IA (`takiy-ai`) se construyen encima sin tocar este crate.

#![forbid(unsafe_code)]

pub mod chord;
pub mod pitch;
pub mod scale;
pub mod score;

pub use chord::{Chord, ChordQuality};
pub use pitch::{Pitch, PitchClass};
pub use scale::Scale;
pub use score::{
    AutomationLane, AutomationPoint, DelayParams, ReverbParams, Score, ScoreNote, Track, TrackView,
};
