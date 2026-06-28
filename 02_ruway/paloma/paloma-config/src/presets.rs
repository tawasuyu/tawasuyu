//! Presets de proveedor — los servidores y endpoints conocidos de los correos
//! grandes, para autocompletar una cuenta sin que el usuario adivine puertos.
//!
//! Cada preset trae IMAP/SMTP y, si el proveedor exige OAuth2 (Gmail desde 2022,
//! Outlook personal), los endpoints del flujo de autorización + el *scope* de
//! correo. El proveedor `custom` no autocompleta nada (servidor propio).

/// Un preset de proveedor: servidores + (opcional) endpoints OAuth2.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    /// Id estable del preset (`"google"`, `"microsoft"`, `"custom"`).
    pub id: &'static str,
    /// Rótulo para la UI.
    pub label: &'static str,
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub imap_security: &'static str,
    pub smtp_host: &'static str,
    pub smtp_port: u16,
    pub smtp_security: &'static str,
    /// Proveedor OAuth (`"google"`/`"microsoft"`); vacío ⇒ el preset es de
    /// contraseña (servidor propio / IMAP genérico).
    pub oauth_provider: &'static str,
    /// Endpoint de autorización OAuth2 (donde abre el navegador).
    pub auth_url: &'static str,
    /// Endpoint de intercambio/renovación de token.
    pub token_url: &'static str,
    /// Scope OAuth2 que pide acceso IMAP+SMTP del proveedor.
    pub scope: &'static str,
}

/// Catálogo de presets soportados. El primero es el genérico (contraseña).
pub fn presets() -> &'static [Preset] {
    &[
        Preset {
            id: "custom",
            label: "Otro (servidor propio)",
            imap_host: "",
            imap_port: 993,
            imap_security: "tls",
            smtp_host: "",
            smtp_port: 465,
            smtp_security: "tls",
            oauth_provider: "",
            auth_url: "",
            token_url: "",
            scope: "",
        },
        Preset {
            id: "google",
            label: "Gmail / Google Workspace",
            imap_host: "imap.gmail.com",
            imap_port: 993,
            imap_security: "tls",
            smtp_host: "smtp.gmail.com",
            smtp_port: 465,
            smtp_security: "tls",
            oauth_provider: "google",
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
            token_url: "https://oauth2.googleapis.com/token",
            scope: "https://mail.google.com/",
        },
        Preset {
            id: "microsoft",
            label: "Outlook / Microsoft 365",
            imap_host: "outlook.office365.com",
            imap_port: 993,
            imap_security: "tls",
            smtp_host: "smtp.office365.com",
            smtp_port: 587,
            smtp_security: "starttls",
            oauth_provider: "microsoft",
            auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            // offline_access ⇒ refresh_token; SMTP/IMAP scopes de Outlook.
            scope: "offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send",
        },
    ]
}

/// Busca un preset por `id`.
pub fn preset(id: &str) -> Option<&'static Preset> {
    presets().iter().find(|p| p.id == id)
}
