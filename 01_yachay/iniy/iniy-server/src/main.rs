//! iniy-server — API HTTP read-only sobre una DB de iniy.
//!
//! Endpoints (todos JSON):
//!   GET /healthz                              → "ok"
//!   GET /fuentes                              → lista con reputación
//!   GET /aserciones?tag=X&limit=N&offset=M    → aserciones paginadas
//!   GET /aserciones/:id                       → detalle + vecinos NLI
//!   GET /testimonio?q=...&umbral=...&top=...  → apoyan/contradicen
//!   GET /consenso?q=...&pesar_reputacion=1    → opinión fusionada
//!   GET /contradicciones?top=N                → top-N pares
//!
//! El binario abre la DB UNA VEZ al arrancar y la comparte vía
//! `Arc<Mutex<Store>>`. Sin escrituras (read-only).

use anyhow::Result;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Json;
use clap::Parser;
use iniy_core::{AsercionId, FuenteId, Opinion};
use iniy_nli::relacion_lexica;
use iniy_store::{AsercionAtribuida, Store};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Parser)]
#[command(name = "iniy-server", about = "API HTTP read-only sobre iniy")]
struct Cli {
    /// Ruta a la DB SQLite (default: ./iniy.db).
    #[arg(long, default_value = "iniy.db")]
    db: PathBuf,

    /// Bind address (default: 127.0.0.1:7777).
    #[arg(long, default_value = "127.0.0.1:7777")]
    bind: String,
}

#[derive(Clone)]
struct AppState {
    store: Arc<Mutex<Store>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();
    let cli = Cli::parse();
    let store = Store::abrir(&cli.db)?;
    let state = AppState { store: Arc::new(Mutex::new(store)) };

    let app = axum::Router::new()
        .route("/healthz", get(healthz))
        .route("/fuentes", get(get_fuentes))
        .route("/aserciones", get(get_aserciones))
        .route("/aserciones/{id}", get(get_asercion))
        .route("/testimonio", get(get_testimonio))
        .route("/consenso", get(get_consenso))
        .route("/contradicciones", get(get_contradicciones))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&cli.bind).await?;
    let addr = listener.local_addr()?;
    println!("iniy-server escuchando en http://{}", addr);
    println!("DB: {}", cli.db.display());
    println!("rutas disponibles:");
    for r in &["/healthz", "/fuentes", "/aserciones", "/aserciones/:id",
                "/testimonio?q=...", "/consenso?q=...", "/contradicciones?top=N"] {
        println!("  GET {r}");
    }
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> &'static str { "ok" }

#[derive(Serialize)]
struct FuenteOut {
    id: String,
    nombre: String,
    kind: Option<String>,
    n_docs: u32,
    n_aserciones: u32,
    reputacion: f32,
}

async fn get_fuentes(State(state): State<AppState>) -> Result<Json<Vec<FuenteOut>>, ApiError> {
    let store = state.store.lock().await;
    let fuentes = store.listar_fuentes()?;
    let reps = store.cargar_reputaciones_todas().unwrap_or_default();
    let rep_map: std::collections::HashMap<FuenteId, f32> = reps.into_iter()
        .map(|r| (r.fuente_id, r.score)).collect();
    let out: Vec<FuenteOut> = fuentes.into_iter().map(|f| FuenteOut {
        id: f.fuente.id.0.to_string(),
        nombre: f.fuente.nombre,
        kind: f.fuente.kind,
        n_docs: f.n_docs,
        n_aserciones: f.n_aserciones,
        reputacion: rep_map.get(&f.fuente.id).copied().unwrap_or(0.0),
    }).collect();
    Ok(Json(out))
}

#[derive(Deserialize)]
struct ListaQuery {
    tag: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}
fn default_limit() -> usize { 50 }

#[derive(Serialize)]
struct AsercionOut {
    id: String,
    texto: String,
    fuente: Option<FuenteResumenOut>,
    doc_titulo: String,
    citada: bool,
    opinion: OpinionOut,
}

#[derive(Serialize)]
struct FuenteResumenOut {
    id: String,
    nombre: String,
    kind: Option<String>,
}

#[derive(Serialize)]
struct OpinionOut {
    creencia: f32,
    descreencia: f32,
    incertidumbre: f32,
    base_rate: f32,
    probabilidad_esperada: f32,
}

fn opinion_out(op: &Opinion) -> OpinionOut {
    OpinionOut {
        creencia: op.creencia,
        descreencia: op.descreencia,
        incertidumbre: op.incertidumbre,
        base_rate: op.base_rate,
        probabilidad_esperada: op.probabilidad_esperada(),
    }
}

fn att_out(att: &AsercionAtribuida) -> AsercionOut {
    AsercionOut {
        id: att.asercion.id.0.to_string(),
        texto: att.asercion.texto.clone(),
        fuente: att.fuente.as_ref().map(|f| FuenteResumenOut {
            id: f.id.0.to_string(),
            nombre: f.nombre.clone(),
            kind: f.kind.clone(),
        }),
        doc_titulo: att.doc_titulo.clone(),
        citada: att.citada,
        opinion: opinion_out(&att.asercion.opinion_autoral),
    }
}

async fn get_aserciones(
    State(state): State<AppState>,
    Query(q): Query<ListaQuery>,
) -> Result<Json<Vec<AsercionOut>>, ApiError> {
    let store = state.store.lock().await;
    let todas = match q.tag {
        Some(t) => store.cargar_aserciones_atribuidas_por_tag(&t)?,
        None => store.cargar_aserciones_atribuidas_todas()?,
    };
    let out: Vec<AsercionOut> = todas.iter()
        .skip(q.offset)
        .take(q.limit)
        .map(att_out)
        .collect();
    Ok(Json(out))
}

#[derive(Serialize)]
struct AsercionConVecinos {
    asercion: AsercionOut,
    vecinos: Vec<VecinoOut>,
}

#[derive(Serialize)]
struct VecinoOut {
    asercion_id: String,
    texto: String,
    relacion: &'static str,  // "entailment" | "contradiction"
    score: f32,
    fuente: Option<String>,
}

async fn get_asercion(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
) -> Result<Json<AsercionConVecinos>, ApiError> {
    let store = state.store.lock().await;
    let aid = AsercionId(ulid::Ulid::from_string(&id_str)
        .map_err(|_| ApiError::NotFound("id inválido".into()))?);
    let todas = store.cargar_aserciones_atribuidas_todas()?;
    let att = todas.iter().find(|a| a.asercion.id == aid)
        .ok_or_else(|| ApiError::NotFound(format!("aserción {id_str}")))?;
    let imps = store.cargar_implicaciones_todas()?;
    let texto_por_id: std::collections::HashMap<AsercionId, (&str, Option<String>)> =
        todas.iter().map(|a| (
            a.asercion.id,
            (a.asercion.texto.as_str(), a.fuente.as_ref().map(|f| f.nombre.clone())),
        )).collect();
    let vecinos: Vec<VecinoOut> = imps.iter()
        .filter_map(|i| {
            let otro = if i.premisa == aid { Some(i.hipotesis) }
                else if i.hipotesis == aid { Some(i.premisa) }
                else { None }?;
            let (relacion, score) = if i.relacion.contradiction > i.relacion.entailment && i.relacion.contradiction > 0.0 {
                ("contradiction", i.relacion.contradiction)
            } else if i.relacion.entailment > 0.0 {
                ("entailment", i.relacion.entailment)
            } else {
                return None;
            };
            let (texto, fuente) = texto_por_id.get(&otro).map(|(t, f)| (t.to_string(), f.clone()))?;
            Some(VecinoOut {
                asercion_id: otro.0.to_string(),
                texto,
                relacion,
                score,
                fuente,
            })
        })
        .collect();
    Ok(Json(AsercionConVecinos { asercion: att_out(att), vecinos }))
}

#[derive(Deserialize)]
struct ConsultaQuery {
    q: String,
    #[serde(default = "default_umbral")]
    umbral: f32,
    #[serde(default = "default_top")]
    top: usize,
    tag: Option<String>,
    #[serde(default)]
    pesar_reputacion: bool,
}
fn default_umbral() -> f32 { 0.20 }
fn default_top() -> usize { 10 }

#[derive(Serialize)]
struct TestimonioOut {
    query: String,
    scanned: usize,
    apoyan: Vec<ContribucionOut>,
    contradicen: Vec<ContribucionOut>,
}

#[derive(Serialize)]
struct ContribucionOut {
    score: f32,
    asercion: AsercionOut,
}

async fn get_testimonio(
    State(state): State<AppState>,
    Query(q): Query<ConsultaQuery>,
) -> Result<Json<TestimonioOut>, ApiError> {
    let store = state.store.lock().await;
    let todas = match q.tag.as_deref() {
        Some(t) => store.cargar_aserciones_atribuidas_por_tag(t)?,
        None => store.cargar_aserciones_atribuidas_todas()?,
    };
    let mut apoyan = Vec::new();
    let mut contradicen = Vec::new();
    for att in todas.iter() {
        let rel = relacion_lexica(&q.q, &att.asercion.texto, q.umbral);
        if rel.entailment > 0.0 {
            apoyan.push((rel.entailment, att.clone()));
        } else if rel.contradiction > 0.0 {
            contradicen.push((rel.contradiction, att.clone()));
        }
    }
    apoyan.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    contradicen.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let scanned = todas.len();
    Ok(Json(TestimonioOut {
        query: q.q.clone(),
        scanned,
        apoyan: apoyan.into_iter().take(q.top).map(|(s, a)| ContribucionOut { score: s, asercion: att_out(&a) }).collect(),
        contradicen: contradicen.into_iter().take(q.top).map(|(s, a)| ContribucionOut { score: s, asercion: att_out(&a) }).collect(),
    }))
}

#[derive(Serialize)]
struct ConsensoOut {
    query: String,
    fuentes_que_hablan: usize,
    opinion_fusionada: OpinionOut,
    pesado_por_reputacion: bool,
}

async fn get_consenso(
    State(state): State<AppState>,
    Query(q): Query<ConsultaQuery>,
) -> Result<Json<ConsensoOut>, ApiError> {
    let store = state.store.lock().await;
    let todas = match q.tag.as_deref() {
        Some(t) => store.cargar_aserciones_atribuidas_por_tag(t)?,
        None => store.cargar_aserciones_atribuidas_todas()?,
    };
    let reps: std::collections::HashMap<FuenteId, f32> = if q.pesar_reputacion {
        store.cargar_reputaciones_todas().unwrap_or_default().into_iter()
            .map(|r| (r.fuente_id, r.score)).collect()
    } else {
        std::collections::HashMap::new()
    };
    let mut ops = Vec::new();
    for att in todas.iter() {
        let rel = relacion_lexica(&q.q, &att.asercion.texto, q.umbral);
        let base = if rel.entailment > 0.0 {
            Some(att.asercion.opinion_autoral.descontar(rel.entailment))
        } else if rel.contradiction > 0.0 {
            Some(att.asercion.opinion_autoral.invertir().descontar(rel.contradiction))
        } else {
            None
        };
        if let Some(mut op) = base {
            if q.pesar_reputacion {
                if let Some(fid) = att.fuente.as_ref().map(|f| f.id) {
                    let rep = reps.get(&fid).copied().unwrap_or(0.0);
                    let peso = ((1.0 + rep) / 2.0).clamp(0.0, 1.0);
                    op = op.descontar(peso);
                }
            }
            ops.push(op);
        }
    }
    let fusion = Opinion::fusionar_muchas(&ops);
    Ok(Json(ConsensoOut {
        query: q.q.clone(),
        fuentes_que_hablan: ops.len(),
        opinion_fusionada: opinion_out(&fusion),
        pesado_por_reputacion: q.pesar_reputacion,
    }))
}

#[derive(Deserialize)]
struct TopQuery {
    #[serde(default = "default_top")]
    top: usize,
}

#[derive(Serialize)]
struct ContradiccionOut {
    contradiction_score: f32,
    premisa: AsercionOut,
    hipotesis: AsercionOut,
}

async fn get_contradicciones(
    State(state): State<AppState>,
    Query(q): Query<TopQuery>,
) -> Result<Json<Vec<ContradiccionOut>>, ApiError> {
    let store = state.store.lock().await;
    let todas = store.cargar_aserciones_atribuidas_todas()?;
    let imps = store.cargar_implicaciones_todas()?;
    let att_por_id: std::collections::HashMap<AsercionId, &AsercionAtribuida> =
        todas.iter().map(|a| (a.asercion.id, a)).collect();
    let mut filtradas: Vec<_> = imps.iter()
        .filter(|i| i.relacion.contradiction > i.relacion.entailment && i.relacion.contradiction > 0.0)
        .collect();
    filtradas.sort_by(|a, b| b.relacion.contradiction.partial_cmp(&a.relacion.contradiction)
        .unwrap_or(std::cmp::Ordering::Equal));
    let out: Vec<ContradiccionOut> = filtradas.into_iter().take(q.top)
        .filter_map(|i| {
            let p = att_por_id.get(&i.premisa)?;
            let h = att_por_id.get(&i.hipotesis)?;
            Some(ContradiccionOut {
                contradiction_score: i.relacion.contradiction,
                premisa: att_out(p),
                hipotesis: att_out(h),
            })
        })
        .collect();
    Ok(Json(out))
}

#[derive(Debug)]
enum ApiError {
    Internal(String),
    NotFound(String),
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match self {
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            ApiError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}
