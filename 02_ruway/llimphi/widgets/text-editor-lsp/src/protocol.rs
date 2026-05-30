//! Parsers y handlers JSON-RPC de las respuestas/notificaciones LSP.

use super::*;

pub(crate) fn handle_publish_diagnostics(json: &serde_json::Value, state: &SharedState) {
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
pub(crate) fn handle_response(id: i64, json: &serde_json::Value, state: &SharedState) {
    let flags = {
        let Ok(mut s) = state.lock() else { return };
        (
            s.pending_completion_ids.remove(&id),
            s.pending_hover_ids.remove(&id),
            s.pending_definition_ids.remove(&id),
            s.pending_formatting_ids.remove(&id),
            s.pending_signature_help_ids.remove(&id),
            s.pending_references_ids.remove(&id),
            s.pending_rename_ids.remove(&id),
            s.pending_document_symbols_ids.remove(&id),
        )
    };
    let (was_completion, was_hover, was_def, was_fmt, was_sig, was_refs, was_rename, was_doc_sym) =
        flags;
    if was_completion {
        handle_completion_response(json, state);
    }
    if was_hover {
        handle_hover_response(json, state);
    }
    if was_def {
        handle_definition_response(json, state);
    }
    if was_fmt {
        handle_text_edits_response(json, state);
    }
    if was_sig {
        handle_signature_help_response(json, state);
    }
    if was_refs {
        handle_references_response(json, state);
    }
    if was_rename {
        handle_rename_response(json, state);
    }
    if was_doc_sym {
        handle_document_symbols_response(json, state);
    }
}

/// Parsea la respuesta de `textDocument/documentSymbol`. Devuelve dos
/// formatos posibles según la versión del server:
///
/// - `DocumentSymbol[]` (jerárquico, moderno) — el que usa rust-analyzer.
/// - `SymbolInformation[]` (plano, legacy) — fallback razonable.
///
/// Ambos se flatten a `Vec<DocumentSymbolEntry>` con depth para que el
/// caller pueda indentar visualmente.
pub(crate) fn handle_document_symbols_response(json: &serde_json::Value, state: &SharedState) {
    let Some(result) = json.get("result") else { return };
    if result.is_null() {
        if let Ok(mut s) = state.lock() {
            s.document_symbols.clear();
        }
        return;
    }
    let mut out: Vec<DocumentSymbolEntry> = Vec::new();
    if let Some(arr) = result.as_array() {
        for item in arr {
            // Distingue por la presencia de "selectionRange" (sólo en
            // DocumentSymbol). SymbolInformation tiene "location" en
            // su lugar.
            if item.get("selectionRange").is_some() {
                flatten_document_symbol(item, None, 0, &mut out);
            } else if item.get("location").is_some() {
                if let Some(entry) = parse_symbol_information(item) {
                    out.push(entry);
                }
            }
        }
    }
    if let Ok(mut s) = state.lock() {
        s.document_symbols = out;
    }
}

/// Flatten recursivo de `DocumentSymbol`. `parent` es el nombre del
/// contenedor (para que `container` quede poblado en métodos/campos).
pub(crate) fn flatten_document_symbol(
    node: &serde_json::Value,
    parent: Option<&str>,
    depth: u32,
    out: &mut Vec<DocumentSymbolEntry>,
) {
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let kind_num = node.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
    let kind = symbol_kind_label(kind_num);
    // `selectionRange.start` es la pos del identificador (lo que el
    // usuario quiere ver al saltar). `range.start` apuntaría al `{` de
    // la definición — menos útil para outline.
    let (line, col) = node
        .get("selectionRange")
        .and_then(|r| r.get("start"))
        .and_then(parse_position)
        .or_else(|| node.get("range").and_then(|r| r.get("start")).and_then(parse_position))
        .unwrap_or((0, 0));
    out.push(DocumentSymbolEntry {
        name: name.clone(),
        kind,
        line,
        col,
        container: parent.map(|s| s.to_string()),
        depth,
    });
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for child in children {
            flatten_document_symbol(child, Some(&name), depth + 1, out);
        }
    }
}

pub(crate) fn parse_symbol_information(item: &serde_json::Value) -> Option<DocumentSymbolEntry> {
    let name = item.get("name")?.as_str()?.to_string();
    let kind_num = item.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
    let location = item.get("location")?;
    let (line, col) = location.get("range").and_then(|r| r.get("start")).and_then(parse_position)?;
    let container = item
        .get("containerName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Some(DocumentSymbolEntry {
        name,
        kind: symbol_kind_label(kind_num),
        line,
        col,
        container,
        depth: 0,
    })
}

pub(crate) fn parse_position(p: &serde_json::Value) -> Option<(usize, usize)> {
    let line = p.get("line")?.as_u64()? as usize;
    let col = p.get("character")?.as_u64()? as usize;
    Some((line, col))
}

/// Mapea el `SymbolKind` numérico del LSP a la etiqueta corta que el
/// outline pinta. Sólo cubre las que el usuario suele ver — el resto
/// va a `"sym"`. Lista canónica: <https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#symbolKind>
pub(crate) fn symbol_kind_label(kind: u64) -> String {
    match kind {
        2 => "mod",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "ctor",
        10 => "enum",
        11 => "iface",
        12 => "fn",
        13 => "var",
        14 => "const",
        15 => "str",
        18 => "arr",
        22 => "variant",
        23 => "struct",
        26 => "type",
        _ => "sym",
    }
    .to_string()
}

pub(crate) fn handle_rename_response(json: &serde_json::Value, state: &SharedState) {
    let Some(result) = json.get("result") else { return };
    if result.is_null() {
        return;
    }
    let mut map: HashMap<PathBuf, Vec<TextEdit>> = HashMap::new();
    // changes: { uri → TextEdit[] }
    if let Some(changes) = result.get("changes").and_then(|c| c.as_object()) {
        for (uri, edits_val) in changes {
            let Some(path) = uri.strip_prefix("file://").map(PathBuf::from) else { continue };
            let Some(arr) = edits_val.as_array() else { continue };
            let edits: Vec<TextEdit> = arr.iter().filter_map(parse_text_edit).collect();
            map.insert(path, edits);
        }
    }
    // documentChanges: [{ textDocument: { uri }, edits: [...] }] — más nuevo.
    if let Some(docs) = result.get("documentChanges").and_then(|c| c.as_array()) {
        for doc in docs {
            let Some(uri) = doc
                .get("textDocument")
                .and_then(|t| t.get("uri"))
                .and_then(|u| u.as_str())
            else {
                continue;
            };
            let Some(path) = uri.strip_prefix("file://").map(PathBuf::from) else { continue };
            let Some(arr) = doc.get("edits").and_then(|e| e.as_array()) else { continue };
            let edits: Vec<TextEdit> = arr.iter().filter_map(parse_text_edit).collect();
            map.entry(path).or_default().extend(edits);
        }
    }
    if let Ok(mut s) = state.lock() {
        s.workspace_edit = map;
    }
}

pub(crate) fn handle_references_response(json: &serde_json::Value, state: &SharedState) {
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
pub(crate) fn parse_location(loc: &serde_json::Value) -> Option<DefinitionLocation> {
    let uri = loc.get("uri")?.as_str()?;
    let path = uri.strip_prefix("file://").map(PathBuf::from)?;
    let range = loc.get("range")?;
    let start = range.get("start")?;
    let line = start.get("line")?.as_u64()? as usize;
    let col = start.get("character")?.as_u64()? as usize;
    Some(DefinitionLocation { path, line, col })
}

pub(crate) fn handle_signature_help_response(json: &serde_json::Value, state: &SharedState) {
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

pub(crate) fn parse_signature_help(result: &serde_json::Value) -> Option<SignatureHelpInfo> {
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

pub(crate) fn handle_text_edits_response(json: &serde_json::Value, state: &SharedState) {
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

pub(crate) fn parse_text_edit(v: &serde_json::Value) -> Option<TextEdit> {
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

pub(crate) fn handle_definition_response(json: &serde_json::Value, state: &SharedState) {
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

pub(crate) fn handle_completion_response(json: &serde_json::Value, state: &SharedState) {
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

pub(crate) fn handle_hover_response(json: &serde_json::Value, state: &SharedState) {
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
pub(crate) fn parse_hover(result: &serde_json::Value) -> Option<HoverInfo> {
    let contents = result.get("contents")?;
    let text = stringify_hover_contents(contents);
    if text.is_empty() {
        None
    } else {
        Some(HoverInfo { contents: text })
    }
}

pub(crate) fn stringify_hover_contents(v: &serde_json::Value) -> String {
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

pub(crate) fn parse_completion(v: &serde_json::Value) -> Option<CompletionItem> {
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
pub(crate) fn completion_kind_label(k: u64) -> &'static str {
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

pub(crate) fn parse_lsp_diagnostic(d: &serde_json::Value) -> Option<Diagnostic> {
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
