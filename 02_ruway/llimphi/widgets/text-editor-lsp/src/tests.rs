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
fn handle_rename_changes_map() {
    let s = make_state();
    let json = serde_json::json!({
        "id": 1,
        "result": {
            "changes": {
                "file:///tmp/a.rs": [
                    { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }, "newText": "bar" }
                ],
                "file:///tmp/b.rs": [
                    { "range": { "start": { "line": 5, "character": 4 }, "end": { "line": 5, "character": 7 } }, "newText": "bar" },
                    { "range": { "start": { "line": 10, "character": 0 }, "end": { "line": 10, "character": 3 } }, "newText": "bar" }
                ]
            }
        }
    });
    handle_rename_response(&json, &s);
    let we = s.lock().unwrap().workspace_edit.clone();
    assert_eq!(we.len(), 2);
    assert_eq!(we.get(&PathBuf::from("/tmp/a.rs")).unwrap().len(), 1);
    assert_eq!(we.get(&PathBuf::from("/tmp/b.rs")).unwrap().len(), 2);
}

#[test]
fn handle_rename_document_changes() {
    let s = make_state();
    let json = serde_json::json!({
        "id": 1,
        "result": {
            "documentChanges": [
                {
                    "textDocument": { "uri": "file:///tmp/x.rs", "version": 2 },
                    "edits": [
                        { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }, "newText": "foo" }
                    ]
                }
            ]
        }
    });
    handle_rename_response(&json, &s);
    let we = s.lock().unwrap().workspace_edit.clone();
    assert_eq!(we.len(), 1);
    assert_eq!(we.get(&PathBuf::from("/tmp/x.rs")).unwrap().len(), 1);
}

#[test]
fn handle_document_symbols_jerarquico() {
    let s = make_state();
    // Estructura: struct Foo { fn bar(), fn baz() } + fn top()
    let json = serde_json::json!({
        "id": 1,
        "result": [
            {
                "name": "Foo",
                "kind": 23, // struct
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 10, "character": 1 } },
                "selectionRange": { "start": { "line": 0, "character": 7 }, "end": { "line": 0, "character": 10 } },
                "children": [
                    {
                        "name": "bar",
                        "kind": 6, // method
                        "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 4, "character": 5 } },
                        "selectionRange": { "start": { "line": 2, "character": 7 }, "end": { "line": 2, "character": 10 } }
                    },
                    {
                        "name": "baz",
                        "kind": 6,
                        "range": { "start": { "line": 6, "character": 4 }, "end": { "line": 8, "character": 5 } },
                        "selectionRange": { "start": { "line": 6, "character": 7 }, "end": { "line": 6, "character": 10 } }
                    }
                ]
            },
            {
                "name": "top",
                "kind": 12, // function
                "range": { "start": { "line": 12, "character": 0 }, "end": { "line": 14, "character": 1 } },
                "selectionRange": { "start": { "line": 12, "character": 3 }, "end": { "line": 12, "character": 6 } }
            }
        ]
    });
    handle_document_symbols_response(&json, &s);
    let syms = s.lock().unwrap().document_symbols.clone();
    assert_eq!(syms.len(), 4, "esperaba 4 entradas flattening");

    assert_eq!(syms[0].name, "Foo");
    assert_eq!(syms[0].kind, "struct");
    assert_eq!(syms[0].line, 0);
    assert_eq!(syms[0].depth, 0);
    assert_eq!(syms[0].container, None);

    assert_eq!(syms[1].name, "bar");
    assert_eq!(syms[1].kind, "method");
    assert_eq!(syms[1].line, 2);
    assert_eq!(syms[1].depth, 1);
    assert_eq!(syms[1].container.as_deref(), Some("Foo"));

    assert_eq!(syms[2].name, "baz");
    assert_eq!(syms[2].depth, 1);
    assert_eq!(syms[2].container.as_deref(), Some("Foo"));

    assert_eq!(syms[3].name, "top");
    assert_eq!(syms[3].kind, "fn");
    assert_eq!(syms[3].depth, 0);
}

#[test]
fn handle_document_symbols_legacy_symbolinformation() {
    let s = make_state();
    // Formato viejo: SymbolInformation[] (plano + location).
    let json = serde_json::json!({
        "id": 1,
        "result": [
            {
                "name": "main",
                "kind": 12,
                "location": {
                    "uri": "file:///tmp/x.rs",
                    "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 7 } }
                }
            },
            {
                "name": "helper",
                "kind": 12,
                "containerName": "main",
                "location": {
                    "uri": "file:///tmp/x.rs",
                    "range": { "start": { "line": 5, "character": 3 }, "end": { "line": 5, "character": 9 } }
                }
            }
        ]
    });
    handle_document_symbols_response(&json, &s);
    let syms = s.lock().unwrap().document_symbols.clone();
    assert_eq!(syms.len(), 2);
    assert_eq!(syms[1].name, "helper");
    assert_eq!(syms[1].container.as_deref(), Some("main"));
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
