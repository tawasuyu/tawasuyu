//! Handshake Noise_XK (X25519 + ChaCha20-Poly1305 + BLAKE2s).
//!
//! Roles:
//!
//! - **Client** ([`client_handshake`]): conoce de antemano la pubkey
//!   del server (igual que `known_hosts` de SSH). Envía 3 mensajes
//!   Noise (`e`, `es; s, ss`, vacío); al final tiene una
//!   `TransportState` lista para enviar y recibir.
//! - **Server** ([`server_handshake`]): conoce su propia keypair y
//!   recibe la pubkey del cliente DURANTE el handshake. Tras
//!   completarlo, la pubkey del cliente se extrae con
//!   `get_remote_static()` y la decisión de aceptar/rechazar pasa al
//!   caller (típicamente: validar contra `KnownPeers`).
//!
//! Diseño tipo SSH: client confía en server por pre-shared pubkey
//! (no TOFU automático; en una shell remota el TOFU se vuelve un
//! TOCTOU); server confía en client por allowlist explícita
//! (`~/.config/shuma/known_peers.txt`).

use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::channel::FramedChannel;
use crate::identity::{noise_pattern, Keypair, KeypairError, PublicKey, KEY_LEN};

/// Tope de tamaño de un mensaje de handshake — viene de Noise (65 535
/// bytes), pero los handshakes XK son <200 B. Reservamos por si en el
/// futuro añadimos prólogo con metadatos (versión protocolo, etc.).
const HANDSHAKE_FRAME_MAX: usize = 4096;

/// Cliente Noise_XK: conecta y autentica.
///
/// `expected_server` es la pubkey que el cliente espera. Si el server
/// se identifica con otra, el handshake falla con `WrongServerKey`.
/// Tras completar, devuelve un canal cifrado listo para mensajes de
/// aplicación.
pub async fn client_handshake<S>(
    mut stream: S,
    our_keypair: &Keypair,
    expected_server: PublicKey,
) -> Result<FramedChannel<S>, HandshakeError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    let pattern = noise_pattern().map_err(HandshakeError::Keypair)?;
    let mut hs = snow::Builder::new(pattern)
        .local_private_key(our_keypair.private_bytes())
        .remote_public_key(expected_server.as_bytes())
        .build_initiator()
        .map_err(HandshakeError::Snow)?;

    let mut buf = vec![0u8; HANDSHAKE_FRAME_MAX];
    // Mensaje 1 (cliente → server): -> e
    let n = hs.write_message(&[], &mut buf).map_err(HandshakeError::Snow)?;
    write_msg(&mut stream, &buf[..n]).await?;
    // Mensaje 2 (server → cliente): <- e, ee, s, es
    let resp = read_msg(&mut stream).await?;
    let _ = hs.read_message(&resp, &mut buf).map_err(HandshakeError::Snow)?;
    // Mensaje 3 (cliente → server): -> s, se
    let n = hs.write_message(&[], &mut buf).map_err(HandshakeError::Snow)?;
    write_msg(&mut stream, &buf[..n]).await?;

    // En XK el cliente conoce la pubkey del server desde el principio,
    // así que `get_remote_static` después del handshake sólo confirma
    // lo que ya validó snow.
    let remote = hs
        .get_remote_static()
        .ok_or(HandshakeError::NoRemoteStatic)?;
    if remote != expected_server.as_bytes() {
        return Err(HandshakeError::WrongServerKey);
    }
    let transport = hs.into_transport_mode().map_err(HandshakeError::Snow)?;
    Ok(FramedChannel::new(stream, transport))
}

/// Servidor Noise_XK: acepta una conexión y autentica el cliente.
///
/// Devuelve `(channel, peer_pubkey)` — el caller decide si la
/// `peer_pubkey` está autorizada (típicamente: `KnownPeers::contains`).
pub async fn server_handshake<S>(
    mut stream: S,
    our_keypair: &Keypair,
) -> Result<(FramedChannel<S>, PublicKey), HandshakeError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    let pattern = noise_pattern().map_err(HandshakeError::Keypair)?;
    let mut hs = snow::Builder::new(pattern)
        .local_private_key(our_keypair.private_bytes())
        .build_responder()
        .map_err(HandshakeError::Snow)?;

    let mut buf = vec![0u8; HANDSHAKE_FRAME_MAX];
    // Mensaje 1 (cliente → server)
    let m1 = read_msg(&mut stream).await?;
    let _ = hs.read_message(&m1, &mut buf).map_err(HandshakeError::Snow)?;
    // Mensaje 2 (server → cliente)
    let n = hs.write_message(&[], &mut buf).map_err(HandshakeError::Snow)?;
    write_msg(&mut stream, &buf[..n]).await?;
    // Mensaje 3 (cliente → server)
    let m3 = read_msg(&mut stream).await?;
    let _ = hs.read_message(&m3, &mut buf).map_err(HandshakeError::Snow)?;

    let remote_bytes = hs
        .get_remote_static()
        .ok_or(HandshakeError::NoRemoteStatic)?;
    let arr: [u8; KEY_LEN] = remote_bytes
        .try_into()
        .map_err(|_| HandshakeError::BadPubkeyLength)?;
    let peer = PublicKey(arr);
    let transport = hs.into_transport_mode().map_err(HandshakeError::Snow)?;
    Ok((FramedChannel::new(stream, transport), peer))
}

/// Frame de handshake con length-prefix u32 BE (igual layout que el
/// canal post-handshake, simétrico — facilita lectura del wire en
/// debug y tcpdump).
async fn write_msg<S>(stream: &mut S, msg: &[u8]) -> Result<(), HandshakeError>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    let len = (msg.len() as u32).to_be_bytes();
    stream.write_all(&len).await.map_err(HandshakeError::Io)?;
    stream.write_all(msg).await.map_err(HandshakeError::Io)?;
    stream.flush().await.map_err(HandshakeError::Io)?;
    Ok(())
}

async fn read_msg<S>(stream: &mut S) -> Result<Vec<u8>, HandshakeError>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                HandshakeError::Closed
            } else {
                HandshakeError::Io(e)
            }
        })?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > HANDSHAKE_FRAME_MAX {
        return Err(HandshakeError::Oversize(len));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await.map_err(HandshakeError::Io)?;
    Ok(buf)
}

/// Errores del handshake.
#[derive(Debug, Error)]
pub enum HandshakeError {
    #[error("identidad: {0}")]
    Keypair(KeypairError),
    #[error("snow: {0}")]
    Snow(snow::Error),
    #[error("io: {0}")]
    Io(std::io::Error),
    #[error("conexión cerrada antes de completar el handshake")]
    Closed,
    #[error("frame de handshake oversize: {0} bytes")]
    Oversize(usize),
    #[error("server presentó una pubkey distinta a la esperada")]
    WrongServerKey,
    #[error("snow no expuso la pubkey remota")]
    NoRemoteStatic,
    #[error("la pubkey remota no tiene longitud 32")]
    BadPubkeyLength,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Keypair;

    /// Handshake completo sobre un par de UnixStream — verifica que
    /// ambas partes pueden enviarse mensajes cifrados y descifrarlos
    /// en ambos sentidos.
    #[tokio::test]
    async fn xk_handshake_round_trip() {
        let server_kp = Keypair::generate().unwrap();
        let client_kp = Keypair::generate().unwrap();
        let server_pub = server_kp.public();

        let (server_io, client_io) = tokio::net::UnixStream::pair().unwrap();
        let server_kp2 = server_kp.clone();
        let server_task = tokio::spawn(async move {
            let (mut ch, peer) = server_handshake(server_io, &server_kp2).await.unwrap();
            // El primer mensaje cifrado del cliente debe llegar.
            let m = ch.recv().await.unwrap();
            assert_eq!(m, b"hola server");
            // Respondemos cifrado.
            ch.send(b"hola cliente").await.unwrap();
            peer
        });
        let mut ch = client_handshake(client_io, &client_kp, server_pub).await.unwrap();
        ch.send(b"hola server").await.unwrap();
        let r = ch.recv().await.unwrap();
        assert_eq!(r, b"hola cliente");

        let peer_seen_by_server = server_task.await.unwrap();
        assert_eq!(
            peer_seen_by_server,
            client_kp.public(),
            "server vio la pubkey real del cliente"
        );
    }

    /// Si el cliente espera una pubkey distinta a la que el server
    /// presenta, el handshake debe fallar — protección MITM.
    #[tokio::test]
    async fn wrong_server_key_aborts_handshake() {
        let real_server = Keypair::generate().unwrap();
        let attacker = Keypair::generate().unwrap();
        let client = Keypair::generate().unwrap();

        let (server_io, client_io) = tokio::net::UnixStream::pair().unwrap();
        let attacker_clone = attacker.clone();
        let server_task = tokio::spawn(async move {
            // El "atacante" es el que responde con su propia keypair.
            server_handshake(server_io, &attacker_clone).await
        });
        // El cliente espera la pubkey del server real.
        let res = client_handshake(client_io, &client, real_server.public()).await;
        assert!(
            res.is_err(),
            "el handshake con pubkey equivocada debe fallar"
        );
        // No esperamos del server_task — su error/éxito no es lo que
        // estamos validando.
        let _ = server_task.await;
    }

    /// Múltiples mensajes en ambos sentidos — counter de Noise + framing
    /// se mantienen sincronizados a lo largo de la sesión.
    #[tokio::test]
    async fn multiple_messages_keep_counters_in_sync() {
        let server_kp = Keypair::generate().unwrap();
        let client_kp = Keypair::generate().unwrap();
        let server_pub = server_kp.public();

        let (server_io, client_io) = tokio::net::UnixStream::pair().unwrap();
        let server_kp2 = server_kp.clone();
        let server_task = tokio::spawn(async move {
            let (mut ch, _) = server_handshake(server_io, &server_kp2).await.unwrap();
            for i in 0..20 {
                let msg = ch.recv().await.unwrap();
                assert_eq!(msg, format!("ping {i}").as_bytes());
                ch.send(format!("pong {i}").as_bytes()).await.unwrap();
            }
        });
        let mut ch = client_handshake(client_io, &client_kp, server_pub).await.unwrap();
        for i in 0..20 {
            ch.send(format!("ping {i}").as_bytes()).await.unwrap();
            let pong = ch.recv().await.unwrap();
            assert_eq!(pong, format!("pong {i}").as_bytes());
        }
        server_task.await.unwrap();
    }

    /// Round-trip de un valor serializado con postcard sobre el canal
    /// cifrado — demuestra que daemon y remote-exec van a poder enviar
    /// sus Request/Response sin tocar el bucle de framing.
    #[tokio::test]
    async fn postcard_round_trips_over_encrypted_channel() {
        use serde::{Deserialize, Serialize};
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
        struct Msg {
            label: String,
            n: u64,
        }

        let server_kp = Keypair::generate().unwrap();
        let client_kp = Keypair::generate().unwrap();
        let server_pub = server_kp.public();
        let (server_io, client_io) = tokio::net::UnixStream::pair().unwrap();
        let server_kp2 = server_kp.clone();
        let server_task = tokio::spawn(async move {
            let (mut ch, _) = server_handshake(server_io, &server_kp2).await.unwrap();
            let m: Msg = ch.recv_postcard().await.unwrap();
            assert_eq!(m, Msg { label: "hola".into(), n: 42 });
            ch.send_postcard(&Msg { label: "ok".into(), n: 43 }).await.unwrap();
        });
        let mut ch = client_handshake(client_io, &client_kp, server_pub).await.unwrap();
        ch.send_postcard(&Msg { label: "hola".into(), n: 42 }).await.unwrap();
        let r: Msg = ch.recv_postcard().await.unwrap();
        assert_eq!(r, Msg { label: "ok".into(), n: 43 });
        server_task.await.unwrap();
    }

    /// Si el cierre del peer corta la conexión mitad de stream, `recv`
    /// devuelve `Closed`, no se cuelga.
    #[tokio::test]
    async fn peer_close_translates_to_closed_error() {
        let server_kp = Keypair::generate().unwrap();
        let client_kp = Keypair::generate().unwrap();
        let server_pub = server_kp.public();
        let (server_io, client_io) = tokio::net::UnixStream::pair().unwrap();
        let server_kp2 = server_kp.clone();
        let server_task = tokio::spawn(async move {
            let (ch, _) = server_handshake(server_io, &server_kp2).await.unwrap();
            // Server cierra el canal inmediatamente.
            drop(ch);
        });
        let mut ch = client_handshake(client_io, &client_kp, server_pub).await.unwrap();
        let r = ch.recv().await;
        assert!(matches!(r, Err(crate::channel::FrameError::Closed)));
        server_task.await.unwrap();
    }
}
