//! `shuma-link` — capa de transporte autenticado del daemon shuma.
//!
//! Reemplaza el "Unix socket + SO_PEERCRED" (válido sólo localmente) por
//! un canal **autenticado y cifrado** sobre cualquier transporte
//! `AsyncRead + AsyncWrite` (TCP, Unix socket sobre el escritorio
//! compartido, lo que toque). Es la palanca que convierte
//! `shuma-remote-exec` en un cliente de un daemon **remoto**, sin tener
//! que apoyarse en SSH externo ni en mosh.
//!
//! ## Patrón Noise_XK
//!
//! - **X**: cliente envía su pubkey estática durante el handshake
//!   (anonymity property: la pubkey va cifrada).
//! - **K**: cliente conoce la pubkey del servidor *de antemano* (igual
//!   que el `known_hosts` de SSH). Cualquier man-in-the-middle falla
//!   el handshake.
//!
//! Resultado: mutual auth, forward secrecy, replay-protection y
//! 0-RTT en el primer mensaje del cliente tras el handshake.
//!
//! ## Componentes
//!
//! - [`identity::Keypair`] — par X25519 persistente del nodo
//!   (`~/.config/shuma/keys/identity.x25519`). Se genera al primer
//!   arranque y se reusa después.
//! - [`peers::KnownPeers`] — set de pubkeys confiables (allowlist
//!   estilo `~/.ssh/authorized_keys`, en
//!   `~/.config/shuma/known_peers.txt`).
//! - [`handshake::client_handshake`] / [`handshake::server_handshake`]
//!   — establecen el canal Noise sobre un `AsyncRead+AsyncWrite`.
//! - [`channel::FramedChannel`] — wrapper post-handshake que envía y
//!   recibe payloads opacos (bytes), con framing + cifrado/MAC
//!   ChaCha20-Poly1305 + counter de Noise.
//!
//! Las helpers de framing del protocolo (`shuma_protocol::read_frame`,
//! `write_frame`) NO se reutilizan aquí: el canal cifrado tiene su
//! propio length-prefix interior por mensaje Noise (max 65 535 B).
//! Para payloads más grandes, `FramedChannel` los partiría — hoy el
//! protocolo de shuma cabe muy holgado en un solo Noise frame.

#![forbid(unsafe_code)]

pub mod channel;
pub mod handshake;
pub mod identity;
pub mod peers;

pub use channel::{FramedChannel, FramedReader, FramedWriter};
pub use handshake::{client_handshake, server_handshake, HandshakeError};
pub use identity::{Keypair, KeypairError, PublicKey};
pub use peers::KnownPeers;
