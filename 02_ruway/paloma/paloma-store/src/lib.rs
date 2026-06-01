//! paloma-store — la caché en disco del correo.
//!
//! Persiste lo que el cliente ya trajo (buzones y mensajes por buzón) para que
//! paloma abra **offline-first**: al arrancar pinta lo último conocido y recién
//! después refresca contra el servidor. Es la contraparte durable de la caché
//! en memoria (`paloma_core::MailStore`), no un segundo modelo: guarda los
//! mismos tipos nativos, serializados con **postcard** (compacto, sin reflexión)
//! y direccionados por **BLAKE3** del nombre del buzón (que puede traer `/`,
//! espacios y mayúsculas que no sirven como nombre de archivo).
//!
//! Es agnóstica a la red y a la UI: sólo sabe de `Mailbox`/`Message` y del
//! sistema de archivos. El sync incremental se apoya en esto — hoy reemplaza el
//! snapshot por buzón en cada `save`; el delta por UID llega en una fase
//! posterior sobre la misma estructura.
//!
//! Layout en disco:
//! ```text
//! <root>/<account_id>/buzones.pc            ← lista de Mailbox (postcard)
//! <root>/<account_id>/msgs-<blake3hex>.pc   ← Vec<Message> de un buzón
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use paloma_core::{Mailbox, Message};
use thiserror::Error;

/// Errores de la caché en disco.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Fallo de (de)serialización postcard — un blob corrupto o de otra versión.
    #[error("códec: {0}")]
    Codec(String),
}

impl From<postcard::Error> for StoreError {
    fn from(e: postcard::Error) -> Self {
        StoreError::Codec(e.to_string())
    }
}

/// La caché: una raíz de disco bajo la cual cuelga un directorio por cuenta.
/// Barata de clonar (sólo un `PathBuf`).
#[derive(Debug, Clone)]
pub struct MailDb {
    root: PathBuf,
}

impl MailDb {
    /// Abre (creando si hace falta) la caché bajo `root`. No toca la red.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Directorio de una cuenta, creado al vuelo. El `account_id` se sanea a un
    /// nombre de archivo seguro (no confiamos en que sea un slug).
    fn account_dir(&self, account_id: &str) -> Result<PathBuf, StoreError> {
        let dir = self.root.join(sanitize(account_id));
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Persiste la lista de buzones de una cuenta (reemplaza la anterior).
    pub fn save_mailboxes(&self, account_id: &str, mailboxes: &[Mailbox]) -> Result<(), StoreError> {
        let path = self.account_dir(account_id)?.join("buzones.pc");
        let bytes = postcard::to_stdvec(mailboxes)?;
        write_atomic(&path, &bytes)
    }

    /// Lee los buzones cacheados; vacío si no hay nada guardado todavía.
    pub fn load_mailboxes(&self, account_id: &str) -> Vec<Mailbox> {
        let Ok(dir) = self.account_dir(account_id) else { return Vec::new() };
        read_postcard(&dir.join("buzones.pc")).unwrap_or_default()
    }

    /// Persiste los mensajes de un buzón (reemplaza el snapshot anterior).
    pub fn save_messages(
        &self,
        account_id: &str,
        mailbox: &str,
        messages: &[Message],
    ) -> Result<(), StoreError> {
        let path = self.account_dir(account_id)?.join(mailbox_file(mailbox));
        let bytes = postcard::to_stdvec(messages)?;
        write_atomic(&path, &bytes)
    }

    /// Lee los mensajes cacheados de un buzón; vacío si no hay snapshot.
    pub fn load_messages(&self, account_id: &str, mailbox: &str) -> Vec<Message> {
        let Ok(dir) = self.account_dir(account_id) else { return Vec::new() };
        read_postcard(&dir.join(mailbox_file(mailbox))).unwrap_or_default()
    }
}

/// Nombre de archivo de un buzón: `msgs-<blake3hex>.pc`. El hash evita que `/`,
/// espacios o mayúsculas del nombre del buzón rompan la ruta.
fn mailbox_file(mailbox: &str) -> String {
    let hash = blake3::hash(mailbox.as_bytes()).to_hex();
    format!("msgs-{hash}.pc")
}

/// Sanea un `account_id` a un segmento de ruta seguro (alfanumérico, `-`, `_`).
fn sanitize(s: &str) -> String {
    let clean: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if clean.is_empty() { "default".to_string() } else { clean }
}

/// Lee y deserializa un blob postcard; `None` si el archivo no existe o el blob
/// no decodifica (versión vieja/corrupto) — la caché es best-effort.
fn read_postcard<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    postcard::from_bytes(&bytes).ok()
}

/// Escribe `bytes` de forma atómica: a un `.tmp` y luego `rename`, para no
/// dejar un snapshot a medio escribir si el proceso muere en el medio.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use paloma_core::{Address, Flags, MessageId};

    fn msg(id: &str) -> Message {
        Message {
            id: MessageId(id.into()),
            from: Address::named("Ana", "ana@x.com"),
            to: vec![Address::new("yo@x.com")],
            cc: vec![],
            bcc: vec![],
            subject: "Hola".into(),
            date: 100,
            in_reply_to: None,
            references: vec![],
            body_text: "cuerpo".into(),
            body_html: None,
            flags: Flags { seen: true, ..Default::default() },
            mailbox: "INBOX".into(),
        }
    }

    #[test]
    fn roundtrip_mensajes_por_buzon() {
        let dir = tempfile::tempdir().unwrap();
        let db = MailDb::open(dir.path()).unwrap();
        let msgs = vec![msg("<1@x>"), msg("<2@x>")];
        db.save_messages("acc1", "INBOX", &msgs).unwrap();
        let back = db.load_messages("acc1", "INBOX");
        assert_eq!(back, msgs);
    }

    #[test]
    fn buzon_con_nombre_raro_no_rompe_la_ruta() {
        let dir = tempfile::tempdir().unwrap();
        let db = MailDb::open(dir.path()).unwrap();
        let msgs = vec![msg("<1@x>")];
        db.save_messages("acc1", "[Gmail]/Sent Mail", &msgs).unwrap();
        assert_eq!(db.load_messages("acc1", "[Gmail]/Sent Mail"), msgs);
        // Buzón distinto → snapshot distinto, no se pisan.
        assert!(db.load_messages("acc1", "INBOX").is_empty());
    }

    #[test]
    fn roundtrip_buzones() {
        let dir = tempfile::tempdir().unwrap();
        let db = MailDb::open(dir.path()).unwrap();
        let boxes = vec![Mailbox::new("INBOX"), Mailbox::new("Enviados")];
        db.save_mailboxes("acc1", &boxes).unwrap();
        assert_eq!(db.load_mailboxes("acc1"), boxes);
    }

    #[test]
    fn miss_devuelve_vacio() {
        let dir = tempfile::tempdir().unwrap();
        let db = MailDb::open(dir.path()).unwrap();
        assert!(db.load_messages("nadie", "INBOX").is_empty());
        assert!(db.load_mailboxes("nadie").is_empty());
    }

    #[test]
    fn cuentas_aisladas() {
        let dir = tempfile::tempdir().unwrap();
        let db = MailDb::open(dir.path()).unwrap();
        db.save_messages("a", "INBOX", &[msg("<a@x>")]).unwrap();
        db.save_messages("b", "INBOX", &[msg("<b@x>"), msg("<b2@x>")]).unwrap();
        assert_eq!(db.load_messages("a", "INBOX").len(), 1);
        assert_eq!(db.load_messages("b", "INBOX").len(), 2);
    }
}
