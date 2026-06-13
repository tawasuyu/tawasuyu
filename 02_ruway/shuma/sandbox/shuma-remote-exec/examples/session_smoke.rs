//! Smoke e2e de las sesiones persistentes (E4) contra un daemon vivo:
//! spawn → list → attach (lee scrollback) → kill. Requiere shuma-daemon
//! corriendo. `cargo run -p shuma-remote-exec --example session_smoke`
use shuma_exec::{CommandSpec, Exec};
fn main() {
    let sock = shuma_protocol::default_socket_path();
    let spec = CommandSpec {
        exec: Exec::Pty { program: "bash".into(), args: vec!["-lc".into(), "echo hola-e4; sleep 30".into()], cols: 80, rows: 24 },
        cwd: "/".into(), capture_limit: 0, spill_path: None, stdin_data: None, capture_stages: false,
    };
    let id = shuma_remote_exec::spawn_session_id(&spec, &sock, "smoke-e4").expect("spawn");
    println!("spawned: {id}");
    let sessions = shuma_remote_exec::list_sessions(&sock).expect("list");
    println!("list: {} sesiones; la nuestra viva={}", sessions.len(),
        sessions.iter().find(|s| s.session == id).map(|s| s.alive).unwrap_or(false));
    let mut h = shuma_remote_exec::attach_session(&sock, id, 24, 80).expect("attach");
    std::thread::sleep(std::time::Duration::from_millis(400));
    let evs = h.try_events();
    let bytes: Vec<u8> = evs.iter().flat_map(|e| if let shuma_exec::RunEvent::Bytes(b) = e { b.clone() } else { vec![] }).collect();
    println!("attach scrollback contiene 'hola-e4': {}", String::from_utf8_lossy(&bytes).contains("hola-e4"));
    drop(h); // detach (no mata)
    std::thread::sleep(std::time::Duration::from_millis(200));
    let still = shuma_remote_exec::list_sessions(&sock).expect("list2");
    println!("tras detach sigue viva: {}", still.iter().any(|s| s.session == id && s.alive));
    let killed = shuma_remote_exec::kill_session(&sock, id).expect("kill");
    println!("kill existed: {killed}");
}
