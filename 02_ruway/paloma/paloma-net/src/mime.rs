//! Puente RFC 822 / MIME → `Message` nativo.
//!
//! IMAP entrega bytes crudos del mensaje; acá los parseamos (con
//! `mail-parser`, puro Rust) al [`Message`] de `paloma-core`. Resuelve los
//! headers cotidianos (`From`/`To`/`Cc`/`Subject`/`Date`/`Message-ID`/
//! `In-Reply-To`/`References`), los cuerpos `text/plain` y `text/html`, y los
//! nombres codificados (`=?utf-8?…?=`) que `mail-parser` decodifica solo. Es
//! el único punto donde el formato ajeno toca la suite.

use mail_parser::{Address as MpAddress, HeaderValue, MessageParser};
use paloma_core::{Address, Flags, MailError, Message, MessageId, SignatureStatus};

/// Parsea un mensaje RFC 822 crudo al modelo nativo. `mailbox` y `flags`
/// vienen del lado IMAP (no están en los bytes del mensaje).
pub fn parse_message(raw: &[u8], mailbox: &str, flags: Flags) -> Result<Message, MailError> {
    let parsed = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| MailError::Parse("mensaje vacío o ilegible".into()))?;

    let from = parsed
        .from()
        .and_then(|a| addresses(a).into_iter().next())
        .unwrap_or_else(|| Address::new("desconocido@invalido.local"));

    let to = parsed.to().map(addresses).unwrap_or_default();
    let cc = parsed.cc().map(addresses).unwrap_or_default();
    let bcc = parsed.bcc().map(addresses).unwrap_or_default();

    let subject = parsed.subject().unwrap_or_default().to_string();
    let date = parsed.date().map(|d| d.to_timestamp()).unwrap_or(0);

    let id = parsed
        .message_id()
        .map(ensure_brackets)
        .map(MessageId)
        .unwrap_or_else(|| MessageId(format!("<sin-id-{mailbox}@paloma.local>")));

    let in_reply_to = header_ids(parsed.in_reply_to()).into_iter().next();
    let references = header_ids(parsed.references());

    let body_text = parsed.body_text(0).map(|c| c.into_owned()).unwrap_or_default();
    let body_html = parsed.body_html(0).map(|c| c.into_owned());

    // Firma Ed25519 (Eje 3): si vienen los headers `X-Paloma-*`, recomputamos
    // los bytes canónicos del mensaje y verificamos. Sin headers → Unsigned.
    let pubkey_b64 = parsed.header_raw("X-Paloma-Pubkey");
    let sig_b64 = parsed.header_raw("X-Paloma-Signature");
    let (signature, signer) = match (pubkey_b64, sig_b64) {
        (Some(pk), Some(sg)) => match paloma_sign::decode_signature(pk, sg) {
            Some(ms) => {
                let to_emails: Vec<String> = to.iter().map(|a| a.email.clone()).collect();
                let canonical =
                    paloma_core::canonical_signing_bytes(&from.email, &to_emails, &subject, &body_text);
                let st = paloma_sign::verify(&canonical, &ms.pubkey, &ms.sig);
                // Sólo confiamos en la identidad si la firma verifica.
                let signer = (st == SignatureStatus::Verified).then_some(ms.pubkey);
                (st, signer)
            }
            // Headers presentes pero corruptos → firma rota.
            None => (SignatureStatus::Invalid, None),
        },
        _ => (SignatureStatus::Unsigned, None),
    };

    // Lienzos multilienzo (Eje 4): header `X-Paloma-Cuerpos` (base64 postcard).
    let cuerpos = parsed
        .header_raw("X-Paloma-Cuerpos")
        .and_then(decode_cuerpos)
        .unwrap_or_default();

    Ok(Message {
        id,
        from,
        to,
        cc,
        bcc,
        subject,
        date,
        in_reply_to,
        references,
        body_text,
        body_html,
        flags,
        signature,
        mailbox: mailbox.to_string(),
        cuerpos,
        signer,
    })
}

/// Serializa los lienzos para el header `X-Paloma-Cuerpos` (base64 de postcard).
/// `None` si no hay lienzos (no se emite el header).
pub fn encode_cuerpos(cuerpos: &[paloma_core::MailCuerpo]) -> Option<String> {
    if cuerpos.is_empty() {
        return None;
    }
    let bytes = postcard::to_allocvec(cuerpos).ok()?;
    Some(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes))
}

/// Decodifica el header `X-Paloma-Cuerpos` a los lienzos. `None` si no es base64
/// válido o no decodifica (header corrupto → se ignora, sin romper el parseo).
pub fn decode_cuerpos(header: &str) -> Option<Vec<paloma_core::MailCuerpo>> {
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, header.trim()).ok()?;
    postcard::from_bytes(&bytes).ok()
}

/// Aplana una dirección `mail-parser` (que puede ser lista o grupo) a nuestras
/// [`Address`], quedándose con las que tienen un `address` real.
fn addresses(a: &MpAddress) -> Vec<Address> {
    a.iter()
        .filter_map(|addr| {
            let email = addr.address.as_deref()?.trim().to_string();
            if email.is_empty() {
                return None;
            }
            let name = addr
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            Some(Address { name, email })
        })
        .collect()
}

/// Extrae `Message-ID`s de un header `In-Reply-To`/`References` (texto único o
/// lista). Cada token se normaliza con `<…>`.
fn header_ids(hv: &HeaderValue) -> Vec<MessageId> {
    match hv {
        HeaderValue::Text(s) => extract_ids(s),
        HeaderValue::TextList(list) => list.iter().flat_map(|s| extract_ids(s)).collect(),
        _ => Vec::new(),
    }
}

fn extract_ids(s: &str) -> Vec<MessageId> {
    s.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .map(|t| MessageId(ensure_brackets(t)))
        .collect()
}

/// Garantiza la forma `<id>` (mail-parser entrega el `Message-ID` sin ángulos;
/// los `References` pueden venir con o sin ellos).
fn ensure_brackets(s: impl AsRef<str>) -> String {
    let t = s.as_ref().trim();
    let inner = t.trim_matches(|c| c == '<' || c == '>');
    format!("<{inner}>")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"From: Ana \xC3\x9eerez <ana@ejemplo.com>\r\n\
To: Yo <yo@ejemplo.com>, Bob <bob@x.com>\r\n\
Subject: Hola mundo\r\n\
Date: Mon, 1 Jun 2026 10:00:00 +0000\r\n\
Message-ID: <abc123@ejemplo.com>\r\n\
\r\n\
Cuerpo del mensaje.\r\n";

    #[test]
    fn parsea_headers_y_cuerpo() {
        let m = parse_message(SAMPLE, "INBOX", Flags::default()).unwrap();
        assert_eq!(m.from.email, "ana@ejemplo.com");
        assert!(m.from.name.is_some());
        assert_eq!(m.to.len(), 2);
        assert_eq!(m.to[1].email, "bob@x.com");
        assert_eq!(m.subject, "Hola mundo");
        assert_eq!(m.id, MessageId("<abc123@ejemplo.com>".into()));
        assert!(m.date > 0, "la fecha debería parsearse a un timestamp");
        assert!(m.body_text.contains("Cuerpo del mensaje"));
        assert_eq!(m.mailbox, "INBOX");
    }

    #[test]
    fn parsea_hilado_in_reply_to_y_references() {
        let raw = b"From: a@x.com\r\n\
Subject: Re: Hola\r\n\
Message-ID: <reply@x.com>\r\n\
In-Reply-To: <abc123@ejemplo.com>\r\n\
References: <root@x.com> <abc123@ejemplo.com>\r\n\
\r\n\
ok\r\n";
        let m = parse_message(raw, "INBOX", Flags::default()).unwrap();
        assert_eq!(m.in_reply_to, Some(MessageId("<abc123@ejemplo.com>".into())));
        assert_eq!(m.references.len(), 2);
        assert_eq!(m.references[0], MessageId("<root@x.com>".into()));
    }

    #[test]
    fn flags_y_mailbox_vienen_de_afuera() {
        let f = Flags { seen: true, flagged: true, ..Default::default() };
        let m = parse_message(SAMPLE, "Archivo", f).unwrap();
        assert!(m.flags.seen && m.flags.flagged);
        assert_eq!(m.mailbox, "Archivo");
    }

    #[test]
    fn mensaje_ilegible_da_error() {
        // Bytes sin headers ni estructura: mail-parser igual produce algo,
        // pero un input vacío no debería romper.
        let m = parse_message(b"", "INBOX", Flags::default());
        // vacío → o Err(Parse) o un Message degenerado; no debe panicar.
        let _ = m;
    }

    /// Multilienzo: los lienzos viajan en el header `X-Paloma-Cuerpos` y vuelven
    /// intactos al parsear (escribir una vez → leer en otro idioma).
    #[test]
    fn lienzos_viajan_y_vuelven() {
        use paloma_core::MailCuerpo;
        let cuerpos = vec![
            MailCuerpo { lang: "en".into(), tone: None, body_text: "see you friday".into() },
            MailCuerpo { lang: "qu".into(), tone: Some("cercano".into()), body_text: "tinkusunchik".into() },
        ];
        let header = encode_cuerpos(&cuerpos).unwrap();
        let raw = format!(
            "From: Yo <yo@x.com>\r\nTo: Ana <ana@x.com>\r\nSubject: hola\r\n\
             X-Paloma-Cuerpos: {header}\r\n\r\nnos vemos el viernes\r\n"
        );
        let m = parse_message(raw.as_bytes(), "INBOX", Flags::default()).unwrap();
        assert_eq!(m.cuerpos.len(), 2);
        assert_eq!(m.body_for("en"), "see you friday");
        assert_eq!(m.body_for("qu"), "tinkusunchik");
        // Sin lienzo en ese idioma → cae al cuerpo principal.
        assert_eq!(m.body_for("fr").trim(), "nos vemos el viernes");
        // Header corrupto → se ignora, no rompe el parseo.
        let roto = raw.replace(&header, "no-base64!!");
        assert!(parse_message(roto.as_bytes(), "INBOX", Flags::default()).unwrap().cuerpos.is_empty());
    }

    /// Sin los headers `X-Paloma-*`, la firma queda `Unsigned`.
    #[test]
    fn sin_headers_de_firma_es_unsigned() {
        let m = parse_message(SAMPLE, "INBOX", Flags::default()).unwrap();
        assert_eq!(m.signature, SignatureStatus::Unsigned);
    }

    /// Roundtrip de firma de punta a punta por el cable: firmamos un saliente,
    /// emitimos los headers base64 como lo hace SMTP, parseamos los bytes y la
    /// verificación recomputa los bytes canónicos → `Verified`. Manipular el
    /// cuerpo deja la firma `Invalid`.
    #[test]
    fn verifica_firma_del_entrante() {
        use agora_core::Keypair;
        use paloma_core::{Address, OutgoingMessage};

        let kp = Keypair::from_seed([7; 32]);
        let mut out = OutgoingMessage {
            from: Address::named("Yo", "yo@suyu.net"),
            to: vec![Address::named("Ana", "ana@otro.net")],
            cc: vec![],
            bcc: vec![],
            subject: "factura".into(),
            body_text: "el pago vence el viernes".into(),
            body_html: None,
            in_reply_to: None,
            references: vec![],
            signature: None,
            cuerpos: Vec::new(),
        };
        paloma_sign::sign_outgoing(&kp, &mut out);
        let (pk_b64, sg_b64) = paloma_sign::encode_signature(out.signature.as_ref().unwrap());

        // Construimos los bytes RFC822 como saldrían por SMTP (headers + cuerpo).
        let raw = format!(
            "From: Yo <yo@suyu.net>\r\n\
             To: Ana <ana@otro.net>\r\n\
             Subject: factura\r\n\
             X-Paloma-Pubkey: {pk_b64}\r\n\
             X-Paloma-Signature: {sg_b64}\r\n\
             \r\n\
             el pago vence el viernes\r\n"
        );
        let m = parse_message(raw.as_bytes(), "INBOX", Flags::default()).unwrap();
        assert_eq!(m.signature, SignatureStatus::Verified, "la firma íntegra verifica");

        // Mismo correo con el cuerpo manipulado → la firma no cierra.
        let tampered = raw.replace("el viernes", "el LUNES");
        let mt = parse_message(tampered.as_bytes(), "INBOX", Flags::default()).unwrap();
        assert_eq!(mt.signature, SignatureStatus::Invalid, "cuerpo alterado invalida");
    }
}
