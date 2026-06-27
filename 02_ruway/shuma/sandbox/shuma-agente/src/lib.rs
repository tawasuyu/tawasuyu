//! # shuma-agente — núcleo de la IA conversacional de shuma
//!
//! Hoy la IA de shuma es **invocación atómica**: cada `:?`/`:hacé`/`:explica`
//! arma un `ChatRequest::una_vuelta` (turno único, sin memoria) y vuelca la
//! respuesta al input o al bloque. Este crate sube un escalón: modela
//! **múltiples agentes** configurables y **conversaciones multi-turno**
//! persistidas — el modelo de las apps web de IA (un panel de charlas, cada una
//! contra un agente), pero embebido en la suite.
//!
//! ## Reparto de responsabilidades (mismo patrón que el resto de shuma)
//!
//! El módulo/host **expresa la intención y corre la red**; este núcleo es
//! **sync, puro y testeable**, sin tocar sockets ni `tokio`:
//!
//! - [`Agente`] — identidad + backend ([`wawa_config::LlmSettings`] por agente)
//!   + persona (`system_prompt`) + qué [`Capacidades`] de control puede proponer.
//! - [`Conversacion`] — hilo multi-turno ([`Turno`]s usuario/asistente), cada
//!   turno del asistente desglosado en [`BloqueSalida`]s (texto, código, acción).
//! - [`motor`] — `construir_request` arma el [`ChatRequest`] con todo el
//!   historial; `interpretar_respuesta` parte el texto crudo del modelo en
//!   bloques (la **gama de outputs**). El host hace el `.complete()` con
//!   `pluma-llm` en un thread, igual que con el `LlmRequest` del shell.
//! - [`Almacen`] — persistencia sled de agentes y conversaciones.
//!
//! Las acciones de control nunca se auto-ejecutan: el agente las **propone**
//! como [`AccionPropuesta`] validada por [`atipay`], y el usuario aprueba —
//! exactamente la doctrina de `:hacé`.
//!
//! [`ChatRequest`]: pluma_llm_core::ChatRequest

mod agente;
mod almacen;
mod conversacion;
pub mod motor;

pub use agente::{Agente, Capacidades};
pub use almacen::{Almacen, AlmacenError};
pub use conversacion::{
    AccionPropuesta, BloqueSalida, Conversacion, EstadoAccion, Peligro, Rol, Turno, Uso,
};
