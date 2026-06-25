//! Protocolo de la wire entre los productores/lectores y el daemon willay.
//!
//! Son sólo **datos** (postcard sobre el socket Unix) — el framing y el io viven
//! en `willay-emit` (std). Acá, por estar en el core `no_std`, quedan
//! wawa-compatibles como el resto del esquema. Ver `shared/willay/SDD.md` §1.1.

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::{Clase, Evento};

/// Lo que un cliente le pide al daemon. Una conexión puede mandar varias.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Solicitud {
    /// Agregá este evento al índice (escritura). Responde [`Respuesta::Ok`].
    Emitir(Evento),
    /// Dame los `n` eventos más recientes (descendente).
    Recientes(u32),
    /// Dame los `n` más recientes de esta clase.
    PorClase(Clase, u32),
    /// Búsqueda literal: los `n` más recientes que matcheen la aguja.
    Buscar(String, u32),
    /// Vaciá el índice entero. Responde [`Respuesta::Ok`].
    Limpiar,
    /// Suscribite a los cambios: el daemon no responde de inmediato, sino que
    /// empuja un [`Respuesta::Cambio`] cada vez que el índice cambia (un append),
    /// hasta que la conexión caiga. La conexión queda dedicada a esto.
    Suscribir,
}

/// Lo que el daemon responde a una [`Solicitud`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Respuesta {
    /// La escritura se aplicó.
    Ok,
    /// El resultado de una consulta.
    Eventos(Vec<Evento>),
    /// Algo falló del lado del daemon (mensaje para log, no para el usuario).
    Error(String),
    /// Notificación push (sólo en una conexión suscrita): el índice cambió.
    Cambio,
}
