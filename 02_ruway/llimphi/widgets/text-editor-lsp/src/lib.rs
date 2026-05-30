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
    /// Dispara textDocument/rename con `new_name` como nuevo identificador.
    fn request_rename(&mut self, path: &Path, line: usize, col: usize, new_name: &str);
    /// Última WorkspaceEdit recibida (rename o code actions). Mapeado a
    /// `path → Vec<TextEdit>` por simplicidad.
    fn latest_workspace_edit(&self) -> std::collections::HashMap<PathBuf, Vec<TextEdit>>;
    fn clear_workspace_edit(&mut self);

    /// Dispara textDocument/documentSymbol. La respuesta llega
    /// asincrónica; el caller la recoge con [`latest_document_symbols`].
    fn request_document_symbols(&mut self, path: &Path);
    /// Última respuesta de documentSymbol — lista plana flattening del
    /// árbol jerárquico que devuelve el server. Orden: top-down,
    /// children en orden de aparición. `depth` refleja la profundidad
    /// para que el caller indente visualmente.
    fn latest_document_symbols(&self) -> Vec<DocumentSymbolEntry>;
    fn clear_document_symbols(&mut self);
}

/// Una entrada flattening del árbol `DocumentSymbol` del LSP. Espejo
/// mínimo que evita arrastrar `lsp_types::SymbolKind` a los hosts —
/// `kind` viene ya como string corta (`"fn"`, `"struct"`, `"method"`, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSymbolEntry {
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub col: usize,
    pub container: Option<String>,
    pub depth: u32,
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
    fn request_rename(&mut self, _: &Path, _: usize, _: usize, _: &str) {}
    fn latest_workspace_edit(&self) -> std::collections::HashMap<PathBuf, Vec<TextEdit>> {
        std::collections::HashMap::new()
    }
    fn clear_workspace_edit(&mut self) {}
    fn request_document_symbols(&mut self, _: &Path) {}
    fn latest_document_symbols(&self) -> Vec<DocumentSymbolEntry> {
        Vec::new()
    }
    fn clear_document_symbols(&mut self) {}
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
    /// Última WorkspaceEdit (de rename). Mapeo path → edits.
    workspace_edit: HashMap<PathBuf, Vec<TextEdit>>,
    /// Última lista de document symbols (flattened del árbol que devuelve
    /// el server). Se sobreescribe en cada request.
    document_symbols: Vec<DocumentSymbolEntry>,
    /// IDs de requests pendientes para distinguir responses; el reader
    /// usa estos sets para routear cada response al handler correcto.
    pending_completion_ids: std::collections::HashSet<i64>,
    pending_hover_ids: std::collections::HashSet<i64>,
    pending_definition_ids: std::collections::HashSet<i64>,
    pending_formatting_ids: std::collections::HashSet<i64>,
    pending_signature_help_ids: std::collections::HashSet<i64>,
    pending_references_ids: std::collections::HashSet<i64>,
    pending_rename_ids: std::collections::HashSet<i64>,
    pending_document_symbols_ids: std::collections::HashSet<i64>,
}

type SharedState = Arc<Mutex<SharedInner>>;

// Cliente y protocolo partidos del monolito (regla dura #1, 1660 LOC):
// `client` (RustAnalyzerClient + impls), `protocol` (parsers/handlers JSON-RPC).
mod client;
mod protocol;

pub use client::RustAnalyzerClient;
pub(crate) use protocol::*;

#[cfg(test)]
mod tests;
