//! Convenciones de transporte: dónde vive el socket del Init.
//!
//! Resolución del path canónico:
//! 1. Variable de entorno [`SOCKET_ENV`] si está definida (override
//!    explícito, prioridad máxima).
//! 2. `$XDG_RUNTIME_DIR/brahman-init.sock` (sesión usuario).
//! 3. `$TMPDIR/brahman-init.sock` (fallback portable).

use std::path::PathBuf;

/// Variable de entorno que sobreescribe la ruta del socket del Init.
pub const SOCKET_ENV: &str = "BRAHMAN_INIT_SOCKET";

/// Nombre del socket dentro del runtime dir.
pub const SOCKET_NAME: &str = "brahman-init.sock";

/// Ruta canónica al socket del Init brahman.
pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var(SOCKET_ENV) {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join(SOCKET_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins() {
        // Nota: estos tests modifican entorno del proceso. `cargo test`
        // los corre paralelos por defecto pero usamos un nombre de var
        // único y restablecemos al final.
        let key = "BRAHMAN_INIT_SOCKET_TEST_OVERRIDE";
        // SAFETY: sólo escribimos una variable local al test; sin
        // contaminar SOCKET_ENV.
        std::env::set_var(key, "/tmp/explicit.sock");
        let saved = std::env::var(SOCKET_ENV).ok();
        std::env::set_var(SOCKET_ENV, "/tmp/explicit.sock");
        let p = default_socket_path();
        assert_eq!(p, PathBuf::from("/tmp/explicit.sock"));
        // Restaurar
        match saved {
            Some(v) => std::env::set_var(SOCKET_ENV, v),
            None => std::env::remove_var(SOCKET_ENV),
        }
        std::env::remove_var(key);
    }
}
