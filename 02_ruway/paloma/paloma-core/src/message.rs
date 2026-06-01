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

/// Estado de la firma criptográfica de un mensaje (Ed25519, vía la identidad de
/// `agora`). `Unsigned` es lo normal hoy; la **verificación** del entrante la
/// completa la integración con `agora` (ver LEEME · Pendiente) — este enum es el
/// dato que esa capa va a poblar y que la UI ya sabe pintar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignatureStatus {
    /// Sin firma (o sin verificar): el caso por defecto.
    #[default]
    Unsigned,
    /// Firma presente y válida para el remitente declarado.
    Verified,
    /// Firma presente pero que no valida (manipulado o clave equivocada).
    Invalid,
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
    /// Estado de la firma Ed25519 del mensaje. `#[serde(default)]` para que las
    /// cachés viejas (sin el campo) sigan decodificando como `Unsigned`.
    #[serde(default)]
    pub signature: SignatureStatus,
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

    /// El cuerpo a mostrar en modo lectura sobria: el texto plano si lo hay; si
    /// el mensaje vino sólo en HTML, una versión despojada de etiquetas. Así la
    /// UI nativa siempre tiene algo legible sin embeber un motor HTML (puriy
    /// pinta el HTML rico recién cuando el usuario lo pide).
    pub fn display_body(&self) -> String {
        if !self.body_text.trim().is_empty() {
            return self.body_text.clone();
        }
        match &self.body_html {
            Some(html) => strip_html(html),
            None => String::new(),
        }
    }
}

/// Despoja un fragmento HTML a texto plano legible: convierte saltos de bloque
/// (`<br>`, `</p>`, `</div>`, `</tr>`, `</li>`) en newlines, descarta el resto
/// de las etiquetas y `<style>`/`<script>` enteros, decodifica las entidades
/// más comunes y colapsa el whitespace horizontal. No pretende renderizar HTML
/// —para eso está puriy— sino dar un texto leíble cuando el correo no trae
/// `text/plain`.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        // Texto antes de la etiqueta (UTF-8 intacto vía slices de string).
        out.push_str(&rest[..lt]);
        let after = &rest[lt + 1..];
        let gt = after.find('>').unwrap_or(after.len());
        let tag = after[..gt].to_ascii_lowercase();
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        // Saltar el contenido entero de <style>/<script>.
        if name == "style" || name == "script" {
            let close = format!("</{name}>");
            let tail = if gt < after.len() { &after[gt + 1..] } else { "" };
            match tail.to_ascii_lowercase().find(&close) {
                Some(rel) => {
                    rest = &tail[rel + close.len()..];
                    continue;
                }
                None => {
                    rest = "";
                    break;
                }
            }
        }
        if matches!(name.as_str(), "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "ul" | "ol") {
            out.push('\n');
        }
        rest = if gt < after.len() { &after[gt + 1..] } else { "" };
    }
    out.push_str(rest);
    let decoded = decode_entities(&out);
    // Colapsar espacios/tabs horizontales sin tocar los newlines, y recortar
    // líneas en blanco repetidas.
    let mut result = String::with_capacity(decoded.len());
    for (n, line) in decoded.lines().enumerate() {
        let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() && result.ends_with("\n\n") {
            continue;
        }
        if n > 0 {
            result.push('\n');
        }
        result.push_str(&collapsed);
    }
    result.trim().to_string()
}

/// Decodifica las entidades HTML más frecuentes. Subconjunto a propósito:
/// cubre lo cotidiano sin arrastrar una tabla completa.
fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&hellip;", "…")
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
            signature: SignatureStatus::Unsigned,
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

    #[test]
    fn strip_html_da_texto_legible() {
        let html = "<style>.x{}</style><p>Hola&nbsp;<b>Ana</b></p><div>línea 2 &amp; fin</div>";
        let txt = strip_html(html);
        assert!(txt.contains("Hola Ana"));
        assert!(txt.contains("línea 2 & fin"));
        assert!(!txt.contains('<'));
        assert!(!txt.contains(".x{}"), "el contenido de <style> se descarta");
    }

    #[test]
    fn display_body_cae_a_html_si_no_hay_texto() {
        let mut m = msg("", "x");
        m.body_text = String::new();
        m.body_html = Some("<p>Sólo&gt;HTML</p>".into());
        assert_eq!(m.display_body(), "Sólo>HTML");
        // Con texto plano, lo prefiere.
        m.body_text = "plano".into();
        assert_eq!(m.display_body(), "plano");
    }
}
