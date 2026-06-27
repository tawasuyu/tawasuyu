//! `pluma-llm-claude-cli` — backend LLM sobre el binario `claude` (Claude Code).
//!
//! En vez de hablar HTTP con la API de Anthropic (que exige una **API key** y
//! cobra por token), este backend ejecuta el CLI oficial:
//!
//! ```text
//! claude -p "<prompt>" --output-format json [--model M] [--append-system-prompt S]
//! ```
//!
//! **Por qué importa:** `claude` resuelve la autenticación con las credenciales
//! que ya tiene configuradas — incluida la **suscripción Pro/Max** vía OAuth. La
//! app NO toca ni reusa el token (eso violaría los ToS); Claude Code, que es la
//! app oficial, hace el auth, y nosotros sólo lo invocamos como subproceso. Es
//! el camino legítimo para aprovechar una suscripción desde software propio,
//! sin pagar por token aparte.
//!
//! El binario se resuelve por `$CLAUDE_CLI_BIN` (si está) o `claude` en el PATH.
//! Se corre en un directorio temporal para que no escanee el cwd del proceso.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use pluma_llm_core::{
    ChatClient, ChatError, ChatRequest, ChatResponse, ChatUsage, Role, StopReason,
};
use std::process::Stdio;

/// Cliente que delega en el CLI `claude`.
pub struct ClaudeCliClient {
    /// Ruta/nombre del binario (`claude` por defecto).
    bin: String,
    /// Modelo a pedir (`--model`). `None` = el default de la cuenta/suscripción.
    model: Option<String>,
    /// Lo que reporta [`ChatClient::model_id`].
    model_id: String,
}

impl Default for ClaudeCliClient {
    fn default() -> Self {
        let bin = std::env::var("CLAUDE_CLI_BIN").unwrap_or_else(|_| "claude".to_string());
        Self { bin, model: None, model_id: "claude-cli".to_string(), }
    }
}

impl ClaudeCliClient {
    /// Cliente con el binario por defecto (`claude` o `$CLAUDE_CLI_BIN`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Encadenable: fija el modelo (`--model`). Vacío = default de la cuenta.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        let m = model.into();
        if !m.trim().is_empty() {
            self.model_id = format!("claude-cli:{m}");
            self.model = Some(m);
        }
        self
    }

    /// Encadenable: fija el binario explícito.
    pub fn with_bin(mut self, bin: impl Into<String>) -> Self {
        self.bin = bin.into();
        self
    }

    /// Renderiza la conversación a un único prompt de texto. Para una sola
    /// vuelta es el mensaje tal cual; para multi-turno, el transcripto con
    /// roles + `Asistente:` al final para que el modelo continúe.
    fn render_prompt(req: &ChatRequest) -> String {
        if req.messages.len() <= 1 {
            return req
                .messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
        }
        let mut s = String::new();
        for m in &req.messages {
            let who = match m.role {
                Role::User => "Usuario",
                Role::Assistant => "Asistente",
            };
            s.push_str(who);
            s.push_str(": ");
            s.push_str(&m.content);
            s.push_str("\n\n");
        }
        s.push_str("Asistente:");
        s
    }
}

#[async_trait]
impl ChatClient for ClaudeCliClient {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, ChatError> {
        let prompt = Self::render_prompt(req);

        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.arg("-p")
            .arg(&prompt)
            .arg("--output-format")
            .arg("json");
        if let Some(m) = &self.model {
            cmd.arg("--model").arg(m);
        }
        if let Some(sys) = &req.system {
            if !sys.trim().is_empty() {
                cmd.arg("--append-system-prompt").arg(sys);
            }
        }
        // Aislado del cwd del proceso para que no lea/escriba archivos.
        cmd.current_dir(std::env::temp_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let out = cmd.output().await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ChatError::AuthMissing(format!(
                    "no encontré el binario «{}» — instalá Claude Code y `claude login`",
                    self.bin
                ))
            } else {
                ChatError::Backend(format!("no pude ejecutar «{}»: {e}", self.bin))
            }
        })?;

        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = if err.trim().is_empty() {
                String::from_utf8_lossy(&out.stdout).into_owned()
            } else {
                err.into_owned()
            };
            return Err(ChatError::Backend(format!(
                "claude salió con error: {}",
                err.trim()
            )));
        }

        let v: serde_json::Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| ChatError::Backend(format!("salida de claude no es JSON: {e}")))?;

        if v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false) {
            let msg = v.get("result").and_then(|r| r.as_str()).unwrap_or("error de claude");
            return Err(ChatError::Backend(msg.to_string()));
        }

        let content = v
            .get("result")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();

        let usage = v.get("usage").map(|u| ChatUsage {
            input_tokens: u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            output_tokens: u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            cache_read_input_tokens: u
                .get("cache_read_input_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0) as u32,
            cache_creation_input_tokens: u
                .get("cache_creation_input_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0) as u32,
        });

        Ok(ChatResponse {
            content,
            stop_reason: Some(StopReason("end_turn".to_string())),
            usage,
        })
    }

    async fn stream(
        &self,
        req: &ChatRequest,
        on_delta: &mut (dyn for<'s> FnMut(&'s str) + Send),
    ) -> Result<ChatResponse, ChatError> {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let prompt = Self::render_prompt(req);

        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.arg("-p")
            .arg(&prompt)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages");
        if let Some(m) = &self.model {
            cmd.arg("--model").arg(m);
        }
        if let Some(sys) = &req.system {
            if !sys.trim().is_empty() {
                cmd.arg("--append-system-prompt").arg(sys);
            }
        }
        cmd.current_dir(std::env::temp_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ChatError::AuthMissing(format!(
                    "no encontré el binario «{}» — instalá Claude Code y `claude login`",
                    self.bin
                ))
            } else {
                ChatError::Backend(format!("no pude ejecutar «{}»: {e}", self.bin))
            }
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ChatError::Backend("sin stdout de claude".into()))?;
        let mut lineas = BufReader::new(stdout).lines();

        let mut acumulado = String::new();
        let mut content: Option<String> = None;
        let mut usage: Option<ChatUsage> = None;
        let mut err_msg: Option<String> = None;

        // NDJSON: cada línea es un evento. Los `content_block_delta` traen el
        // texto incremental; el `result` final trae el texto completo + usage.
        while let Ok(Some(linea)) = lineas.next_line().await {
            let linea = linea.trim();
            if linea.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(linea) else {
                continue;
            };
            match v.get("type").and_then(|t| t.as_str()) {
                Some("stream_event") => {
                    let ev = v.get("event");
                    let es_delta = ev
                        .and_then(|e| e.get("type"))
                        .and_then(|t| t.as_str())
                        == Some("content_block_delta");
                    if es_delta {
                        if let Some(txt) = ev
                            .and_then(|e| e.get("delta"))
                            .and_then(|d| d.get("text"))
                            .and_then(|t| t.as_str())
                        {
                            acumulado.push_str(txt);
                            on_delta(txt);
                        }
                    }
                }
                Some("result") => {
                    if v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false) {
                        err_msg = Some(
                            v.get("result")
                                .and_then(|r| r.as_str())
                                .unwrap_or("error de claude")
                                .to_string(),
                        );
                    }
                    content = v.get("result").and_then(|r| r.as_str()).map(|s| s.to_string());
                    usage = v.get("usage").map(parse_usage);
                }
                _ => {}
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| ChatError::Backend(format!("claude no terminó bien: {e}")))?;

        if let Some(e) = err_msg {
            return Err(ChatError::Backend(e));
        }
        if !status.success() && content.is_none() {
            let mut buf = String::new();
            if let Some(mut se) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = se.read_to_string(&mut buf).await;
            }
            return Err(ChatError::Backend(format!(
                "claude salió con error: {}",
                buf.trim()
            )));
        }

        Ok(ChatResponse {
            content: content.unwrap_or(acumulado),
            stop_reason: Some(StopReason("end_turn".to_string())),
            usage,
        })
    }
}

/// Extrae el conteo de tokens de un objeto `usage` del CLI.
fn parse_usage(u: &serde_json::Value) -> ChatUsage {
    let g = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    ChatUsage {
        input_tokens: g("input_tokens"),
        output_tokens: g("output_tokens"),
        cache_read_input_tokens: g("cache_read_input_tokens"),
        cache_creation_input_tokens: g("cache_creation_input_tokens"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pluma_llm_core::ChatMessage;

    fn req_multi() -> ChatRequest {
        ChatRequest {
            system: Some("Sos útil.".into()),
            messages: vec![
                ChatMessage::user("hola"),
                ChatMessage::assistant("¡hola!"),
                ChatMessage::user("¿y ahora?"),
            ],
            max_tokens: 256,
            temperature: 0.3,
        }
    }

    #[test]
    fn render_una_vuelta_es_el_mensaje() {
        let req = ChatRequest::una_vuelta("listá archivos", 64);
        assert_eq!(ClaudeCliClient::render_prompt(&req), "listá archivos");
    }

    #[test]
    fn render_multi_turno_arma_transcripto() {
        let p = ClaudeCliClient::render_prompt(&req_multi());
        assert!(p.contains("Usuario: hola"));
        assert!(p.contains("Asistente: ¡hola!"));
        assert!(p.trim_end().ends_with("Asistente:"));
    }

    #[test]
    fn model_id_refleja_el_modelo() {
        let c = ClaudeCliClient::new().with_model("claude-opus-4-8");
        assert_eq!(c.model_id(), "claude-cli:claude-opus-4-8");
        let d = ClaudeCliClient::new();
        assert_eq!(d.model_id(), "claude-cli");
    }
}
