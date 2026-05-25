//! Modo stub: arranca el bin pero rechaza las requests con un error
//! que explica que falta la feature `embeddings`.

use chasqui_nous::{EmbedRequest, ErrorResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tracing::warn;

pub async fn handle_conn(stream: UnixStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(());
    }

    // Parseamos para validar la forma; igual rechazamos.
    let _: Result<EmbedRequest, _> = serde_json::from_str(&line);

    warn!("rechazando request en modo stub (feature `embeddings` ausente)");

    let resp = ErrorResponse {
        error: format!(
            "chasqui-nous-real compilado sin la feature `embeddings`. \
             Rebuild con: cargo build -p chasqui-nous-real --features embeddings"
        ),
    };
    let mut stream = reader.into_inner();
    let payload = serde_json::to_string(&resp).unwrap_or_else(|_| {
        "{\"error\":\"stub mode and serialization failed\"}".to_string()
    });
    stream.write_all(payload.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.shutdown().await?;
    Ok(())
}
