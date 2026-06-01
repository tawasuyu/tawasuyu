use thiserror::Error;

/// Errores del núcleo de calendario/contactos. El transporte real (CalDAV/
/// CardDAV) mapea sus fallos a estas variantes para que los frontends no
/// dependan del crate de red concreto.
#[derive(Debug, Error)]
pub enum CalError {
    /// Fallo de red/transporte (conexión, TLS, timeout…).
    #[error("transporte: {0}")]
    Transport(String),
    /// Credenciales rechazadas por el servidor.
    #[error("autenticación rechazada")]
    Auth,
    /// El calendario/libreta pedido no existe.
    #[error("colección desconocida: {0}")]
    UnknownCollection(String),
    /// Un objeto (iCalendar/vCard) no se pudo parsear.
    #[error("parseo: {0}")]
    Parse(String),
}
