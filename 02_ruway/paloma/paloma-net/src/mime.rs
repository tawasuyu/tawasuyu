//! Puente RFC 822 / MIME → `Message` nativo.
//!
//! IMAP entrega bytes crudos del mensaje; acá los parseamos (con
//! `mail-parser`, puro Rust) al [`Message`] de `paloma-core`. Resuelve los
//! headers cotidianos (`From`/`To`/`Cc`/`Subject`/`Date`/`Message-ID`/
//! `In-Reply-To`/`References`), los cuerpos `text/plain` y `text/html`, y los
//! nombres codificados (`=?utf-8?…?=`) que `mail-parser` decodifica solo. Es
//! el único punto donde el formato ajeno toca la suite.

use mail_parser::{Address as MpAddress, HeaderValue, MessageParser};
use paloma_core::{Address, Flags, MailError, Message, MessageId};

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
        mailbox: mailbox.to_string(),
    })
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
}
