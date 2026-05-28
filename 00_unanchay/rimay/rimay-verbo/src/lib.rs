//! `rimay-verbo` — la fachada que los consumidores de embeddings importan
//! cuando no quieren reinventar la convención del socket ni decidir entre
//! "daemon real" y "mock determinista" en cada call site.
//!
//! Los crates plugin (`rimay-verbo-core`, `rimay-verbo-mock`,
//! `rimay-verbo-daemon`, `rimay-verbo-fastembed`) siguen siendo accesibles
//! por separado para quien necesita ensamblar a mano. Esta fachada cubre
//! el caso del 90 %:
//!
//! ```no_run
//! use std::sync::Arc;
//! use rimay_verbo::Provider;
//!
//! # async fn ejemplo() -> Result<(), Box<dyn std::error::Error>> {
//! // Si hay un verbo-daemon corriendo, lo usa; si no, MockProvider 384d.
//! let provider: Arc<dyn Provider> = rimay_verbo::conectar_o_mock(384).await;
//! let v = provider.embed("hola").await?;
//! assert_eq!(v.values.len(), 384);
//! # Ok(()) }
//! ```
//!
//! La convención del socket por defecto es la misma que usa el binario
//! `verbo-daemon`: `$XDG_RUNTIME_DIR/verbo.sock`, con fallback a
//! `/tmp/verbo-{uid}.sock` cuando esa variable no está. Un cambio aquí
//! debe espejarse en `rimay-verbo-daemon-bin::socket_por_defecto` — son
//! la misma convención partida en dos archivos por dependencia, no dos
//! decisiones independientes.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use rimay_verbo_core::{EmbedError, EmbeddingVector, ModelId, Provider};
pub use rimay_verbo_daemon::DaemonClient;
pub use rimay_verbo_mock::MockProvider;

/// Ruta del socket Unix donde el `verbo-daemon` escucha por defecto.
///
/// Convención de la suite (idéntica a la del binario `verbo-daemon`):
/// 1. `$XDG_RUNTIME_DIR/verbo.sock` si la variable está definida.
/// 2. Fallback: `/tmp/verbo-{uid}.sock` — prefijado por UID del usuario
///    para no chocar en sistemas multi-usuario.
pub fn socket_por_defecto() -> PathBuf {
    socket_desde(std::env::var("XDG_RUNTIME_DIR").ok().as_deref(), uid_actual())
}

/// Lógica pura de la convención del socket, separada del acceso a env
/// para que los tests no tengan que tocar variables globales.
fn socket_desde(xdg_runtime_dir: Option<&str>, uid: u32) -> PathBuf {
    if let Some(xdg) = xdg_runtime_dir {
        return PathBuf::from(xdg).join("verbo.sock");
    }
    PathBuf::from(format!("/tmp/verbo-{uid}.sock"))
}

/// UID del proceso vía `/proc/self/loginuid`. Si falla, devuelve 1000 —
/// el socket sigue siendo único por usuario en la práctica (idéntica
/// heurística a la del binario, ver `verbo-daemon-bin::libc_uid`).
fn uid_actual() -> u32 {
    std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&u| u != u32::MAX)
        .unwrap_or(1000)
}

/// Conecta al `verbo-daemon` en el socket por defecto. Falla si no hay
/// daemon corriendo. Para el caso "úsalo si está, si no mock", ver
/// [`conectar_o_mock`].
pub async fn conectar() -> Result<DaemonClient, EmbedError> {
    DaemonClient::connect(socket_por_defecto()).await
}

/// Conecta al `verbo-daemon` en una ruta arbitraria. Útil cuando se
/// corre más de un daemon (un modelo distinto por socket).
pub async fn conectar_en(socket: impl AsRef<Path>) -> Result<DaemonClient, EmbedError> {
    DaemonClient::connect(socket).await
}

/// Conecta al `verbo-daemon` si está corriendo; si no, devuelve un
/// `MockProvider` determinista de `dim` dimensiones. Esta es la forma
/// recomendada de empezar a consumir embeddings en un crate que aún no
/// tiene un daemon corriendo en CI ni en máquinas frescas.
///
/// **Importante**: los vectores del daemon y del mock no son
/// intercambiables — el `ModelId` los etiqueta. Un consumidor que
/// guarde embeddings a disco debe persistir también el `ModelId` y
/// rechazar comparaciones cruzadas; `EmbeddingVector::cosine` ya
/// retorna `EmbedError::ModelMismatch` en ese caso.
pub async fn conectar_o_mock(dim: usize) -> Arc<dyn Provider> {
    conectar_o_mock_en(&socket_por_defecto(), dim).await
}

/// Variante explícita de [`conectar_o_mock`] que toma el socket. Útil
/// para tests, para apps que conocen su propio path, o para soportar
/// multi-daemon (cada modelo en su socket).
pub async fn conectar_o_mock_en(socket: &Path, dim: usize) -> Arc<dyn Provider> {
    match DaemonClient::connect(socket).await {
        Ok(cliente) => Arc::new(cliente),
        Err(_) => Arc::new(MockProvider::new(dim)),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn socket_usa_xdg_runtime_dir_cuando_existe() {
        let s = socket_desde(Some("/run/user/1234"), 1234);
        assert_eq!(s, PathBuf::from("/run/user/1234/verbo.sock"));
    }

    #[test]
    fn socket_cae_a_tmp_por_uid_sin_xdg() {
        let s = socket_desde(None, 42);
        assert_eq!(s, PathBuf::from("/tmp/verbo-42.sock"));
    }

    #[tokio::test]
    async fn conectar_o_mock_cae_a_mock_sin_daemon() {
        // Apuntamos a un socket que con toda seguridad no existe — la
        // fachada debería devolver el mock sin propagar el error.
        let path = PathBuf::from("/tmp/rimay-verbo-no-existe-xyzzy.sock");
        let p = conectar_o_mock_en(&path, 64).await;
        assert_eq!(p.model_id().dimension, 64);
        let v = p.embed("hola").await.unwrap();
        assert_eq!(v.values.len(), 64);
    }
}
