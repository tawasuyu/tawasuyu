use super::*;

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

    fn request_rename(&mut self, path: &Path, line: usize, col: usize, new_name: &str) {
        let id = self.alloc_id();
        if let Ok(mut s) = self.state.lock() {
            s.pending_rename_ids.insert(id);
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/rename",
            "params": {
                "textDocument": { "uri": format!("file://{}", path.display()) },
                "position": { "line": line, "character": col },
                "newName": new_name
            }
        });
        self.send_raw(req.to_string());
    }

    fn latest_workspace_edit(&self) -> std::collections::HashMap<PathBuf, Vec<TextEdit>> {
        self.state.lock().map(|s| s.workspace_edit.clone()).unwrap_or_default()
    }

    fn clear_workspace_edit(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.workspace_edit.clear();
        }
    }

    fn request_document_symbols(&mut self, path: &Path) {
        let id = self.alloc_id();
        if let Ok(mut s) = self.state.lock() {
            s.pending_document_symbols_ids.insert(id);
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": format!("file://{}", path.display()) }
            }
        });
        self.send_raw(req.to_string());
    }

    fn latest_document_symbols(&self) -> Vec<DocumentSymbolEntry> {
        self.state.lock().map(|s| s.document_symbols.clone()).unwrap_or_default()
    }

    fn clear_document_symbols(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.document_symbols.clear();
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
