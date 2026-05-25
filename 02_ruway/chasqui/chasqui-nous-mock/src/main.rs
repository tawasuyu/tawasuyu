//! `chasqui-nous-mock` — proveedor de embeddings determinista (sin LLM).
//!
//! Implementa el contrato `chasqui-nous` usando los pseudo-embeddings
//! de Phase C (`chasqui_core::embed`). Sirve como:
//!
//! - **Mock para tests**: en `BRAHMAN_BROKER_CONTEXT=test`, el
//!   `priority_offset` per-contexto declarado en su Card lo prioriza
//!   sobre cualquier proveedor real.
//! - **Bootstrap**: hasta que llegue el LLM real (Phase D futura), el
//!   sistema funciona end-to-end con embeddings determinísticos.
//!
//! ## Vida del proceso
//!
//! 1. Sidecarea a brahman-init declarando una Card con flow output
//!    `embed-result:json` y flow input `embed-request:json`. Su
//!    `priority_contexts.test = { priority_offset: +1 }` lo prioriza
//!    cuando el broker corre bajo contexto test.
//! 2. Bind del Unix socket en `$NOUSER_NOUS_SOCKET` (default
//!    `$XDG_RUNTIME_DIR/chasqui-nous.sock`).
//! 3. Loop: accept → read line JSON → process → write line JSON → close.
//! 4. Cada request se loggea (info) — útil para verificar que el
//!    consumidor está usando este proveedor.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use card_core::{
    ulid::Ulid, Card, CardKind, ContextBias, Flow, Flows, Lifecycle, Payload, Priority,
    Supervision, TypeRef,
};
use chasqui_card::FileEntry;
use chasqui_core::embed;
use chasqui_nous::{
    transport, EmbedFilePayload, EmbedRequest, EmbedResponse, EmbedTextPayload, ErrorResponse,
    PingResponse, RequestKind, FLOW_EMBED_REQUEST, FLOW_EMBED_RESULT, FLOW_TYPE_NAME,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

/// El mock implementa el MISMO algoritmo que `chasqui_core::embed`,
/// así que reportamos el mismo `MODEL_ID` que él. De otro modo el
/// consumer filtraría las Mónadas como "modelo distinto" y los
/// scores quedarían vacíos.
const MODEL_ID: &str = chasqui_core::embed::MODEL_ID;

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    init_tracing();

    // 1. Resolver socket del data-plane ANTES de armar la Card, para
    //    declararlo en `Card.service_socket` y que los consumidores lo
    //    descubran vía MatchEvent.
    let sock_path = transport::provider_socket_path("mock");
    if sock_path.exists() {
        std::fs::remove_file(&sock_path)?;
    }
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(&sock_path)?;
    info!(socket = %sock_path.display(), "chasqui-nous-mock escuchando");

    // 2. Sidecar al brahman-init con la Card que declara el socket.
    let card = build_card(sock_path.clone());
    info!(label = %card.label, "publicando Card al brahman-init");
    card_sidecar::spawn(card);

    // 3. Accept loop.
    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream).await {
                warn!(error = %e, "conn falló");
            }
        });
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .compact()
        .init();
}

/// Card que el mock anuncia al brahman-init. Es kind=Ente (un proceso),
/// con flujos JSON, bias de prioridad para contexto `test`, y el socket
/// data-plane declarado en `service_socket` (consumidores lo reciben
/// directo en el `MatchEvent::Available`).
fn build_card(service_socket: std::path::PathBuf) -> Card {
    let mut priority_contexts = BTreeMap::new();
    priority_contexts.insert(
        "test".into(),
        ContextBias {
            pin_to: None,
            // En contexto test, este mock gana sobre cualquier real-nous.
            priority_offset: 1,
        },
    );

    Card {
        schema_version: card_core::CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        label: "chasqui.nous_mock".into(),
        payload: Payload::Virtual,
        supervision: Supervision::Delegate,
        lifecycle: Lifecycle::Daemon,
        priority: Priority::Normal,
        kind: CardKind::Ente,
        service_socket: Some(service_socket),
        flow: Flows {
            input: vec![Flow {
                name: FLOW_EMBED_REQUEST.into(),
                ty: TypeRef::Primitive {
                    name: FLOW_TYPE_NAME.into(),
                },
                pin_to: None,
            }],
            output: vec![Flow {
                name: FLOW_EMBED_RESULT.into(),
                ty: TypeRef::Primitive {
                    name: FLOW_TYPE_NAME.into(),
                },
                pin_to: None,
            }],
        },
        priority_contexts,
        ..Default::default()
    }
}

/// Procesa una conexión single-shot: lee una línea JSON, despacha,
/// escribe una línea JSON, cierra.
async fn handle_conn(stream: UnixStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(());
    }
    let req: EmbedRequest = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(e) => {
            return write_error(reader.into_inner(), format!("JSON inválido: {e}")).await;
        }
    };

    let started = Instant::now();
    let result = match req.kind {
        RequestKind::EmbedFile => handle_embed_file(req.payload, started),
        RequestKind::EmbedText => handle_embed_text(req.payload, started),
        RequestKind::Ping => handle_ping(),
    };

    let mut stream = reader.into_inner();
    match result {
        Ok(payload) => {
            stream.write_all(payload.as_bytes()).await?;
            stream.write_all(b"\n").await?;
        }
        Err(msg) => {
            return write_error(stream, msg).await;
        }
    }
    stream.shutdown().await?;
    Ok(())
}

fn handle_embed_file(payload: serde_json::Value, started: Instant) -> Result<String, String> {
    let p: EmbedFilePayload =
        serde_json::from_value(payload).map_err(|e| format!("payload inválido: {e}"))?;
    info!(path = %p.path, "embed_file");

    let file = FileEntry {
        id: chasqui_card::FileId::from(Ulid::new()),
        path: PathBuf::from(p.path),
        content_hash: None,
        size: p.size,
        mtime_ms: p.mtime_ms,
        extension: p.extension,
    };
    let v = embed::embed(&file);

    let resp = EmbedResponse {
        embedding: v.to_vec(),
        model: MODEL_ID.into(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    serde_json::to_string(&resp).map_err(|e| format!("encode: {e}"))
}

fn handle_embed_text(payload: serde_json::Value, started: Instant) -> Result<String, String> {
    let p: EmbedTextPayload =
        serde_json::from_value(payload).map_err(|e| format!("payload inválido: {e}"))?;
    info!(text_len = p.text.len(), "embed_text");

    // Mock: tratamos el texto como un "stem" sintético y rellenamos el
    // resto del vector con ceros. No es semánticamente útil, pero respeta
    // la forma para que el cliente no se rompa.
    let synthetic = FileEntry {
        id: chasqui_card::FileId::from(Ulid::new()),
        path: PathBuf::from(format!("synthetic://{}", p.text)),
        content_hash: None,
        size: p.text.len() as u64,
        mtime_ms: now_ms(),
        extension: Some("text".into()),
    };
    let v = embed::embed(&synthetic);

    let resp = EmbedResponse {
        embedding: v.to_vec(),
        model: MODEL_ID.into(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    serde_json::to_string(&resp).map_err(|e| format!("encode: {e}"))
}

fn handle_ping() -> Result<String, String> {
    let resp = PingResponse {
        model: MODEL_ID.into(),
        embed_dim: embed::EMBED_DIM as u32,
    };
    serde_json::to_string(&resp).map_err(|e| format!("encode: {e}"))
}

async fn write_error(mut stream: UnixStream, msg: String) -> std::io::Result<()> {
    warn!(error = %msg, "respuesta de error");
    let resp = ErrorResponse { error: msg };
    let json = serde_json::to_string(&resp).unwrap_or_else(|_| "{\"error\":\"encode\"}".into());
    stream.write_all(json.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.shutdown().await?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
