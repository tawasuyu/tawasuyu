//! Cosmobiología — server HTTP single-user.
//!
//! - Reusa `cosmos_app-engine` (VSOP2013 + LRU cache) nativo.
//! - Comparte (por default) la misma `charts.db` SQLite que la app
//!   desktop, vía `directories::ProjectDirs::from("net", "gioser",
//!   "cosmos_app")`. La idea es: levantar `cosmos_app-server`
//!   en localhost y abrir el wheel desde el browser cuando no se está
//!   con la app desktop.
//! - Single-user, sin auth, bind a `127.0.0.1` por default. NO debe
//!   exponerse a la red pública sin agregar auth + HTTPS.
//!
//! ## Endpoints (v1)
//!
//! ```text
//! GET  /api/health                       healthcheck
//! GET  /api/tree                         tree completo (groups + contacts + charts)
//! POST /api/groups                       crear grupo
//! PATCH /api/groups/:id                  renombrar
//! DELETE /api/groups/:id                 borrar
//! POST /api/contacts                     crear contacto
//! PATCH /api/contacts/:id                renombrar
//! DELETE /api/contacts/:id               borrar
//! POST /api/charts                       crear carta (contact_id + birth_data)
//! GET  /api/charts/:id                   chart JSON
//! PATCH /api/charts/:id                  renombrar / editar birth_data
//! DELETE /api/charts/:id                 borrar
//! GET  /api/charts/:id/render            RenderModel JSON (overlays via query)
//! GET  /api/charts/:id/svg               SVG inline
//! GET  /api/sky                          "Cielo ahora" — RenderModel UTC actual
//! ```

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use clap::Parser;
use cosmos_engine::{
    compose_with_options, svg_export, EngineError, NatalOptions, PipelineRequest, RenderModel,
};
use cosmos_render::{compose_wheel, draw_commands_to_svg, CompositionOpts};
use cosmos_model::{
    Chart, ChartId, ChartKind, Contact, ContactId, Group, GroupId, StoredBirthData,
    StoredChartConfig,
};
use cosmos_store::Store;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "cosmos_app-server",
    about = "Servidor HTTP single-user de Cosmobiología."
)]
struct Cli {
    /// Puerto donde escuchar. Default 8787.
    #[arg(long, default_value = "8787")]
    port: u16,
    /// IP a bindear. Default `127.0.0.1` (solo localhost — single-user
    /// sin auth).
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,
    /// Path al archivo SQLite. Default = el mismo de la app desktop
    /// (`$XDG_DATA_HOME/cosmos_app/charts.db`).
    #[arg(long)]
    db: Option<PathBuf>,
    /// Directorio con los assets estáticos del cliente WASM
    /// (output de `wasm-pack build --out-dir <este path>`). Si el
    /// directorio no existe, el endpoint `/static/wasm/*` devuelve
    /// 404 y el cliente cae al SSR.
    #[arg(long, default_value = "crates/apps/cosmos_app-server/static/wasm")]
    static_wasm: PathBuf,
}

#[derive(Clone)]
struct AppState {
    store: Arc<Store>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cosmos_server=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();

    let db_path = match cli.db {
        Some(p) => p,
        None => default_db_path()?,
    };
    info!("DB: {}", db_path.display());
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let store = Arc::new(Store::open(&db_path)?);

    let state = AppState { store };
    let app = router()
        .nest_service("/static/wasm", ServeDir::new(&cli.static_wasm))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", cli.bind, cli.port).parse()?;
    info!("listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn default_db_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dirs = directories::ProjectDirs::from("net", "gioser", "cosmos_app")
        .ok_or("no se pudo determinar XDG data dir")?;
    Ok(dirs.data_dir().join("charts.db"))
}

fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_index))
        .route("/api/health", get(health))
        .route("/api/tree", get(get_tree))
        .route("/api/sky", get(get_sky))
        // El render SVG agnóstico (via `cosmos_app-render::compose_wheel`
        // + `draw_commands_to_svg`) sirve a la fase 3 inicial: el
        // cliente recibe SVG ya compuesto, sin necesidad de WASM.
        // Cuando agreguemos el cliente WASM real, este endpoint se
        // mantiene como fallback "ver SVG sin JS".
        .route("/api/sky.svg", get(get_sky_svg))
        .route("/api/charts/:id/wheel.svg", get(get_chart_wheel_svg))
        .route("/api/groups", post(post_group))
        .route("/api/groups/:id", patch(patch_group).delete(delete_group))
        .route("/api/contacts", post(post_contact))
        .route(
            "/api/contacts/:id",
            patch(patch_contact).delete(delete_contact),
        )
        .route("/api/charts", post(post_chart))
        .route(
            "/api/charts/:id",
            get(get_chart).patch(patch_chart).delete(delete_chart),
        )
        .route("/api/charts/:id/render", get(get_chart_render))
        .route("/api/charts/:id/svg", get(get_chart_svg))
        .layer(CorsLayer::permissive()) // single-user, localhost: cors abierto
        .layer(TraceLayer::new_for_http())
}

// =====================================================================
// Página HTML inicial
// =====================================================================

const INDEX_HTML: &str = include_str!("../static/index.html");

async fn get_index() -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        INDEX_HTML.to_string(),
    )
        .into_response()
}

// SVG render agnóstico (no es el del engine — este viene de
// `cosmos_app-render::compose_wheel` que es lo que mañana el
// cliente WASM también va a usar). Útil para demos sin WASM.
async fn get_sky_svg() -> Result<Response, ApiError> {
    let chart = build_present_sky_chart();
    let model = compose_with_options(&chart, 0, &[], &NatalOptions::default())?;
    let cmds = compose_wheel(&model, &CompositionOpts::default());
    let svg = draw_commands_to_svg(&cmds, 600.0);
    Ok((
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        svg,
    )
        .into_response())
}

async fn get_chart_wheel_svg(
    State(s): State<AppState>,
    Path(id): Path<ChartId>,
    Query(q): Query<RenderQuery>,
) -> Result<Response, ApiError> {
    let chart = s
        .store
        .get_chart(id)
        .map_err(|_| ApiError::NotFound(format!("chart {}", id)))?;
    let model =
        compose_with_options(&chart, q.offset_min, &build_requests(&q), &NatalOptions::default())?;
    let cmds = compose_wheel(&model, &CompositionOpts::default());
    let svg = draw_commands_to_svg(&cmds, 600.0);
    Ok((
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        svg,
    )
        .into_response())
}

// =====================================================================
// Error
// =====================================================================

#[derive(thiserror::Error, Debug)]
enum ApiError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("store: {0}")]
    Store(#[from] cosmos_store::StoreError),
    #[error("engine: {0}")]
    Engine(#[from] EngineError),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, msg) = match &self {
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        (code, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

// =====================================================================
// Health
// =====================================================================

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "cosmos_app-server" }))
}

// =====================================================================
// Tree — listado completo
// =====================================================================

#[derive(Serialize)]
struct TreeNode {
    id: String,
    label: String,
    kind: &'static str, // "group" | "contact" | "chart"
    children: Vec<TreeNode>,
}

async fn get_tree(State(s): State<AppState>) -> ApiResult<Vec<TreeNode>> {
    let mut roots = Vec::new();
    // Grupos top-level
    for g in s.store.list_groups(None)? {
        roots.push(group_node(&s.store, &g)?);
    }
    // Contactos sin grupo (van bajo "General" en el tree desktop;
    // acá los listamos directo al root para no confundir al cliente).
    for c in s.store.list_contacts(None)? {
        roots.push(contact_node(&s.store, &c)?);
    }
    Ok(Json(roots))
}

fn group_node(store: &Store, g: &Group) -> Result<TreeNode, ApiError> {
    let mut children = Vec::new();
    for sub in store.list_groups(Some(g.id))? {
        children.push(group_node(store, &sub)?);
    }
    for c in store.list_contacts(Some(g.id))? {
        children.push(contact_node(store, &c)?);
    }
    Ok(TreeNode {
        id: format!("g:{}", g.id),
        label: g.name.clone(),
        kind: "group",
        children,
    })
}

fn contact_node(store: &Store, c: &Contact) -> Result<TreeNode, ApiError> {
    let charts = store.list_charts(c.id).unwrap_or_default();
    let children: Vec<TreeNode> = charts
        .into_iter()
        .map(|h| TreeNode {
            id: format!("h:{}", h.id),
            label: h.label,
            kind: "chart",
            children: Vec::new(),
        })
        .collect();
    Ok(TreeNode {
        id: format!("c:{}", c.id),
        label: c.name.clone(),
        kind: "contact",
        children,
    })
}

// =====================================================================
// Groups CRUD
// =====================================================================

#[derive(Deserialize)]
struct CreateGroupBody {
    name: String,
    parent: Option<GroupId>,
}

async fn post_group(
    State(s): State<AppState>,
    Json(b): Json<CreateGroupBody>,
) -> ApiResult<Group> {
    let g = s.store.create_group(b.parent, &b.name, None)?;
    Ok(Json(g))
}

#[derive(Deserialize)]
struct PatchGroupBody {
    name: String,
}

async fn patch_group(
    State(s): State<AppState>,
    Path(id): Path<GroupId>,
    Json(b): Json<PatchGroupBody>,
) -> ApiResult<serde_json::Value> {
    s.store.rename_group(id, &b.name)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete_group(
    State(s): State<AppState>,
    Path(id): Path<GroupId>,
) -> ApiResult<serde_json::Value> {
    s.store.delete_group(id)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// =====================================================================
// Contacts CRUD
// =====================================================================

#[derive(Deserialize)]
struct CreateContactBody {
    name: String,
    group: Option<GroupId>,
}

async fn post_contact(
    State(s): State<AppState>,
    Json(b): Json<CreateContactBody>,
) -> ApiResult<Contact> {
    let c = s.store.create_contact(b.group, &b.name, None)?;
    Ok(Json(c))
}

#[derive(Deserialize)]
struct PatchContactBody {
    name: String,
}

async fn patch_contact(
    State(s): State<AppState>,
    Path(id): Path<ContactId>,
    Json(b): Json<PatchContactBody>,
) -> ApiResult<serde_json::Value> {
    s.store.rename_contact(id, &b.name)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete_contact(
    State(s): State<AppState>,
    Path(id): Path<ContactId>,
) -> ApiResult<serde_json::Value> {
    s.store.delete_contact(id)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// =====================================================================
// Charts CRUD
// =====================================================================

#[derive(Deserialize)]
struct CreateChartBody {
    contact_id: ContactId,
    #[serde(default)]
    kind: Option<ChartKind>,
    label: String,
    birth_data: StoredBirthData,
    #[serde(default)]
    config: Option<StoredChartConfig>,
}

async fn post_chart(
    State(s): State<AppState>,
    Json(b): Json<CreateChartBody>,
) -> ApiResult<Chart> {
    let kind = b.kind.unwrap_or(ChartKind::Natal);
    let cfg = b.config.unwrap_or_default();
    let chart = s
        .store
        .create_chart(b.contact_id, kind, &b.label, &b.birth_data, &cfg, None)?;
    Ok(Json(chart))
}

async fn get_chart(
    State(s): State<AppState>,
    Path(id): Path<ChartId>,
) -> ApiResult<Chart> {
    let chart = s
        .store
        .get_chart(id)
        .map_err(|_| ApiError::NotFound(format!("chart {}", id)))?;
    Ok(Json(chart))
}

#[derive(Deserialize)]
struct PatchChartBody {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    birth_data: Option<StoredBirthData>,
    #[serde(default)]
    config: Option<StoredChartConfig>,
}

async fn patch_chart(
    State(s): State<AppState>,
    Path(id): Path<ChartId>,
    Json(b): Json<PatchChartBody>,
) -> ApiResult<serde_json::Value> {
    let current = s
        .store
        .get_chart(id)
        .map_err(|_| ApiError::NotFound(format!("chart {}", id)))?;
    let label = b.label.unwrap_or(current.label);
    let birth = b.birth_data.unwrap_or(current.birth_data);
    let cfg = b.config.unwrap_or(current.config);
    s.store.update_chart(id, &label, &birth, &cfg)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete_chart(
    State(s): State<AppState>,
    Path(id): Path<ChartId>,
) -> ApiResult<serde_json::Value> {
    s.store.delete_chart(id)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// =====================================================================
// Render
// =====================================================================

#[derive(Deserialize, Default)]
struct RenderQuery {
    /// Offset de tiempo en minutos (para "scrubbing").
    #[serde(default)]
    offset_min: i64,
    /// "1" = activar overlay de tránsitos al `now` del server.
    #[serde(default)]
    transit: u8,
    /// Edad (años) — activa progresión secundaria si se setea.
    #[serde(default)]
    prog_age: Option<f64>,
    /// Edad (años) — activa solar arc si se setea.
    #[serde(default)]
    sa_age: Option<f64>,
    /// Edad (años) — activa primary directions si se setea.
    #[serde(default)]
    pd_age: Option<f64>,
}

fn build_requests(q: &RenderQuery) -> Vec<PipelineRequest> {
    let mut r = Vec::new();
    if q.transit == 1 {
        r.push(PipelineRequest::Transit);
    }
    if let Some(a) = q.prog_age {
        r.push(PipelineRequest::SecondaryProgression { target_age_years: a });
    }
    if let Some(a) = q.sa_age {
        r.push(PipelineRequest::SolarArc { target_age_years: a });
    }
    if let Some(a) = q.pd_age {
        r.push(PipelineRequest::PrimaryDirections {
            target_age_years: a,
            key: "naibod".into(),
        });
    }
    r
}

async fn get_chart_render(
    State(s): State<AppState>,
    Path(id): Path<ChartId>,
    Query(q): Query<RenderQuery>,
) -> ApiResult<RenderModel> {
    let chart = s
        .store
        .get_chart(id)
        .map_err(|_| ApiError::NotFound(format!("chart {}", id)))?;
    let model =
        compose_with_options(&chart, q.offset_min, &build_requests(&q), &NatalOptions::default())?;
    Ok(Json(model))
}

async fn get_chart_svg(
    State(s): State<AppState>,
    Path(id): Path<ChartId>,
    Query(q): Query<RenderQuery>,
) -> Result<Response, ApiError> {
    let chart = s
        .store
        .get_chart(id)
        .map_err(|_| ApiError::NotFound(format!("chart {}", id)))?;
    let model =
        compose_with_options(&chart, q.offset_min, &build_requests(&q), &NatalOptions::default())?;
    let svg = svg_export::render_to_svg(&model);
    Ok((
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        svg,
    )
        .into_response())
}

// =====================================================================
// Sky now — sin chart
// =====================================================================

async fn get_sky() -> ApiResult<RenderModel> {
    let chart = build_present_sky_chart();
    let model = compose_with_options(&chart, 0, &[], &NatalOptions::default())?;
    Ok(Json(model))
}

fn build_present_sky_chart() -> Chart {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (year, month, day, hour, minute, second) = unix_to_civil_utc(secs);
    let birth = StoredBirthData {
        year,
        month,
        day,
        hour,
        minute,
        second: second as f64,
        tz_offset_minutes: 0,
        latitude_deg: 51.4769, // Greenwich
        longitude_deg: 0.0,
        altitude_m: 47.0,
        time_certainty: Default::default(),
        subject_name: Some("Cielo".into()),
        birthplace_label: Some("Greenwich (UTC)".into()),
    };
    Chart {
        id: ChartId::default(),
        contact_id: ContactId::default(),
        kind: ChartKind::Natal,
        label: format!(
            "Cielo {:04}-{:02}-{:02} {:02}:{:02} UTC",
            year, month, day, hour, minute
        ),
        birth_data: birth,
        config: StoredChartConfig::default(),
        related_chart_id: None,
        created_at_ms: 0,
    }
}

/// Howard Hinnant `days_to_civil` — Unix UTC → calendario.
/// Mismo algoritmo que en la app desktop; duplicado mínimo para no
/// arrastrar el shell entero como dep del server.
fn unix_to_civil_utc(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let day_seconds: i64 = 86_400;
    let z = secs.div_euclid(day_seconds);
    let s = secs.rem_euclid(day_seconds);
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { (y + 1) as i32 } else { y as i32 };
    let hour = (s / 3600) as u32;
    let minute = ((s % 3600) / 60) as u32;
    let second = (s % 60) as u32;
    (year, month, day, hour, minute, second)
}
