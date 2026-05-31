// =============================================================================
//  renaser :: format — el format del grafo de objetos en disco
// -----------------------------------------------------------------------------
//  Hasta la Fase 7a, el format del grafo de objetos —el superbloque, los
//  registros del log, el manifiesto— vivia disperso entre `kernel/almacen.rs`
//  y `kernel/manifiesto.rs`. Lo conocia solo el kernel.
//
//  La Fase 7b se lo entrega tambien a `boot`: el constructor de imagen de
//  ANFITRION debe sembrar el disco con el grafo ya poblado —los objetos de
//  bytecode y el Manifiesto de Genesis— para que el kernel jamas vuelva a
//  empotrar una sola app. Para ello, kernel y boot han de hablar EXACTAMENTE
//  el mismo format: la misma serializacion, el mismo hash, el mismo trazado
//  de registros en el log.
//
//  Esta crate es esa unica verdad. Es un nucleo `#![no_std]` —el kernel
//  bare-metal la enlaza— y, por ser no_std, el anfitrion `boot` la compila sin
//  friccion. Define los tipos del grafo, su (de)serializacion `postcard`, la
//  funcion hash BLAKE3 que da identidad a cada objeto y el trazado de un
//  registro en el log. Ni kernel ni boot vuelven a definir nada de esto.
// =============================================================================

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

// --- Split temático del format (todo pub; API plana preservada con `pub use
// <mod>::*`). Cada módulo abre con `use super::*` para ver los otros tipos +
// las imports de alloc/serde del root. Sigue siendo `#![no_std]`. ---
mod cable;
mod constantes;
mod firma;
mod grafo;
mod tipos;
#[cfg(test)]
mod pruebas;

pub use cable::*;
pub use constantes::*;
pub use firma::*;
pub use grafo::*;
pub use tipos::*;
