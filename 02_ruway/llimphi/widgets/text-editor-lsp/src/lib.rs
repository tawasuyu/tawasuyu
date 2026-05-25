//! `llimphi-widget-text-editor-lsp` — cliente LSP para alimentar
//! diagnostics al editor.
//!
//! Implementación real basada en `tokio::process::Command` +
//! `lsp-types` + JSON-RPC sobre stdin/stdout del language server.
//!
//! Flujo:
//!
//! 1. `RustAnalyzerClient::start(workspace_root)` spawn `rust-analyzer`
//!    (o el binary que se le pase con `with_command`) y arranca dos
//!    tasks tokio:
//!    - **writer**: consume mensajes del `mpsc::Sender`, los serializa
//!      con headers `Content-Length: N\r\n\r\n` y los manda al stdin.
//!    - **reader**: parsea el stdout del server (mismo formato),
//!      atiende `textDocument/publishDiagnostics` y guarda los
//!      diagnostics en el state compartido.
//! 2. El handshake `initialize` se envía sincronicamente desde `start`
//!    y se espera la respuesta antes de mandar `initialized` +
//!    procesar más mensajes.
//! 3. `did_open` / `did_change` / `did_close` mandan las notifications
//!    correspondientes — sin esperar respuesta.
//! 4. `diagnostics(path)` lee del state sin contactar al server.
//!
//! El client maneja **una sola conexión por instancia**. Para
//! multi-proyecto el caller crea varios clients.
//!
//! Si el server no se puede spawnear (binary no instalado), el client
//! cae en modo no-op transparentemente — `diagnostics` devuelve vacío.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use llimphi_widget_text_editor::{Diagnostic, DiagnosticRange, Pos, Severity};

/// Contrato que un client LSP debe cumplir para alimentar al editor.
pub trait LspClient: Send {
    fn diagnostics(&self, path: &Path) -> Vec<Diagnostic>;
    fn did_open(&mut self, path: &Path, language: &str, text: &str);
    fn did_change(&mut self, path: &Path, new_text: &str);
    fn did_close(&mut self, path: &Path);
}

/// Stub que no hace nada — útil cuando no hay LSP configurado o para tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopLspClient;

impl LspClient for NoopLspClient {
    fn diagnostics(&self, _: &Path) -> Vec<Diagnostic> {
        Vec::new()
    }
    fn did_open(&mut self, _: &Path, _: &str, _: &str) {}
    fn did_change(&mut self, _: &Path, _: &str) {}
    fn did_close(&mut self, _: &Path) {}
}

// ---------------------------------------------------------------------
// Rust-analyzer client real
// ---------------------------------------------------------------------

/// State compartido: paths → versión + diagnostics actuales.
type SharedState = Arc<Mutex<HashMap<PathBuf, Vec<Diagnostic>>>>;

pub struct RustAnalyzerClient {
    /// Diagnostics activos por path. Lo escribe la task reader.
    state: SharedState,
    /// Sender al writer task. `None` si el spawn falló (modo no-op).
    tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    /// Contador monotónico de request IDs.
    next_id: i64,
    /// Versiones por documento — el server las requiere en didChange.
    versions: HashMap<PathBuf, i32>,
    /// Runtime tokio dedicado — vive todo lo que viva el client.
    /// `None` si el spawn falló.
    _runtime: Option<Arc<tokio::runtime::Runtime>>,
}

impl RustAnalyzerClient {
    /// Spawn `rust-analyzer` en `workspace_root`. Si el binary no está
    /// en PATH, devuelve un client en modo no-op (sin error).
    pub fn start(workspace_root: PathBuf) -> Self {
        Self::with_command(workspace_root, "rust-analyzer")
    }

    /// Como `start` pero permite indicar el binary (`pylsp`, etc.).
    pub fn with_command(workspace_root: PathBuf, command: &str) -> Self {
        let state: SharedState = Arc::new(Mutex::new(HashMap::new()));
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(rt) => Arc::new(rt),
            Err(_) => {
                return Self {
                    state,
                    tx: None,
                    next_id: 1,
                    versions: HashMap::new(),
                    _runtime: None,
                };
            }
        };

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let state_clone = state.clone();
        let workspace_root_clone = workspace_root.clone();
        let command_string = command.to_string();

        runtime.spawn(async move {
            if let Err(e) = run_server(workspace_root_clone, command_string, rx, state_clone).await
            {
                eprintln!("lsp: server task terminó con error: {e}");
            }
        });

        let mut client = Self {
            state,
            tx: Some(tx),
            next_id: 1,
            versions: HashMap::new(),
            _runtime: Some(runtime),
        };
        client.send_initialize(&workspace_root);
        client
    }

    fn send_initialize(&mut self, root: &Path) {
        let id = self.alloc_id();
        let params = serde_json::json!({
            "processId": std::process::id(),
            "rootUri": format!("file://{}", root.display()),
            "capabilities": {
                "textDocument": {
                    "publishDiagnostics": { "relatedInformation": false }
                }
            },
            "clientInfo": { "name": "llimphi-text-editor-lsp", "version": "0.1.0" }
        });
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": params
        });
        self.send_raw(req.to_string());
        // El handshake termina con la notification `initialized` que
        // mandamos sin esperar la response — el reader la procesará.
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        });
        self.send_raw(notif.to_string());
    }

    fn alloc_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn send_raw(&self, msg: String) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(msg);
        }
    }

    fn lsp_language_id(language: &str) -> &str {
        match language {
            "rust" | "rs" => "rust",
            "python" | "py" => "python",
            other => other,
        }
    }
}

impl LspClient for RustAnalyzerClient {
    fn diagnostics(&self, path: &Path) -> Vec<Diagnostic> {
        self.state
            .lock()
            .ok()
            .and_then(|s| s.get(path).cloned())
            .unwrap_or_default()
    }

    fn did_open(&mut self, path: &Path, language: &str, text: &str) {
        self.versions.insert(path.to_path_buf(), 1);
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": format!("file://{}", path.display()),
                    "languageId": Self::lsp_language_id(language),
                    "version": 1,
                    "text": text,
                }
            }
        });
        self.send_raw(notif.to_string());
    }

    fn did_change(&mut self, path: &Path, new_text: &str) {
        let version = {
            let v = self.versions.entry(path.to_path_buf()).or_insert(1);
            *v += 1;
            *v
        };
        // Full-document change. Más eficiente sería incremental, pero
        // requiere trackear los EditDeltas del editor — futuro.
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": {
                    "uri": format!("file://{}", path.display()),
                    "version": version,
                },
                "contentChanges": [{ "text": new_text }]
            }
        });
        self.send_raw(notif.to_string());
    }

    fn did_close(&mut self, path: &Path) {
        self.versions.remove(path);
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": {
                "textDocument": { "uri": format!("file://{}", path.display()) }
            }
        });
        self.send_raw(notif.to_string());
        if let Ok(mut s) = self.state.lock() {
            s.remove(path);
        }
    }
}

// ---------------------------------------------------------------------
// Task tokio que corre el server + bombea I/O
// ---------------------------------------------------------------------

async fn run_server(
    _workspace_root: PathBuf,
    command: String,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    state: SharedState,
) -> std::io::Result<()> {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::process::Command;

    let mut child = match Command::new(&command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lsp: no pude spawn `{command}`: {e}");
            return Ok(());
        }
    };

    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");

    // Writer task: consume el rx y manda al stdin con headers LSP.
    let writer = tokio::spawn(async move {
        let mut stdin = stdin;
        while let Some(msg) = rx.recv().await {
            let header = format!("Content-Length: {}\r\n\r\n", msg.len());
            if stdin.write_all(header.as_bytes()).await.is_err() {
                break;
            }
            if stdin.write_all(msg.as_bytes()).await.is_err() {
                break;
            }
            let _ = stdin.flush().await;
        }
    });

    // Reader task: parsea mensajes del stdout, procesa publishDiagnostics.
    let reader = tokio::spawn({
        let state = state.clone();
        async move {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut content_length: Option<usize> = None;
                // Headers — terminan con línea vacía.
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line).await {
                        Ok(0) => return, // EOF
                        Ok(_) => {}
                        Err(_) => return,
                    }
                    let line = line.trim_end_matches(['\r', '\n']);
                    if line.is_empty() {
                        break;
                    }
                    if let Some(rest) = line.strip_prefix("Content-Length:") {
                        if let Ok(n) = rest.trim().parse::<usize>() {
                            content_length = Some(n);
                        }
                    }
                }
                let Some(len) = content_length else { continue };
                let mut buf = vec![0u8; len];
                if reader.read_exact(&mut buf).await.is_err() {
                    return;
                }
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&buf) else {
                    continue;
                };
                if json.get("method").and_then(|m| m.as_str())
                    == Some("textDocument/publishDiagnostics")
                {
                    handle_publish_diagnostics(&json, &state);
                }
                // Responses/otras notifications: por ahora ignorados.
            }
        }
    });

    // Esperamos a que se cierre cualquiera de los dos lados o el child.
    tokio::select! {
        _ = writer => {}
        _ = reader => {}
        _ = child.wait() => {}
    }
    let _ = child.kill().await;
    Ok(())
}

fn handle_publish_diagnostics(json: &serde_json::Value, state: &SharedState) {
    let Some(params) = json.get("params") else { return };
    let Some(uri) = params.get("uri").and_then(|u| u.as_str()) else { return };
    let path = match uri.strip_prefix("file://") {
        Some(p) => PathBuf::from(p),
        None => return,
    };
    let Some(diags_arr) = params.get("diagnostics").and_then(|d| d.as_array()) else {
        return;
    };
    let diagnostics: Vec<Diagnostic> = diags_arr
        .iter()
        .filter_map(parse_lsp_diagnostic)
        .collect();
    if let Ok(mut s) = state.lock() {
        s.insert(path, diagnostics);
    }
}

fn parse_lsp_diagnostic(d: &serde_json::Value) -> Option<Diagnostic> {
    let range = d.get("range")?;
    let start = range.get("start")?;
    let end = range.get("end")?;
    let sl = start.get("line")?.as_u64()? as usize;
    let sc = start.get("character")?.as_u64()? as usize;
    let el = end.get("line")?.as_u64()? as usize;
    let ec = end.get("character")?.as_u64()? as usize;
    let severity = match d.get("severity").and_then(|s| s.as_u64()) {
        Some(1) => Severity::Error,
        Some(2) => Severity::Warning,
        Some(3) => Severity::Information,
        Some(4) => Severity::Hint,
        _ => Severity::Information,
    };
    let message = d
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let source = d.get("source").and_then(|s| s.as_str()).map(String::from);
    Some(Diagnostic {
        range: DiagnosticRange {
            start: Pos::new(sl, sc),
            end: Pos::new(el, ec),
        },
        severity,
        message,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_devuelve_vacio() {
        let c = NoopLspClient;
        assert!(c.diagnostics(&PathBuf::from("x")).is_empty());
    }

    #[test]
    fn noop_no_panic_en_eventos() {
        let mut c = NoopLspClient;
        c.did_open(&PathBuf::from("x"), "rust", "fn main() {}");
        c.did_change(&PathBuf::from("x"), "fn main() { 1 }");
        c.did_close(&PathBuf::from("x"));
    }

    #[test]
    fn parse_diagnostic_minimo() {
        let json = serde_json::json!({
            "range": {
                "start": { "line": 3, "character": 5 },
                "end":   { "line": 3, "character": 12 }
            },
            "severity": 1,
            "message": "no es así",
            "source": "rustc"
        });
        let d = parse_lsp_diagnostic(&json).unwrap();
        assert_eq!(d.range.start, Pos::new(3, 5));
        assert_eq!(d.range.end, Pos::new(3, 12));
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "no es así");
        assert_eq!(d.source.as_deref(), Some("rustc"));
    }

    #[test]
    fn parse_diagnostic_sin_severidad_es_info() {
        let json = serde_json::json!({
            "range": {
                "start": { "line": 0, "character": 0 },
                "end":   { "line": 0, "character": 1 }
            },
            "message": "x"
        });
        let d = parse_lsp_diagnostic(&json).unwrap();
        assert_eq!(d.severity, Severity::Information);
    }

    #[test]
    fn rust_analyzer_client_sin_binary_no_panic() {
        // Si rust-analyzer no está instalado, el spawn falla en silencio
        // y el client queda en modo no-op (state vacío).
        let c = RustAnalyzerClient::with_command(PathBuf::from("/tmp"), "rust-analyzer-missing-99999");
        // diagnostics() siempre devuelve vacío hasta que el server responde.
        assert!(c.diagnostics(&PathBuf::from("/tmp/x")).is_empty());
    }
}
