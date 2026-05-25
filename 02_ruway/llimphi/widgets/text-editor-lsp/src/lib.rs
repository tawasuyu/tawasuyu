//! `llimphi-widget-text-editor-lsp` — foundation para integrar LSP al
//! editor. **No implementa un client real todavía** — esto es el esqueleto
//! que un client futuro (rust-analyzer / pylsp / etc.) llenará.
//!
//! El editor mismo (`llimphi-widget-text-editor`) ya soporta diagnostics
//! vía `EditorState::set_diagnostics` y los renderiza como subrayado
//! coloreado bajo el rango. Lo que falta para LSP funcional es:
//!
//! 1. Spawn de un proceso de language server por workspace.
//! 2. JSON-RPC sobre stdin/stdout (`lsp-types` da los tipos).
//! 3. Handshake `initialize` → `initialized`.
//! 4. Lifecycle de documentos (`textDocument/didOpen` / `didChange` /
//!    `didClose`).
//! 5. Recibir `textDocument/publishDiagnostics` y mapear al
//!    `Diagnostic` del crate text-editor.
//!
//! Puntos 1-5 son ~500-1000 LOC de async Rust con tokio + parsing
//! incremental — sesión dedicada. Por ahora dejamos:
//!
//! - El **trait** [`LspClient`] que define el contrato.
//! - El **stub** [`NoopLspClient`] (no hace nada — útil para tests y
//!   para que el editor compile sin client real).
//! - Un sketch de [`rust_analyzer`] con TODOs.

#![forbid(unsafe_code)]

use std::path::Path;

use llimphi_widget_text_editor::Diagnostic;

/// Contrato que un client LSP debe cumplir para alimentar al editor.
///
/// El client guarda el estado por documento (path → version, diagnostics)
/// y emite eventos cuando el server responde. El caller llama
/// `did_open` al cargar un archivo, `did_change` después de cada
/// edición, `did_close` al cerrar, y `diagnostics` cuando arma el frame.
pub trait LspClient: Send {
    /// Diagnostics actuales para `path`. Devuelve vacío si el server no
    /// ha respondido todavía o si el path no está abierto.
    fn diagnostics(&self, path: &Path) -> Vec<Diagnostic>;

    /// Notifica al server que el documento se abrió. Idempotente.
    fn did_open(&mut self, path: &Path, language: &str, text: &str);

    /// Notifica al server que el documento cambió. El client puede
    /// optimizar enviando un diff incremental; este trait solo expone
    /// el text completo por simplicidad.
    fn did_change(&mut self, path: &Path, new_text: &str);

    /// Notifica al server que el documento se cerró.
    fn did_close(&mut self, path: &Path);
}

/// Stub que no hace nada — útil cuando no hay LSP configurado o para
/// tests. El editor sigue funcionando; los diagnostics nunca aparecen.
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

/// Esqueleto del client de rust-analyzer. Sin implementación todavía.
///
/// Plan para llenar:
///
/// 1. `RustAnalyzerClient::new(workspace_root)` spawn `rust-analyzer` con
///    `tokio::process::Command::new("rust-analyzer").spawn()`.
/// 2. Tomar `stdin` y `stdout` del child y arrancar tasks de lectura
///    y escritura JSON-RPC. Usar `lsp-types` para los structs.
/// 3. Enviar `InitializeParams` con `rootUri = file://{workspace_root}`.
///    Esperar `InitializeResult`. Enviar `initialized` notification.
/// 4. Mantener un `HashMap<PathBuf, DocumentState { version, diagnostics }>`.
/// 5. Loop de lectura: parsear cada response. En `textDocument/
///    publishDiagnostics`, mapear a `Diagnostic` del crate text-editor
///    y guardarlos en el state.
/// 6. Shutdown handshake en `Drop` (`shutdown` request + `exit`
///    notification).
pub mod rust_analyzer {
    use super::*;
    use std::path::PathBuf;

    pub struct RustAnalyzerClient {
        _workspace_root: PathBuf,
        // TODO: child: tokio::process::Child,
        // TODO: stdin_tx: mpsc::Sender<JsonRpcRequest>,
        // TODO: state: Arc<RwLock<HashMap<PathBuf, DocumentState>>>,
    }

    impl RustAnalyzerClient {
        pub fn new(workspace_root: PathBuf) -> Self {
            // TODO: spawn rust-analyzer + handshake + tasks de lectura.
            Self { _workspace_root: workspace_root }
        }
    }

    impl LspClient for RustAnalyzerClient {
        fn diagnostics(&self, _path: &Path) -> Vec<Diagnostic> {
            // TODO: leer del state.
            Vec::new()
        }
        fn did_open(&mut self, _path: &Path, _language: &str, _text: &str) {
            // TODO: enviar textDocument/didOpen.
        }
        fn did_change(&mut self, _path: &Path, _new_text: &str) {
            // TODO: enviar textDocument/didChange.
        }
        fn did_close(&mut self, _path: &Path) {
            // TODO: enviar textDocument/didClose.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
}
