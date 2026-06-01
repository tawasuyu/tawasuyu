use std::fmt;

use serde::{Deserialize, Serialize};

use crate::address::Address;

/// El `Message-ID` RFC 5322 de un mensaje (`<algo@host>`). Se conserva tal
/// cual lo trae el header para poder hilar respuestas (`In-Reply-To`/
/// `References`) por igualdad exacta.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Flags IMAP de un mensaje. Booleanos en vez de un bitset para que serde y
/// la UI los lean directo; el puente IMAP los mapea desde `\Seen`, `\Flagged`…
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    /// Leído (`\Seen`).
    pub seen: bool,
    /// Respondido (`\Answered`).
    pub answered: bool,
    /// Destacado/estrella (`\Flagged`).
    pub flagged: bool,
    /// Borrador (`\Draft`).
    pub draft: bool,
    /// Marcado para borrar (`\Deleted`).
    pub deleted: bool,
}

/// Un mensaje ya parseado: headers relevantes + cuerpo + flags + el buzón en
/// el que vive. El cuerpo se guarda en texto plano (siempre) y, si el mensaje
/// era `multipart/alternative`, también el HTML — el frontend elige cuál
/// pinta (puriy/Llimphi para el HTML, texto para el modo lectura sobria).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    /// Fecha de envío, en segundos Unix (UTC). Agnóstico a cualquier crate de
    /// tiempo; el puente convierte el header `Date` a este entero.
    pub date: i64,
    /// `In-Reply-To`: el mensaje al que responde, si hilea.
    pub in_reply_to: Option<MessageId>,
    /// `References`: la cadena de ancestros del hilo (más viejo → más nuevo).
    pub references: Vec<MessageId>,
    pub body_text: String,
    pub body_html: Option<String>,
    pub flags: Flags,
    /// Nombre del buzón donde reside (clave en [`crate::MailStore`]).
    pub mailbox: String,
}

impl Message {
    /// Un extracto de una línea para la lista de mensajes: colapsa whitespace
    /// y recorta a `max` caracteres con elipsis.
    pub fn snippet(&self, max: usize) -> String {
        let collapsed: String = self.body_text.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.chars().count() <= max {
            collapsed
        } else {
            let mut out: String = collapsed.chars().take(max.saturating_sub(1)).collect();
            out.push('…');
            out
        }
    }

    /// El asunto para una respuesta: `Re: <asunto>` sin duplicar el prefijo.
    pub fn reply_subject(&self) -> String {
        let base = self.subject.trim();
        if base.to_ascii_lowercase().starts_with("re:") {
            base.to_string()
        } else {
            format!("Re: {base}")
        }
    }

    /// `true` si el mensaje no fue leído.
    pub fn is_unread(&self) -> bool {
        !self.flags.seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(body: &str, subject: &str) -> Message {
        Message {
            id: MessageId("<a@x>".into()),
            from: Address::new("a@x.com"),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: subject.into(),
            date: 0,
            in_reply_to: None,
            references: vec![],
            body_text: body.into(),
            body_html: None,
            flags: Flags::default(),
            mailbox: "INBOX".into(),
        }
    }

    #[test]
    fn snippet_colapsa_y_recorta() {
        let m = msg("  hola   mundo\n  esto es  largo ", "x");
        assert_eq!(m.snippet(100), "hola mundo esto es largo");
        assert_eq!(m.snippet(5), "hola…");
    }

    #[test]
    fn reply_subject_no_duplica_re() {
        assert_eq!(msg("", "Hola").reply_subject(), "Re: Hola");
        assert_eq!(msg("", "Re: Hola").reply_subject(), "Re: Hola");
    }

    #[test]
    fn unread_por_defecto() {
        assert!(msg("", "x").is_unread());
    }
}
