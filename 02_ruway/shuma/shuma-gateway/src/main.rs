//! `shuma-gateway` — adaptador HTTP/JSON + WebSocket para el daemon shuma.
//!
//! Endpoints:
//! - `POST /rpc`    : body JSON = `shuma_protocol::Request` → round-trip postcard
//!   contra el admin socket → `Response` como JSON. Una request por conexión
//!   (request/response 1:1). Sirve para WorkspaceList, Health, Stats, Run, etc.
//! - `GET /ws/pty`  : WebSocket full-duplex hacia una **sesión PTY
//!   persistente** del daemon. El primer mensaje (texto JSON) abre el
//!   puente: con `{"session":"<ulid>"}` se **adjunta** a una sesión
//!   existente; con `{"program":"claude","args":[...],"cwd":".","label":"…"}`
//!   **crea** una sesión persistente y se adjunta (antes de la salida manda
//!   `{"t":"session","id":"<ulid>"}` con el id, para re-adjuntarse luego).
//!   Después, los frames binarios del cliente son stdin (teclas) y los del
//!   servidor la salida cruda del terminal (empezando por el scrollback).
//!   Resize por `{"t":"resize","rows":R,"cols":C}`. **Cerrar el WS =
//!   DETACH**: la sesión sigue viva; se la mata con `PtyKill` (`POST /rpc`).
//! - `GET /term`    : cliente móvil de terminal (HTML autocontenido con
//!   xterm.js). Lista sesiones por `/rpc`, adjunta por `/ws/pty`. El token va
//!   en `?token=…`. Pensado para abrir desde un teléfono en la misma red.
//! - `GET /` y `GET /health` : healthcheck.
//!
//! Auth opcional por token (`SHIPOTE_GATEWAY_TOKEN`): header
//! `Authorization: Bearer <token>` o, para clientes WS que no fijan headers,
//! `?token=<token>` en la URL. Sin token configurado, el gateway queda abierto
//! (pensado para escucharse en loopback o detrás de un túnel).
//!
//! Pensado para clientes no-Rust (app Android, web, curl) que no hablan postcard.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response as AxumResponse},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use shuma_protocol::{default_socket_path, read_frame, write_frame, Request, Response};
use tokio::net::UnixStream;
use tokio::sync::broadcast;
use tracing::{info, warn};
use ulid::Ulid;

const DEFAULT_LISTEN: &str = "127.0.0.1:7378";

/// Cliente móvil de terminal (E4): página autocontenida servida en `/term`.
/// Adjunta a sesiones PTY persistentes vía `/rpc` + `/ws/pty`. El token (si
/// el gateway lo exige) va en `?token=…` de la URL.
const TERM_HTML: &str = include_str!("term.html");

#[derive(Clone)]
struct AppState {
    sock: Arc<PathBuf>,
    token: Option<Arc<String>>,
    /// Bus de eventos de supervisión: los hooks de Claude Code los publican
    /// por `POST /event` y los clientes (consola) los reciben por
    /// `GET /ws/events`. El gateway solo retransmite el JSON tal cual.
    events: broadcast::Sender<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let listen = std::env::var("SHIPOTE_GATEWAY_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN.into());
    let token = std::env::var("SHIPOTE_GATEWAY_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
        .map(Arc::new);

    let (events, _) = broadcast::channel::<String>(256);
    let state = AppState {
        sock: Arc::new(default_socket_path()),
        token,
        events,
    };

    let app = Router::new()
        .route("/", get(health))
        .route("/health", get(health))
        .route("/term", get(term_page))
        .route("/rpc", post(rpc))
        .route("/ws/pty", get(ws_pty))
        .route("/event", post(post_event))
        .route("/ws/events", get(ws_events))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(&listen).await?;
    info!(
        listen = %listen,
        daemon = %state.sock.display(),
        auth = state.token.is_some(),
        "shuma-gateway listening"
    );
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "shuma-gateway ok\n"
}

/// GET /term — el cliente móvil de terminal. HTML estático (sin secretos): el
/// token va por la URL y lo usa el JS para `/rpc` y `/ws/pty`, que sí están
/// gateados. Por eso la página en sí no requiere auth para cargar.
async fn term_page() -> Html<&'static str> {
    Html(TERM_HTML)
}

// =====================================================================
// Auth
// =====================================================================

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

fn authorized(state: &AppState, headers: &HeaderMap, query_token: Option<&str>) -> bool {
    let Some(expected) = state.token.as_deref() else {
        return true; // sin token configurado = abierto
    };
    if let Some(bearer) = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
    {
        if ct_eq(bearer.trim(), expected) {
            return true;
        }
    }
    matches!(query_token, Some(t) if ct_eq(t, expected))
}

/// Comparación en tiempo constante para no filtrar el token por timing.
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// =====================================================================
// POST /rpc — una Request JSON → una Response JSON
// =====================================================================

async fn rpc(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> AxumResponse {
    if !authorized(&state, &headers, None) {
        return (StatusCode::UNAUTHORIZED, Json(err("unauthorized"))).into_response();
    }
    let req: Request = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(err(&format!("bad json: {e}")))).into_response()
        }
    };
    match round_trip(&state.sock, &req).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(err(&format!("daemon: {e}")))).into_response(),
    }
}

fn err(msg: &str) -> serde_json::Value {
    serde_json::json!({ "error": msg })
}

async fn round_trip(sock: &std::path::Path, req: &Request) -> anyhow::Result<Response> {
    let mut stream = UnixStream::connect(sock).await?;
    write_frame(&mut stream, req).await?;
    let resp: Response = read_frame(&mut stream).await?;
    Ok(resp)
}

// =====================================================================
// GET /ws/pty — WebSocket ↔ subprotocolo ExecPty del daemon
// =====================================================================

/// Primer mensaje del cliente WS (texto JSON): abre el puente a una
/// sesión. Con `session` se adjunta a una existente; con `program` crea
/// una nueva y se adjunta.
#[derive(Deserialize)]
struct PtyOpen {
    /// Id (ULID) de una sesión existente a la que adjuntarse.
    #[serde(default)]
    session: Option<String>,
    /// Programa a lanzar si se crea una sesión nueva (ignorado si hay
    /// `session`).
    #[serde(default)]
    program: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "default_cwd")]
    cwd: String,
    /// Etiqueta legible para la sesión nueva.
    #[serde(default)]
    label: String,
    #[serde(default = "default_rows")]
    rows: u16,
    #[serde(default = "default_cols")]
    cols: u16,
}

fn default_cwd() -> String {
    ".".into()
}
fn default_rows() -> u16 {
    24
}
fn default_cols() -> u16 {
    80
}

/// Mensaje de control (texto JSON) durante un PTY activo.
#[derive(Deserialize)]
struct PtyControl {
    t: String,
    rows: Option<u16>,
    cols: Option<u16>,
}

async fn ws_pty(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    ws: WebSocketUpgrade,
) -> AxumResponse {
    if !authorized(&state, &headers, q.token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let sock = state.sock.clone();
    ws.on_upgrade(move |socket| pty_bridge(socket, sock))
}

// =====================================================================
// Bus de eventos de supervisión (hooks de Claude Code → consola)
// =====================================================================

/// POST /event — un hook publica un evento (JSON arbitrario) que se
/// retransmite tal cual a los clientes de `/ws/events`. Pensado para los
/// hooks `Notification`/`Stop` de Claude Code, que corren en localhost.
async fn post_event(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> AxumResponse {
    if !authorized(&state, &headers, None) {
        return (StatusCode::UNAUTHORIZED, Json(err("unauthorized"))).into_response();
    }
    let payload = match String::from_utf8(body.to_vec()) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return (StatusCode::BAD_REQUEST, Json(err("evento vacío o no-UTF8"))).into_response(),
    };
    // send() falla solo si no hay suscriptores (teléfono desconectado): no es
    // un error, simplemente nadie escucha ahora mismo.
    let subscribers = state.events.send(payload).unwrap_or(0);
    (StatusCode::OK, Json(serde_json::json!({ "ok": true, "subscribers": subscribers }))).into_response()
}

/// GET /ws/events — el cliente se suscribe y recibe cada evento como texto.
async fn ws_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    ws: WebSocketUpgrade,
) -> AxumResponse {
    if !authorized(&state, &headers, q.token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let rx = state.events.subscribe();
    ws.on_upgrade(move |socket| events_bridge(socket, rx))
}

async fn events_bridge(mut ws: WebSocket, mut rx: broadcast::Receiver<String>) {
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(s) => {
                    if ws.send(Message::Text(s)).await.is_err() {
                        break;
                    }
                }
                // Si el cliente se retrasa y pierde eventos, seguimos con los
                // siguientes en vez de cortar la conexión.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            inbound = ws.recv() => match inbound {
                // Ignoramos lo que mande el cliente (keepalive); solo nos
                // importa detectar el cierre/caída para soltar la suscripción.
                Some(Ok(_)) => continue,
                _ => break,
            },
        }
    }
}

async fn pty_bridge(mut ws: WebSocket, sock: Arc<PathBuf>) {
    // 1) Primer mensaje = spec de apertura (texto JSON).
    let open: PtyOpen = loop {
        match ws.recv().await {
            Some(Ok(Message::Text(t))) => {
                let v: serde_json::Value = match serde_json::from_str(&t) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = ws.send(Message::Text(ctl_err(&format!("bad open: {e}")))).await;
                        return;
                    }
                };
                // Un cliente puede encolar un control (p.ej. {"t":"resize"})
                // antes del open por una carrera de UI (el layout dispara el
                // resize antes de que onOpen mande el open). Esos frames llevan
                // `t`; ignóralos y sigue esperando el verdadero open.
                if v.get("t").is_some() {
                    continue;
                }
                match serde_json::from_value(v) {
                    Ok(o) => break o,
                    Err(e) => {
                        let _ = ws.send(Message::Text(ctl_err(&format!("bad open: {e}")))).await;
                        return;
                    }
                }
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return, // cerró o mandó binario antes de abrir
        }
    };

    // 2) Conectar al daemon.
    let daemon = match UnixStream::connect(&*sock).await {
        Ok(s) => s,
        Err(e) => {
            let _ = ws.send(Message::Text(ctl_err(&format!("daemon: {e}")))).await;
            return;
        }
    };
    let (mut drd, mut dwr) = daemon.into_split();

    // 3) Resolver la sesión: adjuntar a una existente, o crear una nueva
    //    (PtySpawn 1:1 sobre la misma conexión) y luego adjuntar.
    let session: Ulid = if let Some(s) = open.session.as_deref() {
        match Ulid::from_string(s) {
            Ok(id) => id,
            Err(_) => {
                let _ = ws.send(Message::Text(ctl_err("session id inválido"))).await;
                return;
            }
        }
    } else if let Some(program) = open.program.clone() {
        let spawn = Request::PtySpawn {
            cwd: open.cwd.clone(),
            program,
            args: open.args.clone(),
            rows: open.rows,
            cols: open.cols,
            label: open.label.clone(),
        };
        if write_frame(&mut dwr, &spawn).await.is_err() {
            let _ = ws.send(Message::Text(ctl_err("write spawn failed"))).await;
            return;
        }
        match read_frame::<Response, _>(&mut drd).await {
            Ok(Response::PtySpawned { session }) => {
                // El cliente aprende el id para poder re-adjuntarse luego.
                let _ = ws
                    .send(Message::Text(
                        serde_json::json!({"t":"session","id":session.to_string()}).to_string(),
                    ))
                    .await;
                session
            }
            Ok(Response::Error { message }) => {
                let _ = ws.send(Message::Text(ctl_err(&message))).await;
                return;
            }
            Ok(other) => {
                let _ = ws
                    .send(Message::Text(ctl_err(&format!(
                        "respuesta inesperada al spawn: {other:?}"
                    ))))
                    .await;
                return;
            }
            Err(e) => {
                let _ = ws.send(Message::Text(ctl_err(&format!("daemon: {e}")))).await;
                return;
            }
        }
    } else {
        let _ = ws
            .send(Message::Text(ctl_err("falta 'session' o 'program'")))
            .await;
        return;
    };

    // 4) Adjuntarse a la sesión. A partir de aquí la conexión es
    //    full-duplex; cerrar el WS = DETACH (la sesión sobrevive).
    let attach = Request::PtyAttach {
        session,
        rows: open.rows,
        cols: open.cols,
    };
    if write_frame(&mut dwr, &attach).await.is_err() {
        let _ = ws.send(Message::Text(ctl_err("write attach failed"))).await;
        return;
    }

    // 5) Puente full-duplex. tokio::select! suelta la rama no completada
    //    antes de correr el handler, así `ws` se puede usar en ambas ramas.
    loop {
        tokio::select! {
            frame = read_frame::<Response, _>(&mut drd) => {
                match frame {
                    Ok(Response::ExecBytes(b)) => {
                        if ws.send(Message::Binary(b)).await.is_err() {
                            break;
                        }
                    }
                    Ok(Response::ExecExited(code)) => {
                        let _ = ws.send(Message::Text(format!("{{\"t\":\"exit\",\"code\":{code}}}"))).await;
                        break;
                    }
                    Ok(Response::ExecFailed(m)) => {
                        let _ = ws.send(Message::Text(ctl_err(&m))).await;
                        break;
                    }
                    Ok(_) => {} // otros frames no aplican al PTY
                    Err(_) => break, // daemon cerró
                }
            }
            msg = ws.recv() => {
                match msg {
                    Some(Ok(Message::Binary(bytes))) => {
                        if write_frame(&mut dwr, &Request::PtyInput { bytes }).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(t))) => {
                        if let Ok(c) = serde_json::from_str::<PtyControl>(&t) {
                            if c.t == "resize" {
                                if let (Some(rows), Some(cols)) = (c.rows, c.cols) {
                                    let _ = write_frame(&mut dwr, &Request::PtyResize { rows, cols }).await;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // ping/pong: axum responde solo
                    Some(Err(_)) => break,
                }
            }
        }
    }
    // Al salir, drd/dwr se dropean → el daemon ve EOF → mata el PTY (convención SSH).
    warn!("pty bridge closed");
}

/// Mensaje de control de error hacia el cliente WS (JSON con string escapado).
fn ctl_err(msg: &str) -> String {
    serde_json::json!({ "t": "error", "msg": msg }).to_string()
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter =
        EnvFilter::try_from_env("SHIPOTE_GATEWAY_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_matches_and_rejects() {
        assert!(ct_eq("s3cr3t", "s3cr3t"));
        assert!(!ct_eq("s3cr3t", "s3cr3T"));
        assert!(!ct_eq("short", "longer"));
    }

    fn test_state(token: Option<&str>) -> AppState {
        let (events, _) = broadcast::channel::<String>(8);
        AppState {
            sock: Arc::new(PathBuf::from("/x")),
            token: token.map(|t| Arc::new(t.to_string())),
            events,
        }
    }

    #[test]
    fn auth_open_when_no_token() {
        let st = test_state(None);
        assert!(authorized(&st, &HeaderMap::new(), None));
    }

    #[test]
    fn auth_requires_token_when_set() {
        let st = test_state(Some("abc"));
        assert!(!authorized(&st, &HeaderMap::new(), None));
        assert!(authorized(&st, &HeaderMap::new(), Some("abc")));
        assert!(!authorized(&st, &HeaderMap::new(), Some("nope")));
    }

    #[test]
    fn term_html_habla_el_protocolo_del_gateway() {
        // El cliente embebido debe cablear los endpoints/protocolo reales:
        // listar por /rpc "PtyList", adjuntar por /ws/pty, resize/kill.
        assert!(TERM_HTML.contains("/ws/pty"));
        assert!(TERM_HTML.contains("\"PtyList\""));
        assert!(TERM_HTML.contains("PtyKill"));
        assert!(TERM_HTML.contains("\"resize\""));
        // Y monta un terminal de verdad (xterm) + pasa el token a las dos vías.
        assert!(TERM_HTML.contains("xterm"));
        assert!(TERM_HTML.contains("Bearer "));
        assert!(TERM_HTML.contains("token="));
    }

    #[tokio::test]
    async fn term_page_devuelve_html_sin_auth() {
        // La página carga sin token (no tiene secretos); el gateo está en
        // /rpc y /ws/pty, que el JS llama con el token de la URL.
        let Html(body) = term_page().await;
        assert!(body.contains("<!DOCTYPE html>"));
        assert_eq!(body, TERM_HTML);
    }

    #[test]
    fn pty_open_spawn_parses_with_defaults() {
        let o: PtyOpen = serde_json::from_str(r#"{"program":"claude","args":["code"]}"#).unwrap();
        assert_eq!(o.program.as_deref(), Some("claude"));
        assert_eq!(o.args, vec!["code"]);
        assert_eq!(o.session, None);
        assert_eq!(o.rows, 24);
        assert_eq!(o.cols, 80);
        assert_eq!(o.cwd, ".");
        assert_eq!(o.label, "");
    }

    #[test]
    fn pty_open_attach_parses() {
        let o: PtyOpen =
            serde_json::from_str(r#"{"session":"01ARZ3NDEKTSV4RRFFQ69G5FAV","rows":40}"#).unwrap();
        assert_eq!(o.session.as_deref(), Some("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert_eq!(o.program, None);
        assert_eq!(o.rows, 40);
        assert_eq!(o.cols, 80);
    }

    #[test]
    fn pty_control_resize_parses() {
        let c: PtyControl = serde_json::from_str(r#"{"t":"resize","rows":40,"cols":120}"#).unwrap();
        assert_eq!(c.t, "resize");
        assert_eq!(c.rows, Some(40));
        assert_eq!(c.cols, Some(120));
    }
}
