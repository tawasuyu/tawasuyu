//! Convenciones de transporte para el socket admin.

use std::path::PathBuf;

/// Variable de entorno que sobreescribe la ruta del socket admin.
pub const SOCKET_ENV: &str = "BRAHMAN_ADMIN_SOCKET";

/// Nombre del socket admin dentro del runtime dir.
pub const SOCKET_NAME: &str = "brahman-admin.sock";

/// Ruta canónica al socket admin del Init.
pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var(SOCKET_ENV) {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join(SOCKET_NAME)
}
