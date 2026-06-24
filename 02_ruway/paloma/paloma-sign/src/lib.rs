//! paloma-sign — firma y verificación **Ed25519** del correo (Eje 3: soberanía).
//!
//! El puente entre el correo y la raíz de confianza de la suite (`agora`):
//! firma los salientes con la `Keypair` del usuario y verifica los entrantes
//! recomputando sus [`paloma_core::canonical_signing_bytes`]. Agnóstico a la red
//! y a la UI — sólo sabe de mensajes y de firmas.
//!
//! ## Qué garantiza (y qué no, todavía)
//!
//! La firma cubre remitente, destinatarios, asunto y cuerpo: cualquier
//! alteración la invalida. [`SignatureStatus::Verified`] significa **"la firma
//! cierra sobre el contenido bajo la clave declarada"** — integridad. Lo que
//! falta para "y esa clave es de verdad de quien dice ser" es un almacén de
//! confianza (mapear `pubkey ↔ contacto`), que llega con la red de confianza de
//! `agora` (ver LEEME · Pendiente). Hasta entonces, `Verified` = no manipulado.
//!
//! ## Formato en el cable
//!
//! El transporte (`paloma-net`) emite dos headers base64:
//! `X-Paloma-Pubkey` y `X-Paloma-Signature`. [`encode_signature`] /
//! [`decode_signature`] son la única fuente de ese formato.

use agora_core::{verify_signature, Keypair};
use base64::Engine;
use paloma_core::{MailSignature, Message, OutgoingMessage, SignatureStatus};

/// Firma un saliente: calcula la firma sobre sus bytes canónicos y la adjunta.
/// Idempotente — vuelve a firmar si ya tenía firma (p. ej. tras editar).
pub fn sign_outgoing(keypair: &Keypair, msg: &mut OutgoingMessage) {
    let canonical = msg.canonical_signing_bytes();
    let sig = keypair.sign(&canonical);
    msg.signature = Some(MailSignature {
        pubkey: keypair.public_key(),
        sig,
    });
}

/// Verifica una firma sobre unos bytes canónicos dados.
pub fn verify(canonical: &[u8], pubkey: &[u8; 32], sig: &[u8; 64]) -> SignatureStatus {
    match verify_signature(pubkey, canonical, sig) {
        Ok(()) => SignatureStatus::Verified,
        Err(_) => SignatureStatus::Invalid,
    }
}

/// Verifica la firma de un mensaje entrante recomputando sus bytes canónicos.
/// El `pubkey`/`sig` vienen de los headers (`paloma-net` los decodifica).
pub fn verify_message(msg: &Message, pubkey: &[u8; 32], sig: &[u8; 64]) -> SignatureStatus {
    verify(&msg.canonical_signing_bytes(), pubkey, sig)
}

/// Serializa una firma a sus dos campos base64 `(pubkey_b64, sig_b64)` para los
/// headers `X-Paloma-Pubkey` / `X-Paloma-Signature`.
pub fn encode_signature(sig: &MailSignature) -> (String, String) {
    let e = base64::engine::general_purpose::STANDARD;
    (e.encode(sig.pubkey), e.encode(sig.sig))
}

/// Reconstruye una firma desde los dos headers base64. `None` si alguno no es
/// base64 válido o no tiene el largo esperado (32 / 64 bytes).
pub fn decode_signature(pubkey_b64: &str, sig_b64: &str) -> Option<MailSignature> {
    let e = base64::engine::general_purpose::STANDARD;
    let pk = e.decode(pubkey_b64.trim()).ok()?;
    let sg = e.decode(sig_b64.trim()).ok()?;
    let pubkey: [u8; 32] = pk.try_into().ok()?;
    let sig: [u8; 64] = sg.try_into().ok()?;
    Some(MailSignature { pubkey, sig })
}

#[cfg(test)]
mod tests {
    use super::*;
    use paloma_core::{Address, Flags, MessageId};

    fn outgoing(subject: &str, body: &str) -> OutgoingMessage {
        OutgoingMessage {
            from: Address::named("Yo", "yo@suyu.net"),
            to: vec![Address::named("Ana", "ana@otro.net")],
            cc: vec![],
            bcc: vec![],
            subject: subject.to_string(),
            body_text: body.to_string(),
            body_html: None,
            in_reply_to: None,
            references: vec![],
            signature: None,
            cuerpos: Vec::new(),
        }
    }

    /// El mensaje "recibido" que espeja un saliente (mismos from/to/subject/body
    /// → mismos bytes canónicos).
    fn received_mirror(out: &OutgoingMessage) -> Message {
        Message {
            id: MessageId("<x@suyu.net>".into()),
            from: out.from.clone(),
            to: out.to.clone(),
            cc: vec![],
            bcc: vec![],
            subject: out.subject.clone(),
            date: 0,
            in_reply_to: None,
            references: vec![],
            body_text: out.body_text.clone(),
            body_html: None,
            flags: Flags::default(),
            signature: SignatureStatus::Unsigned,
            mailbox: "INBOX".into(),
            cuerpos: Vec::new(),
            signer: None,
        }
    }

    #[test]
    fn firmar_y_verificar_roundtrip() {
        let kp = Keypair::from_seed([7; 32]);
        let mut out = outgoing("factura", "el pago vence el viernes");
        sign_outgoing(&kp, &mut out);
        let s = out.signature.unwrap();

        let recibido = received_mirror(&out);
        assert_eq!(verify_message(&recibido, &s.pubkey, &s.sig), SignatureStatus::Verified);
    }

    #[test]
    fn cuerpo_manipulado_invalida() {
        let kp = Keypair::from_seed([7; 32]);
        let mut out = outgoing("factura", "el pago vence el viernes");
        sign_outgoing(&kp, &mut out);
        let s = out.signature.unwrap();

        let mut recibido = received_mirror(&out);
        recibido.body_text = "el pago vence el LUNES".into(); // tampering
        assert_eq!(verify_message(&recibido, &s.pubkey, &s.sig), SignatureStatus::Invalid);
    }

    #[test]
    fn asunto_o_remitente_manipulado_invalida() {
        let kp = Keypair::from_seed([7; 32]);
        let mut out = outgoing("factura", "cuerpo");
        sign_outgoing(&kp, &mut out);
        let s = out.signature.unwrap();

        let mut r1 = received_mirror(&out);
        r1.subject = "otra cosa".into();
        assert_eq!(verify_message(&r1, &s.pubkey, &s.sig), SignatureStatus::Invalid);

        let mut r2 = received_mirror(&out);
        r2.from = Address::named("Impostor", "malo@suyu.net");
        assert_eq!(verify_message(&r2, &s.pubkey, &s.sig), SignatureStatus::Invalid);
    }

    #[test]
    fn clave_equivocada_invalida() {
        let kp = Keypair::from_seed([7; 32]);
        let otra = Keypair::from_seed([8; 32]);
        let mut out = outgoing("hola", "mundo");
        sign_outgoing(&kp, &mut out);
        let s = out.signature.unwrap();

        let recibido = received_mirror(&out);
        // Verificar con la pubkey de otra clave: la firma no cierra.
        assert_eq!(verify_message(&recibido, &otra.public_key(), &s.sig), SignatureStatus::Invalid);
    }

    #[test]
    fn encode_decode_roundtrip() {
        let kp = Keypair::from_seed([42; 32]);
        let sig = MailSignature { pubkey: kp.public_key(), sig: kp.sign(b"x") };
        let (pk_b64, sg_b64) = encode_signature(&sig);
        assert_eq!(decode_signature(&pk_b64, &sg_b64), Some(sig));
        // Basura → None, no panic.
        assert_eq!(decode_signature("no-b64!!", &sg_b64), None);
        assert_eq!(decode_signature(&pk_b64, "AAAA"), None); // largo erróneo
    }
}
