//! # shuma-voz — núcleo agnóstico de la voz manos-libres
//!
//! La voz en shuma es **otra superficie de E/S opt-in y rotulada**, no un
//! piloto (ver `02_ruway/shuma/VOZ.md`). Este crate es la parte **sync, pura y
//! testeable** — espeja el reparto de `shuma-agente`: *el host expresa la
//! intención y corre el mundo* (cpal, VAD, STT, TTS, extracción de f0); *este
//! núcleo sólo decide*. No toca audio, ni red, ni `tokio`.
//!
//! Tres piezas, una por capacidad de `VOZ.md`:
//!
//! - [`maquina`] — la escucha manos-libres como máquina de estados
//!   `Dormido → Despierto → Dictando`. El VAD/STT del host la alimenta con
//!   [`Evento`]s; ella devuelve [`Reaccion`]es (despertó, dictar texto, se
//!   durmió). **Nada sale de la máquina hasta que matchea el llamado.**
//! - [`lectura`] — política de TTS *discriminado*: se vocaliza **sólo la
//!   prosa** del agente, nunca código ni acciones. El host mapea
//!   `shuma_agente::BloqueSalida` → [`lectura::TipoBloque`].
//! - [`prosodia`] — clasificador determinista de entonación: de los rasgos de
//!   f0 que extrae el host saca una *pista* de [`prosodia::Intencion`]
//!   (pregunta / orden / urgencia). Es una pista, no un veredicto.

pub mod lectura;
pub mod maquina;
pub mod prosodia;

pub use lectura::{debe_leer, TipoBloque};
pub use maquina::{ConfigVoz, EstadoVoz, Evento, Maquina, Reaccion};
pub use prosodia::{clasificar, Intencion, Rasgos};
