//! Pruebas de integración: un daemon de voz real sobre socket Unix + clientes.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use rimay_voz_core::{Audio, Locutor, Transcriptor};
use rimay_voz_daemon::{Daemon, DaemonClient};
use rimay_voz_mock::{LocutorMock, TranscriptorMock};

/// Ruta de socket única por test — evita choques entre tests paralelos.
fn socket_unico() -> std::path::PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("voz-d-{}-{n}.sock", std::process::id()))
}

/// Levanta un daemon sirviendo mocks y devuelve su ruta + el handle de la task.
/// El STT mock reconoce un `texto` configurable.
fn spawn_daemon(texto: &str) -> (std::path::PathBuf, tokio::task::JoinHandle<()>) {
    let path = socket_unico();
    let daemon = Daemon::bind(&path).expect("bind");
    let stt: Arc<dyn Transcriptor> = Arc::new(TranscriptorMock::con_texto(texto));
    let tts: Arc<dyn Locutor> = Arc::new(LocutorMock);
    let handle = tokio::spawn(async move {
        let _ = daemon.serve(stt, tts).await;
    });
    (path, handle)
}

#[tokio::test]
async fn stt_sobre_socket_coincide_con_el_directo() {
    let (path, handle) = spawn_daemon("shuma abrí cosmos");
    let client = DaemonClient::connect(&path).await.expect("connect");

    let audio = Audio::new(vec![0; 16_000], 16_000);
    let por_socket = Transcriptor::transcribir(&client, &audio).await.unwrap();
    let directo = TranscriptorMock::con_texto("shuma abrí cosmos")
        .transcribir(&audio)
        .await
        .unwrap();

    // El daemon no debe alterar el texto: igual al directo.
    assert_eq!(por_socket.texto, directo.texto);
    assert_eq!(por_socket.texto, "shuma abrí cosmos");

    handle.abort();
}

#[tokio::test]
async fn tts_sobre_socket_devuelve_audio() {
    let (path, handle) = spawn_daemon("shuma");
    let client = DaemonClient::connect(&path).await.expect("connect");

    let audio = Locutor::sintetizar(&client, "hola mundo").await.unwrap();
    // El mock sintetiza silencio proporcional al texto, a 22 050 Hz.
    assert!(audio.muestras.len() > 0);
    assert_eq!(audio.hz, 22_050);

    let corto = Locutor::sintetizar(&client, "hi").await.unwrap();
    assert!(audio.muestras.len() > corto.muestras.len());

    handle.abort();
}

#[tokio::test]
async fn handshake_reporta_etiquetas_de_ambos_modelos() {
    let (path, handle) = spawn_daemon("shuma");
    let client = DaemonClient::connect(&path).await.expect("connect");
    assert_eq!(Transcriptor::modelo(&client), "mock-stt");
    assert_eq!(Locutor::modelo(&client), "mock-tts");
    handle.abort();
}

#[tokio::test]
async fn varias_llamadas_sobre_un_cliente() {
    // El cliente hace round-trip por llamada: varias seguidas deben funcionar
    // sin estado corrupto.
    let (path, handle) = spawn_daemon("shuma");
    let client = DaemonClient::connect(&path).await.expect("connect");
    let audio = Audio::new(vec![0; 800], 16_000);
    for _ in 0..4 {
        let t = Transcriptor::transcribir(&client, &audio).await.unwrap();
        assert_eq!(t.texto, "shuma");
    }
    let _ = Locutor::sintetizar(&client, "intercalado").await.unwrap();
    let otra = Transcriptor::transcribir(&client, &audio).await.unwrap();
    assert_eq!(otra.texto, "shuma");
    handle.abort();
}

#[tokio::test]
async fn dos_clientes_comparten_un_daemon() {
    let (path, handle) = spawn_daemon("compartido");
    let a = DaemonClient::connect(&path).await.expect("connect a");
    let b = DaemonClient::connect(&path).await.expect("connect b");

    let audio = Audio::new(vec![1; 400], 16_000);
    let va = Transcriptor::transcribir(&a, &audio).await.unwrap();
    let vb = Transcriptor::transcribir(&b, &audio).await.unwrap();
    assert_eq!(va.texto, vb.texto);

    handle.abort();
}

#[tokio::test]
async fn conectar_a_daemon_ausente_erra() {
    let path = socket_unico(); // nunca se bindeó
    let res = DaemonClient::connect(&path).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn ping_reporta_vida() {
    let (path, handle) = spawn_daemon("shuma");
    let client = DaemonClient::connect(&path).await.expect("connect");
    client.ping().await.expect("pong");
    handle.abort();
}

#[tokio::test]
async fn shutdown_clausura_el_loop_sin_panic() {
    let path = socket_unico();
    let daemon = Daemon::bind(&path).expect("bind");
    let stt: Arc<dyn Transcriptor> = Arc::new(TranscriptorMock::default());
    let tts: Arc<dyn Locutor> = Arc::new(LocutorMock);
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        daemon
            .serve_with_shutdown(stt, tts, async move {
                let _ = rx.await;
            })
            .await
            .expect("serve_with_shutdown")
    });

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let client = DaemonClient::connect(&path).await.expect("connect");
    client.ping().await.expect("pong");

    tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("el daemon no salió del loop en 2s")
        .expect("la task panificó");
}
