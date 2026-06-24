//! `paloma-test` — probador de conexión IMAP+SMTP, sin GUI.
//!
//! Verifica los caminos de red de `paloma-net` contra un servidor real y
//! reporta en texto qué pasó. No abre ninguna ventana ni toca la caché.
//!
//! ## Uso rápido (Gmail)
//!
//! ```sh
//! export PALOMA_EMAIL="tucorreo@gmail.com"
//! export PALOMA_PASSWORD="xxxx xxxx xxxx xxxx"   # contraseña de aplicación, NO la del login
//! cargo run -p paloma-app --bin paloma-test --release
//! ```
//!
//! Con `PALOMA_EMAIL` terminando en `@gmail.com`/`@googlemail.com` toma por
//! defecto `imap.gmail.com:993` (TLS) y `smtp.gmail.com:587` (STARTTLS). Para
//! otros proveedores, o pasás un `cuenta.json` (`PALOMA_CONFIG`/ubicación
//! estándar) o seteás los hosts a mano:
//!
//! ```sh
//! export PALOMA_IMAP_HOST=imap.dominio.com PALOMA_IMAP_PORT=993 PALOMA_IMAP_SEC=tls
//! export PALOMA_SMTP_HOST=smtp.dominio.com PALOMA_SMTP_PORT=465 PALOMA_SMTP_SEC=tls
//! ```
//!
//! La contraseña SIEMPRE por entorno (`PALOMA_PASSWORD`, o
//! `PALOMA_IMAP_PASSWORD`/`PALOMA_SMTP_PASSWORD`). Nunca en archivo.
//!
//! Para además mandar un correo de prueba a vos mismo:
//! `PALOMA_SEND_TEST=1`.

use std::process::ExitCode;

use paloma_core::{Account, Address, MailBackend, OutgoingMessage, Security, ServerConfig};
use paloma_net::NetBackend;

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|s| !s.trim().is_empty())
}

fn parse_security(s: &str) -> Security {
    match s.to_ascii_lowercase().as_str() {
        "plain" | "none" => Security::Plain,
        "starttls" => Security::StartTls,
        _ => Security::Tls,
    }
}

fn sec_label(s: Security) -> &'static str {
    match s {
        Security::Tls => "TLS",
        Security::StartTls => "STARTTLS",
        Security::Plain => "plano",
    }
}

/// Arma la cuenta desde el entorno. Devuelve `(account, motivo)` o un error
/// explicando qué falta.
fn cuenta_desde_entorno() -> Result<(Account, String), String> {
    let email = env("PALOMA_EMAIL")
        .ok_or("Falta PALOMA_EMAIL (tu dirección de correo).".to_string())?;
    let display = env("PALOMA_NAME").unwrap_or_else(|| email.clone());
    let user = env("PALOMA_USER").unwrap_or_else(|| email.clone());

    let dominio = email.rsplit('@').next().unwrap_or("").to_ascii_lowercase();
    let es_gmail = dominio == "gmail.com" || dominio == "googlemail.com";

    // Defaults de Gmail si aplica; cualquier env los pisa.
    let (def_ih, def_ip, def_is, def_sh, def_sp, def_ss) = if es_gmail {
        ("imap.gmail.com", 993, "tls", "smtp.gmail.com", 587, "starttls")
    } else {
        ("", 993, "tls", "", 465, "tls")
    };

    let imap_host = env("PALOMA_IMAP_HOST").unwrap_or_else(|| def_ih.to_string());
    let smtp_host = env("PALOMA_SMTP_HOST").unwrap_or_else(|| def_sh.to_string());
    if imap_host.is_empty() || smtp_host.is_empty() {
        return Err(format!(
            "No conozco los servidores de '{dominio}'. Seteá PALOMA_IMAP_HOST y PALOMA_SMTP_HOST."
        ));
    }
    let imap_port: u16 = env("PALOMA_IMAP_PORT").and_then(|s| s.parse().ok()).unwrap_or(def_ip);
    let smtp_port: u16 = env("PALOMA_SMTP_PORT").and_then(|s| s.parse().ok()).unwrap_or(def_sp);
    let imap_sec = parse_security(&env("PALOMA_IMAP_SEC").unwrap_or_else(|| def_is.to_string()));
    let smtp_sec = parse_security(&env("PALOMA_SMTP_SEC").unwrap_or_else(|| def_ss.to_string()));

    let imap = ServerConfig::new(imap_host.clone(), imap_port, imap_sec, user.clone());
    let smtp = ServerConfig::new(smtp_host.clone(), smtp_port, smtp_sec, user);
    let acc = Account::new("default", display.clone(), Address::named(display, email), imap, smtp);

    let motivo = format!(
        "IMAP {imap_host}:{imap_port} ({}) · SMTP {smtp_host}:{smtp_port} ({})",
        sec_label(imap_sec),
        sec_label(smtp_sec),
    );
    Ok((acc, motivo))
}

fn passwords() -> Result<(String, String), String> {
    let both = env("PALOMA_PASSWORD");
    let imap = env("PALOMA_IMAP_PASSWORD").or_else(|| both.clone());
    let smtp = env("PALOMA_SMTP_PASSWORD").or(both);
    match (imap, smtp) {
        (Some(i), Some(s)) => Ok((i, s)),
        _ => Err("Falta PALOMA_PASSWORD (o PALOMA_IMAP_PASSWORD/PALOMA_SMTP_PASSWORD). \
                  En Gmail es una CONTRASEÑA DE APLICACIÓN, no la del login."
            .to_string()),
    }
}

fn main() -> ExitCode {
    println!("paloma · probador de conexión\n");

    let (account, donde) = match cuenta_desde_entorno() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::FAILURE;
        }
    };
    let (imap_pw, smtp_pw) = match passwords() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::FAILURE;
        }
    };

    let me = account.address.clone();
    println!("Cuenta : {me}");
    println!("Destino: {donde}\n");

    // --- IMAP: conectar + autenticar ---
    print!("→ IMAP: conectando y autenticando… ");
    let backend = match NetBackend::connect(account, &imap_pw, &smtp_pw) {
        Ok(b) => {
            println!("OK");
            b
        }
        Err(e) => {
            println!("FALLÓ");
            eprintln!("  ✗ {e}");
            eprintln!("  Pista: en Gmail necesitás 2FA + contraseña de aplicación; \
                       revisá host/puerto/seguridad.");
            return ExitCode::FAILURE;
        }
    };
    backend.set_fetch_limit(Some(20));

    // --- IMAP: listar buzones ---
    print!("→ IMAP: listando buzones… ");
    let mailboxes = match backend.list_mailboxes() {
        Ok(m) => {
            println!("OK ({} buzones)", m.len());
            m
        }
        Err(e) => {
            println!("FALLÓ");
            eprintln!("  ✗ {e}");
            return ExitCode::FAILURE;
        }
    };
    for mb in mailboxes.iter().take(15) {
        println!("    · {}", mb.name);
    }
    if mailboxes.len() > 15 {
        println!("    … (+{} más)", mailboxes.len() - 15);
    }

    // --- IMAP: traer los últimos N de INBOX ---
    let inbox = mailboxes
        .iter()
        .find(|m| m.name.eq_ignore_ascii_case("INBOX"))
        .map(|m| m.name.clone())
        .unwrap_or_else(|| "INBOX".to_string());
    print!("\n→ IMAP: trayendo los últimos de '{inbox}'… ");
    match backend.fetch_messages(&inbox) {
        Ok(msgs) => {
            println!("OK ({} mensajes)", msgs.len());
            for m in msgs.iter().rev().take(5) {
                println!("    · {}  —  {}", m.from, m.subject);
            }
        }
        Err(e) => {
            println!("FALLÓ");
            eprintln!("  ✗ {e}");
            return ExitCode::FAILURE;
        }
    }

    // --- SMTP: opcional, enviar prueba a uno mismo ---
    if env("PALOMA_SEND_TEST").is_some() {
        print!("\n→ SMTP: enviando un correo de prueba a vos mismo… ");
        let out = OutgoingMessage {
            from: me.clone(),
            to: vec![me.clone()],
            cc: vec![],
            bcc: vec![],
            subject: "paloma · prueba de envío".to_string(),
            body_text: "Si ves esto, paloma puede enviar por SMTP. 🕊".to_string(),
            body_html: None,
            in_reply_to: None,
            references: vec![],
            signature: None,
            cuerpos: Vec::new(),
        };
        match backend.send(&out) {
            Ok(id) => println!("OK (Message-ID {id})"),
            Err(e) => {
                println!("FALLÓ");
                eprintln!("  ✗ {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("\n(SMTP no probado — poné PALOMA_SEND_TEST=1 para mandarte un correo de prueba.)");
    }

    println!("\n✓ Conexión verificada.");
    ExitCode::SUCCESS
}
