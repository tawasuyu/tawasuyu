//! Pruebas de integración: un daemon real sobre socket Unix + clientes.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use verbo_core::Provider;
use verbo_daemon::{Daemon, DaemonClient};
use verbo_mock::MockProvider;

/// Ruta de socket única por test — evita choques entre tests paralelos.
fn unique_socket() -> std::path::PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("verbo-d-{}-{n}.sock", std::process::id()))
}

/// Levanta un daemon sirviendo un `MockProvider` y devuelve su ruta + el
/// handle de la task (para abortarla al final).
fn spawn_daemon(dim: usize) -> (std::path::PathBuf, tokio::task::JoinHandle<()>) {
    let path = unique_socket();
    let daemon = Daemon::bind(&path).expect("bind");
    let provider = Arc::new(MockProvider::new(dim));
    let handle = tokio::spawn(async move {
        let _ = daemon.serve(provider).await;
    });
    (path, handle)
}

#[tokio::test]
async fn client_embed_matches_direct_provider() {
    let (path, handle) = spawn_daemon(32);
    let client = DaemonClient::connect(&path).await.expect("connect");

    let over_socket = client.embed("texto de prueba").await.unwrap();
    let direct = MockProvider::new(32).embed("texto de prueba").await.unwrap();

    // El daemon no debe alterar el vector: byte a byte igual al directo.
    assert_eq!(over_socket.values, direct.values);
    assert_eq!(over_socket.model, direct.model);

    handle.abort();
}

#[tokio::test]
async fn handshake_reports_model_id() {
    let (path, handle) = spawn_daemon(384);
    let client = DaemonClient::connect(&path).await.expect("connect");
    assert_eq!(client.model_id().dimension, 384);
    handle.abort();
}

#[tokio::test]
async fn batch_over_socket_matches_individual() {
    let (path, handle) = spawn_daemon(16);
    let client = DaemonClient::connect(&path).await.expect("connect");

    let texts = vec!["uno".to_string(), "dos".to_string(), "tres".to_string()];
    let batch = client.embed_batch(&texts).await.unwrap();
    assert_eq!(batch.len(), 3);

    let single = client.embed("dos").await.unwrap();
    assert_eq!(batch[1].values, single.values);

    handle.abort();
}

#[tokio::test]
async fn many_requests_on_one_client() {
    // El cliente hace round-trip por llamada: varias llamadas seguidas
    // sobre el mismo cliente deben funcionar sin estado corrupto.
    let (path, handle) = spawn_daemon(8);
    let client = DaemonClient::connect(&path).await.expect("connect");
    for word in ["a", "bb", "ccc", "a"] {
        let v = client.embed(word).await.unwrap();
        assert_eq!(v.values.len(), 8);
    }
    // Mismo texto → mismo vector incluso tras otras llamadas.
    let first = client.embed("a").await.unwrap();
    let again = client.embed("a").await.unwrap();
    assert_eq!(first.values, again.values);
    handle.abort();
}

#[tokio::test]
async fn two_clients_share_one_daemon() {
    let (path, handle) = spawn_daemon(24);
    let a = DaemonClient::connect(&path).await.expect("connect a");
    let b = DaemonClient::connect(&path).await.expect("connect b");

    let va = a.embed("compartido").await.unwrap();
    let vb = b.embed("compartido").await.unwrap();
    // Dos procesos, un modelo: el mismo texto da el mismo vector.
    assert_eq!(va.values, vb.values);

    handle.abort();
}

#[tokio::test]
async fn connect_to_missing_daemon_errors() {
    let path = unique_socket(); // nunca se bindeó
    let result = DaemonClient::connect(&path).await;
    assert!(result.is_err());
}
