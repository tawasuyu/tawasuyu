//! `voz-core` — el contrato agnóstico de **habla** para toda la suite.
//!
//! Gemelo de [`rimay-verbo-core`](../rimay-verbo-core): si verbo es *entender*
//! texto (embeddings), voz es *oír y decir*. STT/TTS son IA **general** — no
//! viven en ninguna app concreta; cualquiera (shuma, mirada, pluma) consume
//! este contrato. Las impls concretas (`rimay-voz-whisper`, `rimay-voz-piper`,
//! `rimay-voz-mock`, nube) cumplen los traits [`Transcriptor`] y [`Locutor`].
//!
//! Además del contrato, el crate trae la **lógica general de escucha
//! manos-libres** — pura, sync y testeable, sin tocar audio ni red (el host
//! corre cpal/VAD/STT/TTS y la alimenta):
//!
//! - [`vad`] — la primera compuerta: detector de voz por frame (trait +
//!   default de energía, Silero después) + segmentador de utterances. Entrega
//!   el fragmento de audio que va al STT. **Nada pesado corre hasta que hay voz.**
//! - [`wake`] — la segunda compuerta (F1): ¿esta utterance suena al llamado?
//!   Si no, no se transcribe (ni se manda a la nube). Trait `DetectorLlamado` +
//!   default por plantilla enrolada (DTW, sin modelo); neuronal después.
//! - [`maquina`] — la escucha como autómata `Dormido → Despierto → Dictando`,
//!   con detección del llamado. **Nada cruza al consumidor hasta el llamado.**
//! - [`lectura`] — política de TTS *discriminado*: se vocaliza sólo la prosa,
//!   nunca código ni acciones.
//! - [`prosodia`] — clasificador determinista de entonación (pista de
//!   intención a partir de rasgos de f0, no un veredicto).

#![forbid(unsafe_code)]

pub mod lectura;
pub mod maquina;
pub mod prosodia;
mod traits;
pub mod vad;
pub mod wake;

pub use lectura::{debe_leer, Politica, TipoBloque};
pub use maquina::{detectar_llamado, ConfigVoz, EstadoVoz, Evento, Maquina, Reaccion};
pub use prosodia::{clasificar, Intencion, Rasgos};
pub use traits::{Audio, Locutor, Transcriptor, Transcripcion, VozError};
pub use vad::{
    ConfigVad, DetectorEnergia, DetectorVoz, PulsoVad, SalidaVad, Segmentador, Vad,
};
pub use wake::{
    DetectorLlamado, DetectorPlantilla, ParamsLlamado, Plantilla, UMBRAL_LLAMADO_DEFAULT,
};
