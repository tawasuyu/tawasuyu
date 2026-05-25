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

/// Item de completion — mirror minimal de `lsp_types::CompletionItem`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    /// Texto a insertar. Si `None`, se usa `label`.
    pub insert_text: Option<String>,
    /// Tipo del símbolo según LSP (Function, Variable, etc.) — para
    /// mostrar un ícono. Aquí lo guardamos como string corto.
    pub kind: Option<String>,
    /// Documentación corta — el primer renglón típicamente.
    pub detail: Option<String>,
}

impl CompletionItem {
    pub fn text_to_insert(&self) -> &str {
        self.insert_text.as_deref().unwrap_or(self.label.as_str())
    }
}

/// Contrato que un client LSP debe cumplir para alimentar al editor.
pub trait LspClient: Send {
    fn diagnostics(&self, path: &Path) -> Vec<Diagnostic>;
    fn did_open(&mut self, path: &Path, language: &str, text: &str);
    fn did_change(&mut self, path: &Path, new_text: &str);
    fn did_close(&mut self, path: &Path);
    /// Dispara una petición de completions en `(line, col)` del path.
    /// Fire-and-forget; la respuesta se lee con `latest_completions`.
    fn request_completions(&mut self, path: &Path, line: usize, col: usize);
    /// Última lista de completions recibida (cualquier path/pos).
    /// Vacío hasta que el server responda. El client la limpia cuando
    /// el caller llama `clear_completions`.
    fn latest_completions(&self) -> Vec<CompletionItem>;
    /// Borra el cache de completions — útil al cerrar el popup.
    fn clear_completions(&mut self);
    /// Dispara textDocument/hover. Fire-and-forget; el caller polla
    /// `latest_hover` para leer la respuesta.
    fn request_hover(&mut self, path: &Path, line: usize, col: usize);
    /// Última hover info recibida (cualquier path/pos).
    fn latest_hover(&self) -> Option<HoverInfo>;
    /// Borra el cache de hover.
    fn clear_hover(&mut self);
    /// Dispara textDocument/definition. Fire-and-forget; el caller
    /// polla `latest_definition`.
    fn request_definition(&mut self, path: &Path, line: usize, col: usize);
    /// Última definition recibida (path destino + pos de inicio).
    fn latest_definition(&self) -> Option<DefinitionLocation>;
    fn clear_definition(&mut self);
    /// Dispara textDocument/formatting. Cuando llega la response, el
    /// caller polla `latest_text_edits` y los aplica al buffer.
    fn request_formatting(&mut self, path: &Path, tab_size: u32, insert_spaces: bool);
    /// Última lista de TextEdits recibida (de formatting o rename).
    fn latest_text_edits(&self) -> Vec<TextEdit>;
    fn clear_text_edits(&mut self);
    /// Dispara textDocument/signatureHelp. Cuando llega, el popup
    /// muestra la firma activa con el parámetro current resaltado.
    fn request_signature_help(&mut self, path: &Path, line: usize, col: usize);
    fn latest_signature_help(&self) -> Option<SignatureHelpInfo>;
    fn clear_signature_help(&mut self);
    /// Dispara textDocument/references. `include_decl` controla si la
    /// declaración misma aparece en los resultados.
    fn request_references(&mut self, path: &Path, line: usize, col: usize, include_decl: bool);
    fn latest_references(&self) -> Vec<DefinitionLocation>;
    fn clear_references(&mut self);
}

/// Info de signatureHelp activa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelpInfo {
    /// Firma activa (label completa, ej. "fn foo(x: i32, y: String) -> u64").
    pub label: String,
    /// Documentación de la firma activa.
    pub doc: Option<String>,
    /// Índice del parámetro current (0-based).
    pub active_param: usize,
    /// Labels de los parámetros — para resaltar el activo.
    pub param_labels: Vec<String>,
}

/// Edit estilo LSP: reemplazar el rango `[start..end)` por `new_text`.
/// Para apply: ordenar desc por `start` y aplicar uno por uno.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub new_text: String,
}

/// Resultado de un goto-definition: archivo destino + posición.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionLocation {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
}

/// Información de hover — espejo simplificado de `lsp_types::Hover`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverInfo {
    /// Markdown / plaintext del símbolo bajo el cursor. El render del
    /// caller lo muestra tal cual (sin parsear markdown todavía).
    pub contents: String,
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
    fn request_completions(&mut self, _: &Path, _: usize, _: usize) {}
    fn latest_completions(&self) -> Vec<CompletionItem> {
        Vec::new()
    }
    fn clear_completions(&mut self) {}
    fn request_hover(&mut self, _: &Path, _: usize, _: usize) {}
    fn latest_hover(&self) -> Option<HoverInfo> {
        None
    }
    fn clear_hover(&mut self) {}
    fn request_definition(&mut self, _: &Path, _: usize, _: usize) {}
    fn latest_definition(&self) -> Option<DefinitionLocation> {
        None
    }
    fn clear_definition(&mut self) {}
    fn request_formatting(&mut self, _: &Path, _: u32, _: bool) {}
    fn latest_text_edits(&self) -> Vec<TextEdit> {
        Vec::new()
    }
    fn clear_text_edits(&mut self) {}
    fn request_signature_help(&mut self, _: &Path, _: usize, _: usize) {}
    fn latest_signature_help(&self) -> Option<SignatureHelpInfo> {
        None
    }
    fn clear_signature_help(&mut self) {}
    fn request_references(&mut self, _: &Path, _: usize, _: usize, _: bool) {}
    fn latest_references(&self) -> Vec<DefinitionLocation> {
        Vec::new()
    }
    fn clear_references(&mut self) {}
}

// ---------------------------------------------------------------------
// Rust-analyzer client real
// ---------------------------------------------------------------------

/// State compartido: paths → versión + diagnostics actuales + última
/// lista de completions recibida.
#[derive(Default)]
struct SharedInner {
    diagnostics: HashMap<PathBuf, Vec<Diagnostic>>,
    /// Última respuesta de completions — sobreescribe cualquier
    /// request previo. El caller decide cuándo limpiar.
    completions: Vec<CompletionItem>,
    /// Última hover info recibida.
    hover: Option<HoverInfo>,
    /// Última definition recibida.
    definition: Option<DefinitionLocation>,
    /// Última lista de TextEdits (formatting / rename).
    text_edits: Vec<TextEdit>,
    /// Última signature help.
    signature_help: Option<SignatureHelpInfo>,
    /// Última lista de references.
    references: Vec<DefinitionLocation>,
    /// IDs de requests pendientes para distinguir responses; el reader
    /// usa estos sets para routear cada response al handler correcto.
    pending_completion_ids: std::collections::HashSet<i64>,
    pending_hover_ids: std::collections::HashSet<i64>,
    pending_definition_ids: std::collections::HashSet<i64>,
    pending_formatting_ids: std::collections::HashSet<i64>,
    pending_signature_help_ids: std::collections::HashSet<i64>,
    pending_references_ids: std::collections::HashSet<i64>,
}

type SharedState = Arc<Mutex<SharedInner>>;

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
        let state: SharedState = Arc::new(Mutex::new(SharedInner::default()));
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
            .and_then(|s| s.diagnostics.get(path).cloned())
            .unwrap_or_default()
    }

    fn request_completions(&mut self, path: &Path, line: usize, col: usize) {
        let id = self.alloc_id();
        if let Ok(mut s) = self.state.lock() {
            s.pending_completion_ids.insert(id);
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": format!("file://{}", path.display()) },
                "position": { "line": line, "character": col }
            }
        });
        self.send_raw(req.to_string());
    }

    fn latest_completions(&self) -> Vec<CompletionItem> {
        self.state
            .lock()
            .map(|s| s.completions.clone())
            .unwrap_or_default()
    }

    fn clear_completions(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.completions.clear();
        }
    }

    fn request_hover(&mut self, path: &Path, line: usize, col: usize) {
        let id = self.alloc_id();
        if let Ok(mut s) = self.state.lock() {
            s.pending_hover_ids.insert(id);
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": format!("file://{}", path.display()) },
                "position": { "line": line, "character": col }
            }
        });
        self.send_raw(req.to_string());
    }

    fn latest_hover(&self) -> Option<HoverInfo> {
        self.state.lock().ok().and_then(|s| s.hover.clone())
    }

    fn clear_hover(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.hover = None;
        }
    }

    fn request_definition(&mut self, path: &Path, line: usize, col: usize) {
        let id = self.alloc_id();
        if let Ok(mut s) = self.state.lock() {
            s.pending_definition_ids.insert(id);
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": format!("file://{}", path.display()) },
                "position": { "line": line, "character": col }
            }
        });
        self.send_raw(req.to_string());
    }

    fn latest_definition(&self) -> Option<DefinitionLocation> {
        self.state.lock().ok().and_then(|s| s.definition.clone())
    }

    fn clear_definition(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.definition = None;
        }
    }

    fn request_formatting(&mut self, path: &Path, tab_size: u32, insert_spaces: bool) {
        let id = self.alloc_id();
        if let Ok(mut s) = self.state.lock() {
            s.pending_formatting_ids.insert(id);
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/formatting",
            "params": {
                "textDocument": { "uri": format!("file://{}", path.display()) },
                "options": {
                    "tabSize": tab_size,
                    "insertSpaces": insert_spaces
                }
            }
        });
        self.send_raw(req.to_string());
    }

    fn latest_text_edits(&self) -> Vec<TextEdit> {
        self.state.lock().map(|s| s.text_edits.clone()).unwrap_or_default()
    }

    fn clear_text_edits(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.text_edits.clear();
        }
    }

    fn request_signature_help(&mut self, path: &Path, line: usize, col: usize) {
        let id = self.alloc_id();
        if let Ok(mut s) = self.state.lock() {
            s.pending_signature_help_ids.insert(id);
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/signatureHelp",
            "params": {
                "textDocument": { "uri": format!("file://{}", path.display()) },
                "position": { "line": line, "character": col }
            }
        });
        self.send_raw(req.to_string());
    }

    fn latest_signature_help(&self) -> Option<SignatureHelpInfo> {
        self.state.lock().ok().and_then(|s| s.signature_help.clone())
    }

    fn clear_signature_help(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.signature_help = None;
        }
    }

    fn request_references(&mut self, path: &Path, line: usize, col: usize, include_decl: bool) {
        let id = self.alloc_id();
        if let Ok(mut s) = self.state.lock() {
            s.pending_references_ids.insert(id);
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/references",
            "params": {
                "textDocument": { "uri": format!("file://{}", path.display()) },
                "position": { "line": line, "character": col },
                "context": { "includeDeclaration": include_decl }
            }
        });
        self.send_raw(req.to_string());
    }

    fn latest_references(&self) -> Vec<DefinitionLocation> {
        self.state.lock().map(|s| s.references.clone()).unwrap_or_default()
    }

    fn clear_references(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.references.clear();
        }
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
            s.diagnostics.remove(path);
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
                } else if let Some(id) = json.get("id").and_then(|i| i.as_i64()) {
                    handle_response(id, &json, &state);
                }
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
        s.diagnostics.insert(path, diagnostics);
    }
}

/// Routea una response del server al handler correspondiente según
/// qué set de pendientes la contenía.
fn handle_response(id: i64, json: &serde_json::Value, state: &SharedState) {
    let (was_completion, was_hover, was_definition, was_formatting, was_sig, was_refs) = {
        let Ok(mut s) = state.lock() else { return };
        let c = s.pending_completion_ids.remove(&id);
        let h = s.pending_hover_ids.remove(&id);
        let d = s.pending_definition_ids.remove(&id);
        let f = s.pending_formatting_ids.remove(&id);
        let sg = s.pending_signature_help_ids.remove(&id);
        let r = s.pending_references_ids.remove(&id);
        (c, h, d, f, sg, r)
    };
    if was_completion {
        handle_completion_response(json, state);
    }
    if was_hover {
        handle_hover_response(json, state);
    }
    if was_definition {
        handle_definition_response(json, state);
    }
    if was_formatting {
        handle_text_edits_response(json, state);
    }
    if was_sig {
        handle_signature_help_response(json, state);
    }
    if was_refs {
        handle_references_response(json, state);
    }
}

fn handle_references_response(json: &serde_json::Value, state: &SharedState) {
    let Some(result) = json.get("result") else { return };
    if result.is_null() {
        if let Ok(mut s) = state.lock() {
            s.references.clear();
        }
        return;
    }
    let Some(arr) = result.as_array() else { return };
    let refs: Vec<DefinitionLocation> = arr.iter().filter_map(parse_location).collect();
    if let Ok(mut s) = state.lock() {
        s.references = refs;
    }
}

/// Parsea una `Location` LSP: { uri, range } → DefinitionLocation.
fn parse_location(loc: &serde_json::Value) -> Option<DefinitionLocation> {
    let uri = loc.get("uri")?.as_str()?;
    let path = uri.strip_prefix("file://").map(PathBuf::from)?;
    let range = loc.get("range")?;
    let start = range.get("start")?;
    let line = start.get("line")?.as_u64()? as usize;
    let col = start.get("character")?.as_u64()? as usize;
    Some(DefinitionLocation { path, line, col })
}

fn handle_signature_help_response(json: &serde_json::Value, state: &SharedState) {
    let Some(result) = json.get("result") else { return };
    if result.is_null() {
        if let Ok(mut s) = state.lock() {
            s.signature_help = None;
        }
        return;
    }
    let info = parse_signature_help(result);
    if let Ok(mut s) = state.lock() {
        s.signature_help = info;
    }
}

fn parse_signature_help(result: &serde_json::Value) -> Option<SignatureHelpInfo> {
    let sigs = result.get("signatures")?.as_array()?;
    if sigs.is_empty() {
        return None;
    }
    let active_sig = result.get("activeSignature").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
    let sig = sigs.get(active_sig).or_else(|| sigs.first())?;
    let label = sig.get("label")?.as_str()?.to_string();
    let doc = sig
        .get("documentation")
        .map(stringify_hover_contents)
        .filter(|s| !s.is_empty());
    let active_param = sig
        .get("activeParameter")
        .or_else(|| result.get("activeParameter"))
        .and_then(|n| n.as_u64())
        .unwrap_or(0) as usize;
    let param_labels = sig
        .get("parameters")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let lbl = p.get("label")?;
                    if let Some(s) = lbl.as_str() {
                        Some(s.to_string())
                    } else if let Some(arr2) = lbl.as_array() {
                        let s = arr2.first()?.as_u64()? as usize;
                        let e = arr2.get(1)?.as_u64()? as usize;
                        label.get(s..e).map(String::from)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Some(SignatureHelpInfo { label, doc, active_param, param_labels })
}

fn handle_text_edits_response(json: &serde_json::Value, state: &SharedState) {
    let Some(result) = json.get("result") else { return };
    if result.is_null() {
        return;
    }
    let Some(arr) = result.as_array() else { return };
    let edits: Vec<TextEdit> = arr.iter().filter_map(parse_text_edit).collect();
    if let Ok(mut s) = state.lock() {
        s.text_edits = edits;
    }
}

fn parse_text_edit(v: &serde_json::Value) -> Option<TextEdit> {
    let range = v.get("range")?;
    let start = range.get("start")?;
    let end = range.get("end")?;
    let start_line = start.get("line")?.as_u64()? as usize;
    let start_col = start.get("character")?.as_u64()? as usize;
    let end_line = end.get("line")?.as_u64()? as usize;
    let end_col = end.get("character")?.as_u64()? as usize;
    let new_text = v.get("newText")?.as_str()?.to_string();
    Some(TextEdit { start_line, start_col, end_line, end_col, new_text })
}

fn handle_definition_response(json: &serde_json::Value, state: &SharedState) {
    let Some(result) = json.get("result") else { return };
    if result.is_null() {
        return;
    }
    // `result` puede ser:
    // - Location          { uri, range }
    // - Location[]
    // - LocationLink[]    { targetUri, targetSelectionRange }
    // Tomamos la primera location en cualquier caso.
    let loc_value = if result.is_array() {
        result.as_array().and_then(|a| a.first()).cloned()
    } else {
        Some(result.clone())
    };
    let Some(loc) = loc_value else { return };

    let (uri, range) = if let Some(u) = loc.get("uri") {
        (u, loc.get("range"))
    } else if let Some(u) = loc.get("targetUri") {
        (
            u,
            loc.get("targetSelectionRange").or_else(|| loc.get("targetRange")),
        )
    } else {
        return;
    };
    let Some(uri) = uri.as_str() else { return };
    let path = match uri.strip_prefix("file://") {
        Some(p) => PathBuf::from(p),
        None => return,
    };
    let Some(range) = range else { return };
    let Some(start) = range.get("start") else { return };
    let line = start.get("line").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
    let col = start.get("character").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
    if let Ok(mut s) = state.lock() {
        s.definition = Some(DefinitionLocation { path, line, col });
    }
}

fn handle_completion_response(json: &serde_json::Value, state: &SharedState) {
    let Some(result) = json.get("result") else { return };
    let items_arr = if let Some(arr) = result.as_array() {
        arr.clone()
    } else if let Some(items) = result.get("items").and_then(|i| i.as_array()) {
        items.clone()
    } else {
        return;
    };
    let completions: Vec<CompletionItem> = items_arr.iter().filter_map(parse_completion).collect();
    if let Ok(mut s) = state.lock() {
        s.completions = completions;
    }
}

fn handle_hover_response(json: &serde_json::Value, state: &SharedState) {
    let Some(result) = json.get("result") else { return };
    if result.is_null() {
        if let Ok(mut s) = state.lock() {
            s.hover = None;
        }
        return;
    }
    let info = parse_hover(result);
    if let Ok(mut s) = state.lock() {
        s.hover = info;
    }
}

/// `contents` en LSP puede ser:
/// - String
/// - { kind: "markdown"|"plaintext", value: String }
/// - Array de los anteriores (deprecated pero algunos servers lo mandan)
/// - { language: ..., value: ... } (legacy MarkedString)
fn parse_hover(result: &serde_json::Value) -> Option<HoverInfo> {
    let contents = result.get("contents")?;
    let text = stringify_hover_contents(contents);
    if text.is_empty() {
        None
    } else {
        Some(HoverInfo { contents: text })
    }
}

fn stringify_hover_contents(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => {
            // { kind, value } o { language, value }
            map.get("value")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string()
        }
        serde_json::Value::Array(arr) => arr
            .iter()
            .map(stringify_hover_contents)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn parse_completion(v: &serde_json::Value) -> Option<CompletionItem> {
    let label = v.get("label")?.as_str()?.to_string();
    let insert_text = v
        .get("insertText")
        .and_then(|s| s.as_str())
        .map(String::from);
    let kind = v
        .get("kind")
        .and_then(|k| k.as_u64())
        .map(|n| completion_kind_label(n).to_string());
    let detail = v
        .get("detail")
        .and_then(|d| d.as_str())
        .map(String::from);
    Some(CompletionItem { label, insert_text, kind, detail })
}

/// Etiqueta corta para el CompletionItemKind de LSP (1..25).
fn completion_kind_label(k: u64) -> &'static str {
    match k {
        1 => "Text",
        2 => "Method",
        3 => "Function",
        4 => "Ctor",
        5 => "Field",
        6 => "Var",
        7 => "Class",
        8 => "Iface",
        9 => "Mod",
        10 => "Prop",
        11 => "Unit",
        12 => "Value",
        13 => "Enum",
        14 => "Keyword",
        15 => "Snip",
        16 => "Color",
        17 => "File",
        18 => "Ref",
        19 => "Folder",
        20 => "EnumMember",
        21 => "Const",
        22 => "Struct",
        23 => "Event",
        24 => "Op",
        25 => "TypeParam",
        _ => "?",
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
    fn parse_completion_minimo() {
        let v = serde_json::json!({
            "label": "to_string",
            "insertText": "to_string()",
            "kind": 2,
            "detail": "fn(&self) -> String"
        });
        let c = parse_completion(&v).unwrap();
        assert_eq!(c.label, "to_string");
        assert_eq!(c.insert_text.as_deref(), Some("to_string()"));
        assert_eq!(c.kind.as_deref(), Some("Method"));
        assert_eq!(c.detail.as_deref(), Some("fn(&self) -> String"));
    }

    #[test]
    fn parse_hover_string_simple() {
        let v = serde_json::json!({ "contents": "hola" });
        let h = parse_hover(&v).unwrap();
        assert_eq!(h.contents, "hola");
    }

    #[test]
    fn parse_hover_marked_object() {
        let v = serde_json::json!({
            "contents": { "kind": "markdown", "value": "**fn**(x: i32) -> i32" }
        });
        let h = parse_hover(&v).unwrap();
        assert_eq!(h.contents, "**fn**(x: i32) -> i32");
    }

    #[test]
    fn parse_hover_array_concatena() {
        let v = serde_json::json!({
            "contents": ["primero", { "value": "segundo" }, ""]
        });
        let h = parse_hover(&v).unwrap();
        assert_eq!(h.contents, "primero\nsegundo");
    }

    #[test]
    fn parse_hover_vacio_devuelve_none() {
        let v = serde_json::json!({ "contents": "" });
        assert!(parse_hover(&v).is_none());
    }

    #[test]
    fn parse_completion_sin_insert_text_usa_label() {
        let v = serde_json::json!({ "label": "main" });
        let c = parse_completion(&v).unwrap();
        assert_eq!(c.text_to_insert(), "main");
    }

    fn make_state() -> SharedState {
        Arc::new(Mutex::new(SharedInner::default()))
    }

    #[test]
    fn handle_references_response_array() {
        let s = make_state();
        let json = serde_json::json!({
            "id": 1,
            "result": [
                { "uri": "file:///tmp/a.rs", "range": { "start": { "line": 1, "character": 2 }, "end": { "line": 1, "character": 5 } } },
                { "uri": "file:///tmp/b.rs", "range": { "start": { "line": 10, "character": 0 }, "end": { "line": 10, "character": 3 } } }
            ]
        });
        handle_references_response(&json, &s);
        let refs = s.lock().unwrap().references.clone();
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].path, PathBuf::from("/tmp/a.rs"));
        assert_eq!(refs[0].line, 1);
        assert_eq!(refs[1].path, PathBuf::from("/tmp/b.rs"));
        assert_eq!(refs[1].line, 10);
    }

    #[test]
    fn parse_signature_help_basic() {
        let result = serde_json::json!({
            "signatures": [{
                "label": "fn foo(x: i32, y: String) -> u64",
                "parameters": [
                    { "label": "x: i32" },
                    { "label": "y: String" }
                ]
            }],
            "activeSignature": 0,
            "activeParameter": 1
        });
        let info = parse_signature_help(&result).unwrap();
        assert_eq!(info.label, "fn foo(x: i32, y: String) -> u64");
        assert_eq!(info.active_param, 1);
        assert_eq!(info.param_labels, vec!["x: i32", "y: String"]);
    }

    #[test]
    fn parse_signature_help_offset_label() {
        // Label como [start, end] dentro del label de la firma.
        let result = serde_json::json!({
            "signatures": [{
                "label": "foo(x, y)",
                "parameters": [
                    { "label": [4, 5] },
                    { "label": [7, 8] }
                ]
            }]
        });
        let info = parse_signature_help(&result).unwrap();
        assert_eq!(info.param_labels, vec!["x", "y"]);
    }

    #[test]
    fn parse_text_edit_basic() {
        let v = serde_json::json!({
            "range": {
                "start": { "line": 1, "character": 0 },
                "end":   { "line": 1, "character": 4 }
            },
            "newText": "let "
        });
        let e = parse_text_edit(&v).unwrap();
        assert_eq!(e.start_line, 1);
        assert_eq!(e.start_col, 0);
        assert_eq!(e.end_line, 1);
        assert_eq!(e.end_col, 4);
        assert_eq!(e.new_text, "let ");
    }

    #[test]
    fn handle_text_edits_response_array() {
        let s = make_state();
        let json = serde_json::json!({
            "id": 1,
            "result": [
                { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }, "newText": "fn " },
                { "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 5 } }, "newText": "" }
            ]
        });
        handle_text_edits_response(&json, &s);
        let edits = s.lock().unwrap().text_edits.clone();
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn handle_definition_location_simple() {
        let s = make_state();
        let json = serde_json::json!({
            "id": 1,
            "result": {
                "uri": "file:///tmp/x.rs",
                "range": {
                    "start": { "line": 10, "character": 4 },
                    "end":   { "line": 10, "character": 9 }
                }
            }
        });
        handle_definition_response(&json, &s);
        let d = s.lock().unwrap().definition.clone().unwrap();
        assert_eq!(d.path, PathBuf::from("/tmp/x.rs"));
        assert_eq!(d.line, 10);
        assert_eq!(d.col, 4);
    }

    #[test]
    fn handle_definition_location_link_array() {
        let s = make_state();
        let json = serde_json::json!({
            "id": 1,
            "result": [
                {
                    "targetUri": "file:///tmp/y.rs",
                    "targetSelectionRange": {
                        "start": { "line": 0, "character": 7 },
                        "end":   { "line": 0, "character": 12 }
                    }
                }
            ]
        });
        handle_definition_response(&json, &s);
        let d = s.lock().unwrap().definition.clone().unwrap();
        assert_eq!(d.path, PathBuf::from("/tmp/y.rs"));
        assert_eq!(d.line, 0);
        assert_eq!(d.col, 7);
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
