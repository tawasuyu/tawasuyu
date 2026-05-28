//! Daemon HTTP read-only sobre el repo Minga local.
//!
//! Sólo expone consultas — no permite modificar el repo (sería el job de
//! una API authenticada, fuera del alcance de este MVP). Ítem #C del
//! REPORTE: paralelo a `shuma-gateway`, sirve como puente hacia
//! frontends no-Llimphi (web, mobile, otro shell) que quieran leer
//! roots, history o blame sin embeber `minga-cli` como librería.
//!
//! Endpoints:
//! - `GET /status` — counts del repo
//! - `GET /roots` — lista completa de raíces con metadata
//! - `GET /roots/:alpha/show` — fuente reconstruida (text/plain)
//! - `GET /roots/:alpha/show?sexp=1` — S-expression del árbol
//! - `GET /roots/:alpha/signers` — DIDs que han firmado
//! - `GET /roots/:alpha/history?path=<file>` — historial cronológico
//!
//! El passphrase del keypair se pide UNA vez al arrancar el daemon y se
//! mantiene en memoria — todas las requests reaprovechan el load. No es
//! ideal (el keypair real ni siquiera se necesita para read-only, sólo
//! para descifrar y abrir el repo), pero es el path corto al MVP.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::commands::{cmd_history, cmd_roots, cmd_show, cmd_signers, cmd_status};
use crate::error::CliError;

#[derive(Clone)]
struct AppState {
    repo: Arc<PathBuf>,
    passphrase: Arc<String>,
}

/// Arranca el daemon HTTP. Bloquea hasta Ctrl+C (o hasta que axum cierre
/// por algún error de bind).
pub async fn cmd_serve(
    repo_path: &std::path::Path,
    passphrase: &str,
    addr: &str,
) -> Result<(), CliError> {
    // Sanity check: que el repo se pueda abrir y la passphrase sea
    // correcta. Si esto falla, devolvemos el error al CLI sin levantar
    // el server — mejor que dejarlo arrancar y devolver 500 a la
    // primera request.
    let _ = cmd_status(repo_path, passphrase)?;

    let state = AppState {
        repo: Arc::new(repo_path.to_path_buf()),
        passphrase: Arc::new(passphrase.to_string()),
    };

    let app = Router::new()
        .route("/status", get(get_status))
        .route("/roots", get(get_roots))
        .route("/roots/:alpha/show", get(get_show))
        .route("/roots/:alpha/signers", get(get_signers))
        .route("/roots/:alpha/history", get(get_history))
        .with_state(state);

    let sock: SocketAddr = addr.parse().map_err(|_| {
        CliError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("addr inválida: {addr}"),
        ))
    })?;
    let listener = tokio::net::TcpListener::bind(sock).await.map_err(CliError::Io)?;
    eprintln!("minga serve escuchando en http://{}", sock);
    axum::serve(listener, app).await.map_err(CliError::Io)?;
    Ok(())
}

async fn get_status(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let st = cmd_status(&s.repo, &s.passphrase)?;
    Ok(Json(serde_json::json!({
        "did": st.did.to_string(),
        "roots": st.roots_len,
        "mst": st.mst_len,
        "nodes": st.nodes_len,
        "attestations": st.attestations_len,
    })))
}

async fn get_roots(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let rows = cmd_roots(&s.repo, &s.passphrase)?;
    let items: Vec<_> = rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "alpha": r.alpha.to_string(),
                "struct_hash": r.struct_hash.to_string(),
                "dialect": r.dialect.map(|d| d.name()),
                "path": r.path,
                "last_seen_secs": r.last_seen_secs,
                "attestations": r.attestations,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

#[derive(Deserialize)]
struct ShowQuery {
    sexp: Option<u8>,
}

async fn get_show(
    State(s): State<AppState>,
    AxumPath(alpha): AxumPath<String>,
    Query(q): Query<ShowQuery>,
) -> Result<Response, ApiError> {
    let sexp = matches!(q.sexp, Some(n) if n != 0);
    let r = cmd_show(&s.repo, &s.passphrase, &alpha, sexp)?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        r.rendered,
    )
        .into_response())
}

async fn get_signers(
    State(s): State<AppState>,
    AxumPath(alpha): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let entries = cmd_signers(&s.repo, &s.passphrase, &alpha)?;
    let items: Vec<_> = entries
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "author": e.author.to_string(),
                "ts_secs": e.ts_secs,
                "retracted": e.retracted,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "alpha": alpha, "items": items })))
}

#[derive(Deserialize)]
struct HistoryQuery {
    path: String,
}

async fn get_history(
    State(s): State<AppState>,
    AxumPath(_alpha_unused): AxumPath<String>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // El `:alpha` del path no se usa: la API ergonomicamente reusa el
    // namespace `/roots/:alpha/...` pero `history` opera por path local.
    let entries = cmd_history(&s.repo, &s.passphrase, std::path::Path::new(&q.path))?;
    let items: Vec<_> = entries
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "alpha": e.alpha.to_string(),
                "ts_secs": e.ts_secs,
                "dialect": e.dialect.map(|d| d.name()),
                "current": e.current,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "path": q.path, "items": items })))
}

/// Wrapper que mapea `CliError` a HTTP. Errores "de usuario"
/// (HashNotFound, PathNotIngested, InvalidHash) van como 4xx; el resto
/// como 500.
struct ApiError(CliError);

impl From<CliError> for ApiError {
    fn from(e: CliError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match &self.0 {
            CliError::HashNotFound(_) | CliError::PathNotIngested(_) => {
                (StatusCode::NOT_FOUND, self.0.to_string())
            }
            CliError::InvalidHash(_) | CliError::UnsupportedLanguage { .. } => {
                (StatusCode::BAD_REQUEST, self.0.to_string())
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()),
        };
        (status, Json(serde_json::json!({ "error": body }))).into_response()
    }
}

/// Sólo para que un test pueda llamarlo y armar requests sin levantar
/// un socket. Devuelve el `Router` configurado contra `repo_path`.
#[doc(hidden)]
pub fn build_router_for_test(repo_path: PathBuf, passphrase: String) -> Router {
    let state = AppState {
        repo: Arc::new(repo_path),
        passphrase: Arc::new(passphrase),
    };
    Router::new()
        .route("/status", get(get_status))
        .route("/roots", get(get_roots))
        .route("/roots/:alpha/show", get(get_show))
        .route("/roots/:alpha/signers", get(get_signers))
        .route("/roots/:alpha/history", get(get_history))
        .with_state(state)
}

