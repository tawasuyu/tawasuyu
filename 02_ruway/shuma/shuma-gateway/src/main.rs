//! `shuma-gateway` — HTTP/JSON adapter para el daemon.
//!
//! Acepta `POST /rpc` con body JSON serializado como `shuma_protocol::Request`,
//! hace round-trip al admin socket via postcard, devuelve `Response` como JSON.
//!
//! Diseñado para clients no-Rust (curl, Python, web app) que no pueden
//! hablar postcard directo. NO es un proxy completo — sólo translation
//! layer del protocolo.
//!
//! Sin dep de axum/hyper: HTTP parser ad-hoc, suficiente para 1 endpoint.

use shuma_protocol::{default_socket_path, read_frame, write_frame, Request, Response};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tracing::{info, warn};

const DEFAULT_LISTEN: &str = "127.0.0.1:7378";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let listen = std::env::var("SHIPOTE_GATEWAY_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN.into());
    let daemon_sock = Arc::new(default_socket_path());
    let listener = TcpListener::bind(&listen).await?;
    info!(listen = %listen, daemon = %daemon_sock.display(), "shuma-gateway listening");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let sock = daemon_sock.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_http(stream, sock).await {
                        warn!(?e, ?peer, "request error");
                    }
                });
            }
            Err(e) => {
                warn!(?e, "accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

async fn handle_http(mut stream: TcpStream, daemon_sock: Arc<std::path::PathBuf>) -> anyhow::Result<()> {
    // Parser HTTP mínimo: read hasta `\r\n\r\n`, parsear request line +
    // Content-Length, después leer body exacto.
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end;
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(()); // closed
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_double_crlf(&buf) {
            header_end = pos + 4;
            break;
        }
        if buf.len() > 64 * 1024 {
            return write_error(&mut stream, 413, "headers too large").await;
        }
    }

    let header_str = std::str::from_utf8(&buf[..header_end - 4])?;
    let mut lines = header_str.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let mut content_length: usize = 0;
    for line in lines {
        if let Some(v) = line.strip_prefix("Content-Length:").or_else(|| line.strip_prefix("content-length:")) {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }

    // Rutas:
    if method == "GET" && (path == "/" || path == "/health") {
        return write_text(&mut stream, 200, "shuma-gateway ok\n").await;
    }
    if method != "POST" || path != "/rpc" {
        return write_error(&mut stream, 404, "use POST /rpc").await;
    }

    // Leer body.
    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length);

    // Parsear JSON → Request.
    let req: Request = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return write_error(&mut stream, 400, &format!("bad json: {e}")).await,
    };

    // Round-trip al daemon.
    let resp = match round_trip_daemon(&daemon_sock, &req).await {
        Ok(r) => r,
        Err(e) => return write_error(&mut stream, 502, &format!("daemon: {e}")).await,
    };

    // Serializar Response → JSON.
    let body_json = serde_json::to_vec(&resp)?;
    write_response(&mut stream, 200, "application/json", &body_json).await
}

async fn round_trip_daemon(sock: &std::path::Path, req: &Request) -> anyhow::Result<Response> {
    let mut stream = UnixStream::connect(sock).await?;
    write_frame(&mut stream, req).await?;
    let resp: Response = read_frame(&mut stream).await?;
    Ok(resp)
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

async fn write_response(
    stream: &mut TcpStream,
    code: u16,
    content_type: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    let status = match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        502 => "Bad Gateway",
        _ => "Unknown",
    };
    let head = format!(
        "HTTP/1.1 {code} {status}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

async fn write_text(stream: &mut TcpStream, code: u16, body: &str) -> anyhow::Result<()> {
    write_response(stream, code, "text/plain", body.as_bytes()).await
}

async fn write_error(stream: &mut TcpStream, code: u16, msg: &str) -> anyhow::Result<()> {
    let body = serde_json::json!({ "error": msg }).to_string();
    write_response(stream, code, "application/json", body.as_bytes()).await
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("SHIPOTE_GATEWAY_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();
}
