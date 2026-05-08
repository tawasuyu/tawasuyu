//! Tests del file watcher: el "puente humano" que convierte Minga en
//! un VCS de fondo — el usuario edita archivos y Minga los versiona
//! sin acción explícita.

use std::fs;
use std::time::Duration;

use minga_cli::{cmd_init, cmd_status, cmd_watch};
use tempfile::TempDir;

/// Espera hasta que el `cmd_status` reporte `expected` claves en MST,
/// o hasta `timeout`. Devuelve `true` si se alcanzó la cuenta.
async fn wait_until_mst_size(
    repo: &std::path::Path,
    pass: &str,
    expected: usize,
    timeout: Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(s) = cmd_status(repo, pass) {
            if s.mst_len >= expected {
                return true;
            }
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

#[tokio::test]
async fn watcher_initial_scan_picks_up_existing_files() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    let watch = dir.path().join("src");
    fs::create_dir(&watch).unwrap();
    cmd_init(&repo, "p").unwrap();

    // Escribimos archivos ANTES de arrancar el watcher.
    fs::write(watch.join("a.rs"), "fn a() -> i32 { 1 }").unwrap();
    fs::write(watch.join("b.rs"), "fn b() -> i32 { 2 }").unwrap();

    // Arrancamos el watcher en una task. La pasada inicial debería
    // ingerir ambos.
    let repo_clone = repo.clone();
    let handle = tokio::spawn(async move {
        let _ = cmd_watch(&repo_clone, "p", &watch).await;
    });

    // Damos margen para la pasada inicial. cmd_watch tiene el repo
    // abierto, pero cmd_status no puede mientras tanto (sled lock).
    // Solución: cancelamos el watcher antes de medir.
    tokio::time::sleep(Duration::from_millis(500)).await;
    handle.abort();
    let _ = handle.await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    let s = cmd_status(&repo, "p").unwrap();
    assert_eq!(s.mst_len, 2, "esperaba 2 funciones del initial scan");
    assert_eq!(s.attestations_len, 2);
}

#[tokio::test]
async fn watcher_ingests_new_file_after_creation() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    let watch = dir.path().join("src");
    fs::create_dir(&watch).unwrap();
    cmd_init(&repo, "p").unwrap();

    // Watcher arranca con directorio vacío.
    let repo_clone = repo.clone();
    let watch_clone = watch.clone();
    let handle = tokio::spawn(async move {
        let _ = cmd_watch(&repo_clone, "p", &watch_clone).await;
    });

    // Margen para que el watcher se inicialice y registre con notify.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Creamos un archivo. notify debería emitir un evento y el
    // watcher debería ingerirlo.
    fs::write(watch.join("new.rs"), "fn new() -> i32 { 42 }").unwrap();

    // Esperamos a que el evento se procese.
    tokio::time::sleep(Duration::from_millis(800)).await;

    // Detenemos el watcher para liberar el lock de sled antes de
    // hacer cmd_status.
    handle.abort();
    let _ = handle.await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Polling con timeout — algunos sistemas de archivos tienen
    // latencia de eventos.
    assert!(
        wait_until_mst_size(&repo, "p", 1, Duration::from_secs(3)).await,
        "el watcher no ingirió el archivo creado",
    );
}

#[tokio::test]
async fn watcher_ignores_non_rs_files() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    let watch = dir.path().join("src");
    fs::create_dir(&watch).unwrap();
    cmd_init(&repo, "p").unwrap();

    // Pre-poblamos con un .rs y varios archivos no-Rust.
    fs::write(watch.join("real.rs"), "fn real() -> i32 { 0 }").unwrap();
    fs::write(watch.join("readme.md"), "# proyecto").unwrap();
    fs::write(watch.join("data.json"), "{}").unwrap();

    let repo_clone = repo.clone();
    let handle = tokio::spawn(async move {
        let _ = cmd_watch(&repo_clone, "p", &watch).await;
    });

    tokio::time::sleep(Duration::from_millis(500)).await;
    handle.abort();
    let _ = handle.await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    let s = cmd_status(&repo, "p").unwrap();
    assert_eq!(s.mst_len, 1, "solo el .rs debe haberse ingerido");
}
