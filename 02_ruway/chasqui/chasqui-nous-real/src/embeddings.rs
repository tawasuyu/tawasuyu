//! Modo embeddings: usa fastembed-rs (ONNX Runtime) para producir
//! vectores reales de text-embedding.
//!
//! Modelo default: `all-MiniLM-L6-v2` (384-d). Se descarga al primer
//! arranque a `~/.cache/fastembed` y queda cacheado.
//!
//! ## Mapeo del contrato
//!
//! - `EmbedText`: pasa el texto al modelo, devuelve el vector 384-d.
//! - `EmbedFile`: lee hasta los primeros 8 KiB del archivo, los
//!   interpreta como UTF-8 con replacement-char, y los embeda como
//!   texto. Para archivos binarios el resultado no es semánticamente
//!   útil — caller decide qué hacer.
//! - `Ping`: devuelve `model_id` y `embed_dim` reales.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use chasqui_nous::{
    EmbedFilePayload, EmbedRequest, EmbedResponse, EmbedTextPayload, ErrorResponse, PingResponse,
    RequestKind,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tracing::{info, warn};

use crate::cache::EmbedCache;

const MAX_FILE_BYTES: usize = 8192;

/// Backend concreto: posee el modelo cargado.
pub struct Backend {
    model: TextEmbedding,
}

impl Backend {
    pub fn init() -> Result<Self, String> {
        info!("cargando modelo all-MiniLM-L6-v2 (puede descargar ~80MB la primera vez)");
        let opts = InitOptions::new(EmbeddingModel::AllMiniLML6V2)
            .with_show_download_progress(true);
        let model = TextEmbedding::try_new(opts).map_err(|e| format!("fastembed init: {e}"))?;
        info!("modelo listo");
        Ok(Self { model })
    }

    fn embed_one(&self, text: &str) -> Result<Vec<f32>, String> {
        let out = self
            .model
            .embed(vec![text], None)
            .map_err(|e| format!("embed: {e}"))?;
        out.into_iter()
            .next()
            .ok_or_else(|| "fastembed devolvió 0 vectores".to_string())
    }
}

pub async fn handle_conn(
    stream: UnixStream,
    backend: Arc<Backend>,
    cache: Option<EmbedCache>,
) -> std::io::Result<()> {
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
        RequestKind::EmbedFile => handle_file(req.payload, &backend, cache.as_ref(), started),
        RequestKind::EmbedText => handle_text(req.payload, &backend, started),
        RequestKind::Ping => handle_ping(),
    };

    let mut stream = reader.into_inner();
    match result {
        Ok(json) => {
            stream.write_all(json.as_bytes()).await?;
            stream.write_all(b"\n").await?;
        }
        Err(msg) => return write_error(stream, msg).await,
    }
    stream.shutdown().await?;
    Ok(())
}

fn handle_text(
    payload: serde_json::Value,
    backend: &Backend,
    started: Instant,
) -> Result<String, String> {
    let p: EmbedTextPayload =
        serde_json::from_value(payload).map_err(|e| format!("payload: {e}"))?;
    info!(text_len = p.text.len(), "embed_text");
    let v = backend.embed_one(&p.text)?;
    let resp = EmbedResponse {
        embedding: v,
        model: super::model_id().to_string(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    serde_json::to_string(&resp).map_err(|e| format!("encode: {e}"))
}

fn handle_file(
    payload: serde_json::Value,
    backend: &Backend,
    cache: Option<&EmbedCache>,
    started: Instant,
) -> Result<String, String> {
    let p: EmbedFilePayload =
        serde_json::from_value(payload).map_err(|e| format!("payload: {e}"))?;

    let path = PathBuf::from(&p.path);
    let mut file = File::open(&path).map_err(|e| format!("abrir archivo: {e}"))?;
    let mut buf = vec![0u8; MAX_FILE_BYTES];
    let n = file.read(&mut buf).map_err(|e| format!("leer archivo: {e}"))?;
    buf.truncate(n);

    let model_id = super::model_id();
    // Hash de los bytes que el modelo realmente verá. Si el archivo
    // crece pasada la ventana MAX_FILE_BYTES sin modificar la cabeza,
    // el hash NO cambia — el embedding cacheado sigue siendo válido
    // bajo la semántica del proveedor (el modelo nunca vio los bytes
    // adicionales). Si la cabeza cambia, el hash cambia y caemos a
    // re-embed naturalmente.
    let file_sha = arje_cas::sha256_of(&buf);

    if let Some(cache) = cache {
        if let Some(cached) = cache.get(&file_sha, model_id) {
            info!(
                path = %p.path,
                sha = %arje_cas::hex(&file_sha),
                bytes = n,
                "embed_file: cache HIT"
            );
            let resp = EmbedResponse {
                embedding: cached,
                model: model_id.to_string(),
                elapsed_ms: started.elapsed().as_millis() as u64,
            };
            return serde_json::to_string(&resp).map_err(|e| format!("encode: {e}"));
        }
    }

    info!(
        path = %p.path,
        sha = %arje_cas::hex(&file_sha),
        bytes = n,
        "embed_file: cache MISS — invocando modelo"
    );

    // Write-through al CAS de arje: hacemos la cabeza del archivo
    // direccionable por contenido. No es la fuente de verdad para
    // el cache (sled lo es) pero deja un registro consultable por
    // herramientas como `ente-cas gc` y permite que otros consumers
    // resuelvan los bytes por hash.
    if let Err(e) = arje_cas::store(&buf) {
        // No-fatal: si CAS no escribe, cacheamos el embedding igual.
        warn!(error = %e, "arje_cas::store falló (no-fatal)");
    }

    let text = String::from_utf8_lossy(&buf).to_string();
    let v = backend.embed_one(&text)?;

    if let Some(cache) = cache {
        cache.put(&file_sha, model_id, &v);
    }

    let resp = EmbedResponse {
        embedding: v,
        model: model_id.to_string(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    serde_json::to_string(&resp).map_err(|e| format!("encode: {e}"))
}

fn handle_ping() -> Result<String, String> {
    let resp = PingResponse {
        model: super::model_id().to_string(),
        embed_dim: super::embed_dim(),
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
