//! `chasqui-nous` — el contrato del proveedor de embeddings.
//!
//! Define el wire-format compartido entre `chasqui-core` (consumidor) y
//! cualquier implementación de Nous (mock determinista o LLM real). El
//! protocolo es **line-delimited JSON** sobre Unix socket: cada conexión
//! envía una request, recibe una response, y cierra. Single-shot por
//! conexión, igual al admin de brahman.
//!
//! ## Contrato
//!
//! ```text
//! C → S: {"kind":"embed_file","payload":{...}}\n
//! S → C: {"embedding":[...],"model":"mock-pseudo-32d","elapsed_ms":1}\n
//! ```
//!
//! En caso de error:
//!
//! ```text
//! S → C: {"error":"unsupported kind"}\n
//! ```
//!
//! ## Por qué un crate aparte
//!
//! El consumidor (chasqui-core) y el proveedor (chasqui-nous-mock,
//! chasqui-nous-real) deben acordar en types EXACTOS. Tener el contrato
//! en su crate evita que cada lado declare structs paralelos que se
//! desincronizan. Si bumpeás el wire, bumpeás aquí.
//!
//! ## Swap por priority_contexts
//!
//! Cuando existan dos proveedores (mock-nous y real-nous), ambos declaran
//! el mismo `flow.output: { name: "embed-result", type: ... }` y
//! `flow.input: "embed-request"`. El broker brahman los matchea contra
//! los consumidores; el `priority_offset` per-contexto del Card hace que
//! mock-nous gane en `test` y real-nous en `prod`. chasqui-core sólo
//! consume el flow, sin saber cuál implementación corre.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

// =====================================================================
// Wire types
// =====================================================================

/// Request al proveedor Nous.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedRequest {
    pub kind: RequestKind,
    pub payload: serde_json::Value,
}

/// Tipo de request. El payload se interpreta según el kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    /// payload = `EmbedFilePayload` (path + metadata mínima).
    EmbedFile,
    /// payload = `EmbedTextPayload` (string libre).
    EmbedText,
    /// payload = `{}`. Devuelve `PingResponse`.
    Ping,
}

/// Payload para `EmbedFile`. Es la información mínima que el proveedor
/// necesita para producir un embedding de archivo determinista.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedFilePayload {
    pub path: String,
    pub extension: Option<String>,
    pub size: u64,
    /// `mtime` en ms desde UNIX_EPOCH.
    pub mtime_ms: u64,
}

/// Payload para `EmbedText`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedTextPayload {
    pub text: String,
}

/// Response exitosa con un embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedResponse {
    /// Vector. Su longitud depende del modelo (mock=32, llama=384, etc.).
    pub embedding: Vec<f32>,
    /// Identificador del modelo que produjo el embedding (útil para logs
    /// y para invalidar caches al cambiar de proveedor).
    pub model: String,
    /// Tiempo de cómputo en ms (proveedor lo reporta).
    pub elapsed_ms: u64,
}

/// Response a Ping. Útil para health-checks y discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingResponse {
    pub model: String,
    pub embed_dim: u32,
}

/// Error retornado por el proveedor en lugar de una response normal.
#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("nous: {error}")]
pub struct ErrorResponse {
    pub error: String,
}

// =====================================================================
// Transport
// =====================================================================

pub mod transport {
    use std::path::PathBuf;

    /// Variable de entorno para sobreescribir la ruta del socket.
    pub const SOCKET_ENV: &str = "NOUSER_NOUS_SOCKET";

    /// Nombre genérico del socket cuando hay un solo proveedor.
    pub const SOCKET_NAME: &str = "chasqui-nous.sock";

    /// Ruta canónica al socket cuando un único proveedor está activo
    /// (consumidores que no quieren elegir).
    pub fn default_socket_path() -> PathBuf {
        if let Ok(p) = std::env::var(SOCKET_ENV) {
            return PathBuf::from(p);
        }
        runtime_base().join(SOCKET_NAME)
    }

    /// Ruta default para un proveedor identificado (`"mock"`, `"real"`,
    /// etc). Permite que mock y real coexistan sin clash de socket.
    /// `NOUSER_NOUS_SOCKET` igual override esta función si está set.
    pub fn provider_socket_path(provider: &str) -> PathBuf {
        if let Ok(p) = std::env::var(SOCKET_ENV) {
            return PathBuf::from(p);
        }
        runtime_base().join(format!("chasqui-nous-{}.sock", provider))
    }

    fn runtime_base() -> PathBuf {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
    }
}

// =====================================================================
// Names compartidos para el broker brahman
// =====================================================================

/// Nombre del flow output del proveedor (entrada del consumidor).
pub const FLOW_EMBED_RESULT: &str = "embed-result";

/// Nombre del flow input del proveedor (salida del consumidor).
pub const FLOW_EMBED_REQUEST: &str = "embed-request";

/// Tipo del flow: el wire es JSON serializado, así que el TypeRef
/// declarado en la Card es `primitive::json`.
pub const FLOW_TYPE_NAME: &str = "json";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip_json() {
        let req = EmbedRequest {
            kind: RequestKind::EmbedFile,
            payload: serde_json::to_value(EmbedFilePayload {
                path: "/x/y.rs".into(),
                extension: Some("rs".into()),
                size: 1024,
                mtime_ms: 1_700_000_000_000,
            })
            .unwrap(),
        };
        let s = serde_json::to_string(&req).unwrap();
        let parsed: EmbedRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.kind, RequestKind::EmbedFile);
    }

    #[test]
    fn response_roundtrip() {
        let resp = EmbedResponse {
            embedding: vec![0.1, 0.2, 0.3],
            model: "mock-pseudo-32d".into(),
            elapsed_ms: 1,
        };
        let s = serde_json::to_string(&resp).unwrap();
        let parsed: EmbedResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.model, "mock-pseudo-32d");
        assert_eq!(parsed.embedding.len(), 3);
    }
}
