//! paloma-rail — el **rail soberano** del correo (Eje 3.B).
//!
//! Correo suite-a-suite **sin SMTP**: en vez de empujar el mensaje a un servidor
//! que lo enruta por DNS/MX, paloma lo entrega persona-a-persona sobre un
//! transporte P2P (chasqui/ayni), direccionado por **identidad `agora`** (la
//! clave pública del destinatario), no por una dirección de dominio ajeno.
//!
//! ## La unidad: [`RailEnvelope`]
//!
//! Un sobre lleva el `Message` nativo serializado (postcard) + las identidades
//! emisor/receptor + una firma Ed25519 sobre todo. La firma del sobre **es** la
//! autenticación del rail: quien lo abre verifica que vino de la identidad
//! declarada y que nadie lo tocó. No hay "From spoofing" posible — la dirección
//! es la clave.
//!
//! - [`seal`] — el emisor sella un `Message` para una identidad destino.
//! - [`open`] — el receptor verifica y recupera el `Message` (marcado
//!   [`SignatureStatus::Verified`], porque el sobre firmado ya lo autentica).
//!
//! ## El transporte: [`RailTransport`]
//!
//! Trait mínimo (enviar un sobre a una identidad). La implementación concreta
//! la pone el anfitrión sobre `ayni-sync::Transporte` / chasqui; este crate trae
//! [`MockTransport`] en memoria para tests y para correr el rail de punta a
//! punta sin red. La recepción es push: el anfitrión ingiere los sobres
//! entrantes en una [`RailInbox`].

use agora_core::Keypair;
use paloma_core::{Message, SignatureStatus};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Identidad de un peer del rail: su clave pública Ed25519 (32 bytes). Es la
/// "dirección" — sustituye al `usuario@dominio` del correo clásico.
pub type RailId = [u8; 32];

/// Sufijo de dominio de las direcciones del rail. Una dirección "Suyu" es la
/// clave pública en hex seguida de `@rail.suyu` — encaja en los campos de
/// destinatario del compositor (el dominio lleva punto, como exige el validador
/// de direcciones) y se distingue de un correo normal.
pub const RAIL_DOMAIN: &str = "rail.suyu";

/// Formatea una identidad como dirección del rail: `<hex64>@suyu`.
pub fn rail_address(id: &RailId) -> String {
    let mut s = String::with_capacity(64 + 5);
    for b in id {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s.push('@');
    s.push_str(RAIL_DOMAIN);
    s
}

/// Parsea una dirección del rail (`<hex64>@suyu`) a su identidad. `None` si no
/// tiene el dominio `@suyu` o el hex no son 32 bytes válidos — así el enrutador
/// distingue un destinatario del rail de un correo SMTP normal.
pub fn parse_rail_address(addr: &str) -> Option<RailId> {
    let (local, domain) = addr.trim().rsplit_once('@')?;
    if !domain.eq_ignore_ascii_case(RAIL_DOMAIN) || local.len() != 64 {
        return None;
    }
    let mut id = [0u8; 32];
    let bytes = local.as_bytes();
    for (i, slot) in id.iter_mut().enumerate() {
        let hi = (bytes[i * 2] as char).to_digit(16)?;
        let lo = (bytes[i * 2 + 1] as char).to_digit(16)?;
        *slot = (hi * 16 + lo) as u8;
    }
    Some(id)
}

/// Un sobre del rail: el `Message` serializado + emisor/receptor + firma.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RailEnvelope {
    /// Identidad del emisor (su clave pública).
    pub from: RailId,
    /// Identidad del destinatario.
    pub to: RailId,
    /// `Message` nativo serializado con postcard.
    pub payload: Vec<u8>,
    /// Firma Ed25519 del emisor sobre [`Self::signed_bytes`] (64 bytes).
    pub sig: Vec<u8>,
}

impl RailEnvelope {
    /// Bytes que la firma cubre: versión + from + to + payload. Atar emisor y
    /// destinatario impide reusar un sobre para otro receptor.
    fn signed_bytes(from: &RailId, to: &RailId, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(13 + 64 + payload.len());
        out.extend_from_slice(b"paloma-rail-v1");
        out.extend_from_slice(from);
        out.extend_from_slice(to);
        out.extend_from_slice(payload);
        out
    }

    /// Serializa el sobre para el cable (postcard).
    pub fn to_bytes(&self) -> Result<Vec<u8>, RailError> {
        postcard::to_allocvec(self).map_err(|e| RailError::Codec(e.to_string()))
    }

    /// Reconstruye un sobre desde bytes del cable.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RailError> {
        postcard::from_bytes(bytes).map_err(|e| RailError::Codec(e.to_string()))
    }
}

/// Errores del rail.
#[derive(Debug, Error)]
pub enum RailError {
    #[error("códec del sobre: {0}")]
    Codec(String),
    /// La firma del sobre no valida (manipulado o identidad falsa).
    #[error("firma del sobre inválida")]
    BadSignature,
    /// El sobre venía dirigido a otra identidad.
    #[error("sobre dirigido a otra identidad")]
    WrongRecipient,
    /// El transporte no pudo entregar (peer desconocido, red caída…).
    #[error("transporte: {0}")]
    Transport(String),
}

/// Sella un `Message` para `to`: lo serializa, firma con `keypair` y arma el
/// sobre. El emisor queda atado a su identidad por la firma.
pub fn seal(keypair: &Keypair, to: RailId, message: &Message) -> Result<RailEnvelope, RailError> {
    let payload = postcard::to_allocvec(message).map_err(|e| RailError::Codec(e.to_string()))?;
    let from = keypair.public_key();
    let signed = RailEnvelope::signed_bytes(&from, &to, &payload);
    let sig = keypair.sign(&signed);
    Ok(RailEnvelope {
        from,
        to,
        payload,
        sig: sig.to_vec(),
    })
}

/// Abre un sobre dirigido a `me`: verifica la firma contra la identidad emisora
/// y recupera el `Message` (marcado `Verified`). Falla si la firma no cierra, si
/// el sobre era para otra identidad, o si el payload no decodifica.
pub fn open(env: &RailEnvelope, me: RailId) -> Result<Message, RailError> {
    if env.to != me {
        return Err(RailError::WrongRecipient);
    }
    let sig: [u8; 64] = env.sig.as_slice().try_into().map_err(|_| RailError::BadSignature)?;
    let signed = RailEnvelope::signed_bytes(&env.from, &env.to, &env.payload);
    if paloma_sign::verify(&signed, &env.from, &sig) != SignatureStatus::Verified {
        return Err(RailError::BadSignature);
    }
    let mut message: Message =
        postcard::from_bytes(&env.payload).map_err(|e| RailError::Codec(e.to_string()))?;
    // El sobre firmado autentica al emisor: el mensaje llega verificado.
    message.signature = SignatureStatus::Verified;
    Ok(message)
}

/// El transporte del rail: enviar un sobre a una identidad. La recepción llega
/// por fuera (push): el anfitrión ingiere en una [`RailInbox`]. Lo implementa el
/// anfitrión sobre `ayni-sync::Transporte`/chasqui; [`MockTransport`] sirve en
/// memoria para tests y para el rail sin red.
pub trait RailTransport: Send {
    /// Entrega `envelope` a la identidad `to`.
    fn send(&self, to: RailId, envelope: &RailEnvelope) -> Result<(), RailError>;
}

/// Bandeja del rail: los `Message` recibidos por P2P, ya verificados. El
/// anfitrión la expone como un buzón (p. ej. "Suyu") en la UI.
#[derive(Default)]
pub struct RailInbox {
    me: RailId,
    received: Vec<Message>,
}

impl RailInbox {
    /// Bandeja para la identidad `me` (sólo acepta sobres dirigidos a ella).
    pub fn new(me: RailId) -> Self {
        Self { me, received: Vec::new() }
    }

    /// Ingiere un sobre entrante: lo abre/verifica y, si es válido y para mí, lo
    /// guarda. Devuelve el `Message` recibido o el error (sobre ajeno/roto).
    pub fn ingest(&mut self, env: &RailEnvelope) -> Result<&Message, RailError> {
        let msg = open(env, self.me)?;
        self.received.push(msg);
        Ok(self.received.last().unwrap())
    }

    /// Los mensajes recibidos, más nuevos al final (orden de llegada).
    pub fn messages(&self) -> &[Message] {
        &self.received
    }

    pub fn len(&self) -> usize {
        self.received.len()
    }

    pub fn is_empty(&self) -> bool {
        self.received.is_empty()
    }
}

/// Transporte en memoria: encola los sobres por identidad destino. Para tests y
/// para correr el rail de punta a punta sin red (un proceso, dos identidades).
#[derive(Default)]
pub struct MockTransport {
    /// Cola por destinatario: `to -> [sobres]`.
    queues: std::sync::Mutex<std::collections::HashMap<RailId, Vec<RailEnvelope>>>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drena los sobres encolados para `to` (los que aún no se entregaron).
    pub fn drain(&self, to: RailId) -> Vec<RailEnvelope> {
        self.queues.lock().unwrap().remove(&to).unwrap_or_default()
    }
}

impl RailTransport for MockTransport {
    fn send(&self, to: RailId, envelope: &RailEnvelope) -> Result<(), RailError> {
        self.queues.lock().unwrap().entry(to).or_default().push(envelope.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paloma_core::{Address, Flags, MessageId};

    fn mensaje(subject: &str, body: &str) -> Message {
        Message {
            id: MessageId("<x@suyu>".into()),
            from: Address::named("Yo", "yo@suyu.net"),
            to: vec![Address::named("Ana", "ana@suyu.net")],
            cc: vec![],
            bcc: vec![],
            subject: subject.into(),
            date: 0,
            in_reply_to: None,
            references: vec![],
            body_text: body.into(),
            body_html: None,
            flags: Flags::default(),
            signature: SignatureStatus::Unsigned,
            mailbox: "Borradores".into(),
            cuerpos: Vec::new(),
        }
    }

    #[test]
    fn sellar_y_abrir_roundtrip() {
        let ana = Keypair::from_seed([1; 32]);
        let bob = Keypair::from_seed([2; 32]);
        let msg = mensaje("hola", "nos vemos el viernes");

        let env = seal(&ana, bob.public_key(), &msg).unwrap();
        let recibido = open(&env, bob.public_key()).unwrap();

        assert_eq!(recibido.subject, "hola");
        assert_eq!(recibido.body_text, "nos vemos el viernes");
        // El sobre firmado autentica al emisor → llega verificado.
        assert_eq!(recibido.signature, SignatureStatus::Verified);
    }

    #[test]
    fn sobre_para_otra_identidad_se_rechaza() {
        let ana = Keypair::from_seed([1; 32]);
        let bob = Keypair::from_seed([2; 32]);
        let carla = Keypair::from_seed([3; 32]);
        let env = seal(&ana, bob.public_key(), &mensaje("x", "y")).unwrap();
        assert!(matches!(open(&env, carla.public_key()), Err(RailError::WrongRecipient)));
    }

    #[test]
    fn payload_manipulado_invalida_la_firma() {
        let ana = Keypair::from_seed([1; 32]);
        let bob = Keypair::from_seed([2; 32]);
        let mut env = seal(&ana, bob.public_key(), &mensaje("x", "original")).unwrap();
        // Manipular el payload tras sellar.
        if let Some(b) = env.payload.last_mut() {
            *b ^= 0xff;
        }
        assert!(matches!(open(&env, bob.public_key()), Err(RailError::BadSignature)));
    }

    #[test]
    fn sobre_resellado_para_otro_no_cuela() {
        // Un sobre A→B no se puede redirigir a C cambiando `to`: la firma ata to.
        let ana = Keypair::from_seed([1; 32]);
        let bob = Keypair::from_seed([2; 32]);
        let carla = Keypair::from_seed([3; 32]);
        let mut env = seal(&ana, bob.public_key(), &mensaje("x", "y")).unwrap();
        env.to = carla.public_key();
        assert!(matches!(open(&env, carla.public_key()), Err(RailError::BadSignature)));
    }

    #[test]
    fn direccion_del_rail_roundtrip() {
        let id = Keypair::from_seed([5; 32]).public_key();
        let addr = rail_address(&id);
        assert!(addr.ends_with("@rail.suyu"));
        assert_eq!(parse_rail_address(&addr), Some(id));
        // Un correo normal no es una dirección del rail.
        assert_eq!(parse_rail_address("ana@gmail.com"), None);
        assert_eq!(parse_rail_address("corto@rail.suyu"), None);
    }

    #[test]
    fn bytes_roundtrip_del_cable() {
        let ana = Keypair::from_seed([1; 32]);
        let bob = Keypair::from_seed([2; 32]);
        let env = seal(&ana, bob.public_key(), &mensaje("asunto", "cuerpo")).unwrap();
        let bytes = env.to_bytes().unwrap();
        let env2 = RailEnvelope::from_bytes(&bytes).unwrap();
        assert_eq!(env, env2);
    }

    #[test]
    fn rail_de_punta_a_punta_sobre_transporte() {
        // El rail completo sin red: Ana sella → transporte → bandeja de Bob.
        let ana = Keypair::from_seed([1; 32]);
        let bob = Keypair::from_seed([2; 32]);
        let transporte = MockTransport::new();
        let mut bandeja_bob = RailInbox::new(bob.public_key());

        let env = seal(&ana, bob.public_key(), &mensaje("minga", "vení el sábado")).unwrap();
        transporte.send(bob.public_key(), &env).unwrap();

        // Bob recibe (poll del transporte) e ingiere.
        for sobre in transporte.drain(bob.public_key()) {
            bandeja_bob.ingest(&sobre).unwrap();
        }
        assert_eq!(bandeja_bob.len(), 1);
        assert_eq!(bandeja_bob.messages()[0].subject, "minga");
        assert_eq!(bandeja_bob.messages()[0].signature, SignatureStatus::Verified);
    }
}
