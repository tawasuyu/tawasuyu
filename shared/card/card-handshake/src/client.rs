//! Cliente de handshake. Conecta a un Unix socket y mantiene la sesión.

use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

use card_core::{Card, WitInterface, CARD_SCHEMA_VERSION};
use card_net::Keypair;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;

use crate::codec::{read_frame, write_frame};
use crate::identity::SessionCert;
use crate::messages::{Farewell, Frame, HandshakeError, Hello, HelloAck, MatchEvent, Ping, SessionId};
use crate::signature::{sign_hello, SignatureError};

/// Errores del cliente.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),

    /// El servidor respondió con un error explícito.
    #[error("servidor: {0}")]
    Server(#[source] HandshakeError),

    /// El servidor envió un frame que no esperábamos en este punto del protocolo.
    #[error("frame inesperado: {got}")]
    UnexpectedFrame { got: &'static str },

    /// La Card que el cliente intentó enviar no pasa su propia validación.
    #[error("card inválida pre-envío: {0}")]
    InvalidCard(String),

    /// Firma del Hello falló al construirse (rara — sólo puede pasar
    /// si la keypair pasada está en un estado inválido).
    #[error("firma del Hello falló: {0}")]
    Signature(#[from] SignatureError),
}

/// Cliente conectado y autenticado. Tras `connect` ya completó el handshake
/// y tiene su `SessionId`. Los `MatchEvent` recibidos durante operaciones
/// request/response se buferean en `pending_events` y se obtienen vía
/// [`Client::take_event`] o [`Client::await_event`].
///
/// Genérico sobre el transport (`AsyncRead + AsyncWrite + Unpin + Send`):
/// funciona indistintamente sobre `UnixStream` (path local) o sobre un
/// stream libp2p wrapped con `tokio_util::compat` (path remoto, vía
/// `card_handshake::network`).
#[derive(Debug)]
pub struct Client<S = UnixStream> {
    stream: S,
    session: SessionId,
    server_info: HelloAck,
    pending_events: VecDeque<MatchEvent>,
}

impl Client<UnixStream> {
    /// Conecta como módulo agnóstico (sin WIT) sobre Unix socket.
    /// Equivalente a `connect_with(path, card, None)`.
    pub async fn connect(path: impl AsRef<Path>, card: Card) -> Result<Self, ClientError> {
        Self::connect_with(path, card, None).await
    }

    /// Conecta al socket Unix enviando Hello con la Card dada y
    /// opcionalmente una `WitInterface` ya extraída. Si `wit` es `Some`,
    /// el server registra el módulo como "consciente".
    pub async fn connect_with(
        path: impl AsRef<Path>,
        card: Card,
        wit: Option<WitInterface>,
    ) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(path).await?;
        Self::connect_with_stream(stream, card, wit).await
    }
}

impl<S> Client<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    /// Constructor genérico sobre un stream ya abierto, **sin firma**.
    /// Apto para path Unix (donde SO_PEERCRED del kernel ya autentica)
    /// o tests in-memory. Para libp2p remoto usá
    /// [`connect_with_stream_signed`](Self::connect_with_stream_signed) —
    /// el server libp2p rechaza Hello sin firma.
    pub async fn connect_with_stream(
        stream: S,
        card: Card,
        wit: Option<WitInterface>,
    ) -> Result<Self, ClientError> {
        Self::connect_inner(stream, card, wit, None, None).await
    }

    /// Igual que `connect_with_stream` pero firma el Hello con
    /// `keypair`. Usar para conexiones libp2p donde el server exige
    /// firma. La public key derivada de `keypair` debe coincidir con
    /// el `peer_id` libp2p autenticado por Noise — típicamente la
    /// keypair pasada a [`card_net::BrahmanNet::with_keypair`].
    pub async fn connect_with_stream_signed(
        stream: S,
        card: Card,
        wit: Option<WitInterface>,
        keypair: &Keypair,
    ) -> Result<Self, ClientError> {
        Self::connect_inner(stream, card, wit, Some(keypair), None).await
    }

    /// Igual que `connect_with_stream_signed` pero además adjunta un
    /// `SessionCert` que vincula la session keypair a una identity
    /// master estable. El server, al recibir el cert, evalúa la
    /// política de admisión contra el `master_peer_id` (no contra
    /// el session peer_id) — permitiendo rotar la session sin perder
    /// la identidad reconocida en allowlists remotas.
    pub async fn connect_with_stream_signed_with_cert(
        stream: S,
        card: Card,
        wit: Option<WitInterface>,
        session_keypair: &Keypair,
        identity_cert: SessionCert,
    ) -> Result<Self, ClientError> {
        Self::connect_inner(stream, card, wit, Some(session_keypair), Some(identity_cert)).await
    }

    async fn connect_inner(
        mut stream: S,
        card: Card,
        wit: Option<WitInterface>,
        keypair: Option<&Keypair>,
        identity_cert: Option<SessionCert>,
    ) -> Result<Self, ClientError> {
        card.validate()
            .map_err(|e| ClientError::InvalidCard(e.to_string()))?;

        let wire_card = card_core::WireCard::from(card);
        let signature = match keypair {
            Some(kp) => Some(sign_hello(kp, &wire_card, &wit)?),
            None => None,
        };

        let hello = Hello {
            schema_version: CARD_SCHEMA_VERSION,
            protocol_version: card_core::PROTOCOL_VERSION.to_string(),
            card: wire_card,
            wit,
            signature,
            identity_cert,
        };
        write_frame(&mut stream, &Frame::Hello(hello)).await?;

        let frame = read_frame(&mut stream).await?;
        let ack = match frame {
            Frame::HelloAck(a) => a,
            Frame::Error(e) => return Err(ClientError::Server(e)),
            Frame::Hello(_) => return Err(ClientError::UnexpectedFrame { got: "Hello" }),
            Frame::Ping(_) => return Err(ClientError::UnexpectedFrame { got: "Ping" }),
            Frame::Pong(_) => return Err(ClientError::UnexpectedFrame { got: "Pong" }),
            Frame::Farewell(_) => return Err(ClientError::UnexpectedFrame { got: "Farewell" }),
            Frame::MatchEvent(_) => {
                return Err(ClientError::UnexpectedFrame {
                    got: "MatchEvent (pre-handshake)",
                });
            }
            Frame::ListSessions(_) => {
                return Err(ClientError::UnexpectedFrame {
                    got: "ListSessions (pre-handshake)",
                });
            }
            Frame::SessionList(_) => {
                return Err(ClientError::UnexpectedFrame {
                    got: "SessionList (pre-handshake)",
                });
            }
            Frame::ListMatches(_) => {
                return Err(ClientError::UnexpectedFrame {
                    got: "ListMatches (pre-handshake)",
                });
            }
            Frame::MatchList(_) => {
                return Err(ClientError::UnexpectedFrame {
                    got: "MatchList (pre-handshake)",
                });
            }
        };
        Ok(Self {
            stream,
            session: ack.session,
            server_info: ack,
            pending_events: VecDeque::new(),
        })
    }

    /// `SessionId` asignado por el servidor.
    pub fn session(&self) -> SessionId {
        self.session
    }

    /// Información del servidor recibida en el handshake.
    pub fn server_info(&self) -> &HelloAck {
        &self.server_info
    }

    /// Envía un Ping y devuelve el timestamp del servidor. Los frames
    /// `MatchEvent` que lleguen mezclados se buferean en `pending_events`.
    pub async fn ping(&mut self) -> Result<u64, ClientError> {
        write_frame(
            &mut self.stream,
            &Frame::Ping(Ping {
                session: self.session,
            }),
        )
        .await?;
        loop {
            match read_frame(&mut self.stream).await? {
                Frame::Pong(p) => return Ok(p.timestamp_ms),
                Frame::MatchEvent(ev) => self.pending_events.push_back(ev),
                Frame::Error(e) => return Err(ClientError::Server(e)),
                _ => return Err(ClientError::UnexpectedFrame { got: "non-pong" }),
            }
        }
    }

    /// Saca un evento pendiente del buffer, sin bloquear ni leer del wire.
    pub fn take_event(&mut self) -> Option<MatchEvent> {
        self.pending_events.pop_front()
    }

    /// Espera un `MatchEvent` con timeout. Drena primero el buffer; si
    /// está vacío, lee del wire hasta el timeout. Otros frames recibidos
    /// (Pong huérfano, Error) cortan la espera con error.
    pub async fn await_event(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<MatchEvent>, ClientError> {
        if let Some(ev) = self.pending_events.pop_front() {
            return Ok(Some(ev));
        }
        match tokio::time::timeout(timeout, read_frame(&mut self.stream)).await {
            Err(_) => Ok(None),
            Ok(Err(e)) => Err(ClientError::Io(e)),
            Ok(Ok(Frame::MatchEvent(ev))) => Ok(Some(ev)),
            Ok(Ok(Frame::Error(e))) => Err(ClientError::Server(e)),
            Ok(Ok(_)) => Err(ClientError::UnexpectedFrame {
                got: "non-event en await_event",
            }),
        }
    }

    /// Pide al servidor el listado de sesiones activas. Pensado para
    /// observadores (broker-explorer, CLIs de diagnóstico). Como
    /// `ping`, los `MatchEvent` que lleguen intercalados se bufean
    /// en `pending_events` y no rompen la respuesta.
    pub async fn list_sessions(&mut self) -> Result<crate::messages::SessionList, ClientError> {
        write_frame(
            &mut self.stream,
            &Frame::ListSessions(crate::messages::ListSessions {
                session: self.session,
            }),
        )
        .await?;
        loop {
            match read_frame(&mut self.stream).await? {
                Frame::SessionList(list) => return Ok(list),
                Frame::MatchEvent(ev) => self.pending_events.push_back(ev),
                Frame::Error(e) => return Err(ClientError::Server(e)),
                _ => {
                    return Err(ClientError::UnexpectedFrame {
                        got: "non-session-list",
                    });
                }
            }
        }
    }

    /// Pide al servidor el listado de matches actuales del broker
    /// (consumer↔producer pares con tipo y estrategia). Mismo patrón
    /// de drenado de `MatchEvent`s intermedios.
    pub async fn list_matches(&mut self) -> Result<crate::messages::MatchList, ClientError> {
        write_frame(
            &mut self.stream,
            &Frame::ListMatches(crate::messages::ListMatches {
                session: self.session,
            }),
        )
        .await?;
        loop {
            match read_frame(&mut self.stream).await? {
                Frame::MatchList(list) => return Ok(list),
                Frame::MatchEvent(ev) => self.pending_events.push_back(ev),
                Frame::Error(e) => return Err(ClientError::Server(e)),
                _ => {
                    return Err(ClientError::UnexpectedFrame {
                        got: "non-match-list",
                    });
                }
            }
        }
    }

    /// Cierre cooperativo. Consume el cliente.
    pub async fn farewell(mut self) -> Result<(), ClientError> {
        write_frame(
            &mut self.stream,
            &Frame::Farewell(Farewell {
                session: self.session,
            }),
        )
        .await?;
        Ok(())
    }
}
