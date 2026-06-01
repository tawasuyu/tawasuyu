use thiserror::Error;

/// Errores del núcleo de correo. El transporte real (IMAP/SMTP) mapea sus
/// fallos a estas variantes para que los frontends no dependan del crate de
/// red concreto.
#[derive(Debug, Error)]
pub enum MailError {
    /// Fallo de red/transporte (conexión, TLS, timeout…).
    #[error("transporte: {0}")]
    Transport(String),
    /// Credenciales rechazadas por el servidor.
    #[error("autenticación rechazada")]
    Auth,
    /// El buzón pedido no existe en el servidor/caché.
    #[error("buzón desconocido: {0}")]
    UnknownMailbox(String),
    /// El mensaje pedido no existe.
    #[error("mensaje desconocido: {0}")]
    UnknownMessage(String),
    /// Un header/cuerpo no se pudo parsear.
    #[error("parseo: {0}")]
    Parse(String),
}
