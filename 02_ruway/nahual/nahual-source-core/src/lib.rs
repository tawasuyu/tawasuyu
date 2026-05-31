//! `nahual-source-core` — la **fuente navegable** agnóstica del front
//! universal (Brahman, Fase 3).
//!
//! El norte de `nahual-shell` es abrir, con la misma UI, "una raíz de minga,
//! un objeto de wawa, una Mónada de nouser o un archivo POSIX" (ver
//! `/BRAHMAN.md`, sección "proliferación de exploradores"). Cada uno de esos
//! mundos tiene su propio modelo de árbol —`std::fs`, un DAG BLAKE3, clusters
//! semánticos— pero todos comparten la misma forma mínima: una **raíz**, una
//! manera de **listar hijos** de un contenedor, y una manera de **leer los
//! bytes** de una hoja. Eso es [`Source`].
//!
//! La proliferación de exploradores NO se cura fusionando sus datos
//! (incompatibles), sino poniéndolos detrás de esta interfaz común: el shell
//! deja de saber de `PathBuf` y pasa a navegar `dyn Source`. Hoy hay tres
//! adapters reales:
//!
//! - [`posix::PosixSource`] — el filesystem POSIX vivo (lo que el shell ya
//!   hacía, ahora detrás del trait).
//! - [`wawa::WawaImgSource`] — los objetos content-addressed de una imagen
//!   wawa `.img`, navegando el DAG por hash. Puro local, sin red ni daemon.
//! - `nouser::NouserSource` (feature `nouser`) — las Mónadas semánticas de
//!   `chasqui-core`: clusters de archivos, un árbol que NO existe en disco.
//!
//! Cada uno es una *forma de árbol* distinta (jerarquía física, DAG de
//! contenido, clusters semánticos) y aun así caben en el mismo trait — esa es
//! la prueba de que la abstracción aguanta. Agregar minga como cuarta fuente
//! = un `impl Source` más, sin tocar el shell.

#![forbid(unsafe_code)]

pub mod navigator;
pub mod posix;
pub mod wawa;
#[cfg(feature = "nouser")]
pub mod nouser;

pub use navigator::{Navigator, Opened};
pub use posix::PosixSource;
pub use wawa::WawaImgSource;
#[cfg(feature = "nouser")]
pub use nouser::NouserSource;

/// Identidad opaca de un nodo DENTRO de su fuente. El shell la trata como
/// caja negra (la guarda para volver a pedir hijos o leer), salvo para
/// derivar la identidad de contenido al despachar el visor.
///
/// La codificación es decisión de cada [`Source`]: POSIX usa la ruta
/// absoluta; wawa usa el hash en hex. No mezclar ids entre fuentes.
pub type NodeId = String;

/// Un nodo del árbol de una [`Source`]: lo mínimo que la UI necesita para
/// pintar una fila y decidir si se puede descender.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Identidad estable dentro de la fuente (ver [`NodeId`]).
    pub id: NodeId,
    /// Nombre legible para la fila.
    pub name: String,
    /// `true` si se puede descender (tiene hijos / es directorio); `false`
    /// si es una hoja que se abre en el visor.
    pub is_container: bool,
}

impl Node {
    /// Atajo para construir un nodo.
    pub fn new(id: impl Into<NodeId>, name: impl Into<String>, is_container: bool) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            is_container,
        }
    }
}

/// Una fuente navegable: el contrato agnóstico que el front universal
/// consume. Object-safe a propósito — el shell guarda `Box<dyn Source>` y
/// puede apilar fuentes (descender de un `.img` POSIX a su DAG wawa).
///
/// `Send + Sync` para poder escanear en un worker (`Handle::spawn`) sin
/// devolver el árbol entero por el canal de mensajes.
pub trait Source: Send + Sync {
    /// Nombre humano de la fuente — para breadcrumb / título del panel.
    fn label(&self) -> String;

    /// El nodo raíz desde el que se empieza a navegar.
    fn root(&self) -> Node;

    /// Hijos directos de un contenedor, en orden ya presentable. Error si el
    /// id no existe o no es un contenedor.
    fn children(&self, id: &NodeId) -> std::io::Result<Vec<Node>>;

    /// Bytes de una hoja — para discernir (`shuma-discern`) y visualizar.
    /// Error si el id no existe o no tiene contenido leíble.
    fn read(&self, id: &NodeId) -> std::io::Result<Vec<u8>>;
}

/// Codifica 32 bytes a hex en minúscula (64 chars). Compartido por el
/// adapter wawa y reusable por cualquier fuente content-addressed futura.
pub(crate) fn to_hex(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Decodifica 64 chars hex a 32 bytes. `None` si la longitud o los dígitos
/// son inválidos.
pub(crate) fn from_hex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    let bytes = s.as_bytes();
    for (i, slot) in out.iter_mut().enumerate() {
        let hi = (bytes[2 * i] as char).to_digit(16)?;
        let lo = (bytes[2 * i + 1] as char).to_digit(16)?;
        *slot = (hi * 16 + lo) as u8;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let h = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        ];
        let hex = to_hex(&h);
        assert_eq!(hex.len(), 64);
        assert_eq!(from_hex(&hex), Some(h));
    }

    #[test]
    fn from_hex_rechaza_basura() {
        assert_eq!(from_hex("corto"), None);
        assert_eq!(from_hex(&"z".repeat(64)), None);
    }
}
