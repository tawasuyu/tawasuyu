// =============================================================================
//  ayni :: ayni-app — el núcleo de aplicación compartido (glue)
// -----------------------------------------------------------------------------
//  Las UIs son frontends intercambiables sobre los `*-core` agnósticos; este
//  crate es la capa JUSTO debajo de la UI: el `Nucleo` que sostiene el grafo,
//  el transporte (TCP o minga), la persistencia, el cifrado, los adjuntos y la
//  confianza, y el `Enlace` que unifica los dos transportes. El CLI y la GUI
//  Llimphi comparten esto y sólo difieren en cómo lo pintan.
// =============================================================================

mod enlace;
mod nucleo;

pub use enlace::{Enlace, Tipo};
pub use nucleo::{hex_corto, Nucleo};

// Re-exports de conveniencia para que una UI dependa sólo de `ayni-app`.
pub use ayni_core::{AgoraId, Carga, Conversacion, Hash, MensajeNodo, Membresia};
pub use ayni_crypto::Identidad;
pub use ayni_sync::{EventoRed, PeerId, Transporte};
