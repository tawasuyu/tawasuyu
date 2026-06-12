//! End-to-end de las sesiones PTY persistentes.
//!
//! Arranca el binario real del daemon en un `XDG_RUNTIME_DIR` temporal
//! (socket aislado) y, por el socket Unix, ejercita el ciclo completo:
//! spawn → attach → escribir → DETACH → re-attach (el scrollback debe
//! sobrevivir) → list → kill. Es la prueba de que la sesión vive
//! desacoplada de la conexión.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use shuma_protocol::{read_frame, write_frame, Request, Response};
use tokio::net::UnixStream;
use ulid::Ulid;

/// Mata el daemon al terminar el test, pase lo que pase.
struct DaemonGuard {
    child: Child,
}
impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn connect(sock: &Path) -> UnixStream {
    for _ in 0..200 {
        if let Ok(s) = UnixStream::connect(sock).await {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("el socket del daemon nunca apareció en {sock:?}");
}

/// Drena frames hasta encontrar `needle` en la salida acumulada, o hasta
/// agotar el timeout / ver el exit.
async fn read_until_contains(s: &mut UnixStream, needle: &[u8], timeout: Duration) -> bool {
    let mut acc: Vec<u8> = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match tokio::time::timeout(remaining, read_frame::<Response, _>(s)).await {
            Ok(Ok(Response::ExecBytes(b))) => {
                acc.extend_from_slice(&b);
                if acc.windows(needle.len()).any(|w| w == needle) {
                    return true;
                }
            }
            Ok(Ok(Response::ExecExited(_))) => {
                return acc.windows(needle.len()).any(|w| w == needle);
            }
            Ok(Ok(_)) => {}      // otros frames: ignorar
            Ok(Err(_)) => return false, // conexión cerrada
            Err(_) => return false,     // timeout
        }
    }
}

#[tokio::test]
async fn pty_session_persists_across_detach() {
    let cat = ["/bin/cat", "/usr/bin/cat"]
        .into_iter()
        .find(|p| Path::new(p).exists())
        .expect("se necesita `cat` para el test");

    // Aislamiento: directorio runtime/estado propio del test.
    let dir: PathBuf = std::env::temp_dir().join(format!("shuma-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("shuma.sock");

    let bin = env!("CARGO_BIN_EXE_shuma-daemon");
    let child = Command::new(bin)
        .env("XDG_RUNTIME_DIR", &dir)
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .spawn()
        .expect("arrancar shuma-daemon");
    let _guard = DaemonGuard { child };

    // 1) Crear una sesión `cat` (vive y hace eco por el PTY).
    let mut c = connect(&sock).await;
    write_frame(
        &mut c,
        &Request::PtySpawn {
            cwd: ".".into(),
            program: cat.into(),
            args: vec![],
            rows: 24,
            cols: 80,
            label: "e2e".into(),
        },
    )
    .await
    .unwrap();
    let session: Ulid = match read_frame::<Response, _>(&mut c).await.unwrap() {
        Response::PtySpawned { session } => session,
        other => panic!("esperaba PtySpawned, vino {other:?}"),
    };
    drop(c);

    // 2) Attach #1: escribe una marca y confirma el eco en vivo.
    let mut a1 = connect(&sock).await;
    write_frame(
        &mut a1,
        &Request::PtyAttach { session, rows: 24, cols: 80 },
    )
    .await
    .unwrap();
    write_frame(
        &mut a1,
        &Request::PtyInput { bytes: b"marca-uno\n".to_vec() },
    )
    .await
    .unwrap();
    assert!(
        read_until_contains(&mut a1, b"marca-uno", Duration::from_secs(5)).await,
        "attach #1 no recibió el eco de marca-uno"
    );
    drop(a1); // DETACH — NO debe matar la sesión.

    // 3) Attach #2 (conexión nueva): el scrollback debe traer la marca
    //    previa → la sesión sobrevivió al detach.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut a2 = connect(&sock).await;
    write_frame(
        &mut a2,
        &Request::PtyAttach { session, rows: 24, cols: 80 },
    )
    .await
    .unwrap();
    assert!(
        read_until_contains(&mut a2, b"marca-uno", Duration::from_secs(5)).await,
        "attach #2 no vio el scrollback persistido tras el detach"
    );
    drop(a2);

    // 4) List: la sesión sigue viva y con su etiqueta.
    let mut l = connect(&sock).await;
    write_frame(&mut l, &Request::PtyList).await.unwrap();
    match read_frame::<Response, _>(&mut l).await.unwrap() {
        Response::PtyList { sessions } => {
            let s = sessions
                .iter()
                .find(|s| s.session == session)
                .expect("la sesión debe estar en la lista");
            assert!(s.alive, "la sesión debería estar viva");
            assert_eq!(s.label, "e2e");
            assert_eq!(s.program, cat);
        }
        other => panic!("esperaba PtyList, vino {other:?}"),
    }
    drop(l);

    // 5) Kill: existed=true y deja de aparecer en la lista.
    let mut k = connect(&sock).await;
    write_frame(&mut k, &Request::PtyKill { session }).await.unwrap();
    match read_frame::<Response, _>(&mut k).await.unwrap() {
        Response::PtyKilled { existed, session: s } => {
            assert!(existed, "la sesión debía existir");
            assert_eq!(s, session);
        }
        other => panic!("esperaba PtyKilled, vino {other:?}"),
    }
    write_frame(&mut k, &Request::PtyList).await.unwrap();
    match read_frame::<Response, _>(&mut k).await.unwrap() {
        Response::PtyList { sessions } => {
            assert!(
                !sessions.iter().any(|s| s.session == session),
                "la sesión no debe seguir listada tras el kill"
            );
        }
        other => panic!("esperaba PtyList, vino {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}
