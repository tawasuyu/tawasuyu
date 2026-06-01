//! Envío SMTP (sobre `lettre` + `native-tls`).
//!
//! Arma el RFC 822 desde un [`OutgoingMessage`] y lo manda por el relay de la
//! cuenta. Soporta TLS implícito (465), STARTTLS (587) y plano (sólo pruebas).

use std::time::{SystemTime, UNIX_EPOCH};

use lettre::message::{Mailbox as LettreMailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message as LettreMessage, SmtpTransport, Transport};
use paloma_core::{Address, MailError, MessageId, OutgoingMessage, Security, ServerConfig};

/// Envía `msg` por el servidor `cfg`. Devuelve el `Message-ID` asignado
/// (lo generamos nosotros y lo fijamos en el header, así el store puede
/// referenciar el enviado).
pub fn send(cfg: &ServerConfig, password: &str, msg: &OutgoingMessage) -> Result<MessageId, MailError> {
    let domain = msg.from.domain().unwrap_or("paloma.local");
    let id = MessageId(format!("<{}@{}>", unique_token(), domain));

    let mut builder = LettreMessage::builder()
        .from(to_mailbox(&msg.from)?)
        .subject(msg.subject.clone())
        .message_id(Some(id.0.trim_matches(|c| c == '<' || c == '>').to_string()));
    for a in &msg.to {
        builder = builder.to(to_mailbox(a)?);
    }
    for a in &msg.cc {
        builder = builder.cc(to_mailbox(a)?);
    }
    for a in &msg.bcc {
        builder = builder.bcc(to_mailbox(a)?);
    }
    if let Some(irt) = &msg.in_reply_to {
        builder = builder.in_reply_to(irt.0.clone());
    }
    if !msg.references.is_empty() {
        let refs = msg.references.iter().map(|r| r.0.clone()).collect::<Vec<_>>().join(" ");
        builder = builder.references(refs);
    }

    let email = match &msg.body_html {
        Some(html) => builder.multipart(MultiPart::alternative_plain_html(
            msg.body_text.clone(),
            html.clone(),
        )),
        None => builder.body(msg.body_text.clone()),
    }
    .map_err(|e| MailError::Parse(e.to_string()))?;

    let creds = Credentials::new(cfg.username.clone(), password.to_string());
    let transport = match cfg.security {
        Security::Tls => SmtpTransport::relay(&cfg.host).map_err(map_err)?,
        Security::StartTls => SmtpTransport::starttls_relay(&cfg.host).map_err(map_err)?,
        Security::Plain => SmtpTransport::builder_dangerous(&cfg.host),
    }
    .port(cfg.port)
    .credentials(creds)
    .build();

    transport.send(&email).map_err(map_err)?;
    Ok(id)
}

fn to_mailbox(a: &Address) -> Result<LettreMailbox, MailError> {
    a.to_string()
        .parse::<LettreMailbox>()
        .map_err(|e| MailError::Parse(format!("dirección inválida «{a}»: {e}")))
}

fn map_err(e: lettre::transport::smtp::Error) -> MailError {
    MailError::Transport(e.to_string())
}

/// Token único para el `Message-ID` (nanos desde epoch). Suficiente para no
/// colisionar entre envíos de un cliente.
fn unique_token() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}
