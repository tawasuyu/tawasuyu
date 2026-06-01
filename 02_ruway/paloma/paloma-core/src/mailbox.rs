use serde::{Deserialize, Serialize};

/// Rol semántico de un buzón. Independiza la UI del nombre concreto que use
/// cada servidor (`INBOX`, `[Gmail]/Sent Mail`, `Sent`, `Enviados`…): el
/// frontend pinta un ícono/orden por rol, no por texto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MailboxRole {
    /// Entrada — lo que llega.
    Inbox,
    /// Enviados.
    Sent,
    /// Borradores.
    Drafts,
    /// Papelera.
    Trash,
    /// Spam / correo no deseado.
    Junk,
    /// Archivo.
    Archive,
    /// Cualquier carpeta del usuario sin rol especial.
    Custom,
}

impl MailboxRole {
    /// Orden de presentación canónico (Inbox primero, Custom al final).
    pub fn sort_key(self) -> u8 {
        match self {
            MailboxRole::Inbox => 0,
            MailboxRole::Drafts => 1,
            MailboxRole::Sent => 2,
            MailboxRole::Archive => 3,
            MailboxRole::Junk => 4,
            MailboxRole::Trash => 5,
            MailboxRole::Custom => 6,
        }
    }
}

/// Un buzón/carpeta del servidor. `name` es la ruta completa tal cual la
/// expone el servidor (p. ej. `INBOX`, `Trabajo/Clientes`); `role` se infiere
/// de ese nombre con [`MailboxRole`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    pub name: String,
    pub role: MailboxRole,
}

impl Mailbox {
    /// Construye un buzón infiriendo el rol del nombre.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let role = role_from_name(&name);
        Self { name, role }
    }

    /// Construye un buzón con un rol explícito (cuando el servidor lo anuncia
    /// vía SPECIAL-USE en vez de dejarlo al nombre).
    pub fn with_role(name: impl Into<String>, role: MailboxRole) -> Self {
        Self { name: name.into(), role }
    }

    /// El segmento final de la ruta — el nombre "corto" para mostrar.
    pub fn leaf_name(&self) -> &str {
        self.name.rsplit(['/', '.']).next().unwrap_or(&self.name)
    }
}

/// Infiere el rol a partir del nombre del buzón. Reconoce los nombres en
/// inglés y español más comunes; lo que no matchea es `Custom`.
fn role_from_name(name: &str) -> MailboxRole {
    let leaf = name.rsplit(['/', '.']).next().unwrap_or(name).trim();
    let lower = leaf.to_ascii_lowercase();
    match lower.as_str() {
        "inbox" | "entrada" | "bandeja de entrada" | "recibidos" => MailboxRole::Inbox,
        "sent" | "sent mail" | "sent items" | "enviados" => MailboxRole::Sent,
        "drafts" | "draft" | "borradores" => MailboxRole::Drafts,
        "trash" | "deleted" | "deleted items" | "papelera" | "eliminados" => MailboxRole::Trash,
        "junk" | "spam" | "no deseado" | "correo no deseado" => MailboxRole::Junk,
        "archive" | "archivo" | "all mail" => MailboxRole::Archive,
        _ => MailboxRole::Custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infiere_rol_de_nombres_comunes() {
        assert_eq!(Mailbox::new("INBOX").role, MailboxRole::Inbox);
        assert_eq!(Mailbox::new("[Gmail]/Sent Mail").role, MailboxRole::Sent);
        assert_eq!(Mailbox::new("Enviados").role, MailboxRole::Sent);
        assert_eq!(Mailbox::new("Trabajo/Clientes").role, MailboxRole::Custom);
        assert_eq!(Mailbox::new("Spam").role, MailboxRole::Junk);
    }

    #[test]
    fn leaf_name_toma_el_ultimo_segmento() {
        assert_eq!(Mailbox::new("Trabajo/Clientes").leaf_name(), "Clientes");
        assert_eq!(Mailbox::new("INBOX").leaf_name(), "INBOX");
    }

    #[test]
    fn orden_inbox_primero() {
        assert!(MailboxRole::Inbox.sort_key() < MailboxRole::Sent.sort_key());
        assert!(MailboxRole::Trash.sort_key() < MailboxRole::Custom.sort_key());
    }
}
