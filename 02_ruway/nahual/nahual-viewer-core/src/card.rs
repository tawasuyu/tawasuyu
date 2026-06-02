//! `card` — núcleo agnóstico del visor de card de nahual (parseo + tipos de preview). El render vive en `nahual-card-viewer-llimphi`.

use card_core::{Card, CardKind, Payload, Supervision};
use std::fmt::Write as _;
use std::path::Path;

/// Estado del visor. La Card se boxea (es grande) y se guarda parseada
/// para que el render no re-parsee en cada frame.
pub enum CardPreview {
    /// Sin archivo / no es una Card.
    Empty,
    /// Card parseada, lista para presentar.
    Card(Box<Card>),
    /// El archivo decía ser Card (lens `card`) pero no parseó.
    Error(String),
}

impl Default for CardPreview {
    fn default() -> Self {
        CardPreview::Empty
    }
}

/// Lee y parsea la Card del archivo. Intenta JSON y, si falla, TOML —
/// el shell ya discernió el contenido como Card, pero no asume el
/// formato textual. Sync: una Card pesa KB, no MB.
pub fn load_card(path: &Path) -> CardPreview {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return CardPreview::Error(e.to_string()),
    };
    match Card::from_json(&src).or_else(|_| Card::from_toml(&src)) {
        Ok(card) => CardPreview::Card(Box::new(card)),
        Err(e) => CardPreview::Error(e.to_string()),
    }
}

/// Resume la Card en líneas `clave  valor`. Sólo los campos con señal:
/// los vacíos/default se omiten para no ahogar lo relevante.
pub fn summarize(card: &Card) -> String {
    let mut s = String::new();
    let row = |s: &mut String, k: &str, v: &str| {
        // `{:<13}` alinea las claves en una columna fija.
        let _ = writeln!(s, "{k:<13}{v}");
    };

    row(&mut s, "label", &card.label);
    row(&mut s, "id", &card.id.to_string());
    if let Some(lineage) = card.lineage {
        row(&mut s, "lineage", &lineage.to_string());
    }
    row(
        &mut s,
        "kind",
        match card.kind {
            CardKind::Ente => "ente (proceso)",
            CardKind::Data => "data (mónada)",
        },
    );
    row(&mut s, "payload", &fmt_payload(&card.payload));
    row(&mut s, "supervision", &fmt_supervision(&card.supervision));
    row(
        &mut s,
        "lifecycle",
        &format!("{:?}", card.lifecycle).to_lowercase(),
    );
    row(
        &mut s,
        "priority",
        &format!("{:?}", card.priority).to_lowercase(),
    );

    if !card.provides.is_empty() {
        row(&mut s, "provides", &fmt_caps(&card.provides));
    }
    if !card.requires.is_empty() {
        row(&mut s, "requires", &fmt_caps(&card.requires));
    }

    let perms = &card.permissions;
    let mut pol = Vec::new();
    pol.push(format!("net={:?}", perms.networking).to_lowercase());
    pol.push(format!("fs={:?}", perms.filesystem).to_lowercase());
    if perms.processes {
        pol.push("processes".into());
    }
    row(&mut s, "permissions", &pol.join("  "));

    if let Some(socket) = &card.service_socket {
        row(&mut s, "socket", &socket.display().to_string());
    }

    if !card.references.is_empty() {
        let refs: Vec<String> = card
            .references
            .iter()
            .map(|r| {
                let target = if r.target_label.is_empty() {
                    r.target_id.to_string()
                } else {
                    r.target_label.clone()
                };
                format!("{} → {target}", format!("{:?}", r.kind).to_lowercase())
            })
            .collect();
        row(&mut s, "references", &refs.join(", "));
    }

    if !card.genesis.is_empty() {
        row(
            &mut s,
            "genesis",
            &format!("{} hija(s)", card.genesis.len()),
        );
    }

    if let Some(data) = &card.data {
        if !data.summary.is_empty() {
            row(&mut s, "summary", &data.summary);
        }
        if !data.keywords.is_empty() {
            row(&mut s, "keywords", &data.keywords.join(", "));
        }
        if data.member_count > 0 {
            row(&mut s, "members", &data.member_count.to_string());
        }
        if !data.presentation_hint.is_empty() {
            row(&mut s, "lens", &data.presentation_hint);
        }
    }

    s
}

pub fn fmt_payload(p: &Payload) -> String {
    match p {
        Payload::Wasm { entry, .. } => format!("wasm (entry: {entry})"),
        Payload::Native { exec, .. } => format!("native ({exec})"),
        Payload::Virtual => "virtual (nodo lógico)".into(),
        Payload::Legacy { exec, .. } => format!("legacy ({exec})"),
    }
}

pub fn fmt_supervision(sv: &Supervision) -> String {
    match sv {
        Supervision::Restart { initial, max } => {
            format!("restart ({}ms…{}ms)", initial.as_millis(), max.as_millis())
        }
        Supervision::OneShot => "oneshot".into(),
        Supervision::Delegate => "delegate".into(),
    }
}

pub fn fmt_caps<T: std::fmt::Debug>(caps: impl IntoIterator<Item = T>) -> String {
    caps.into_iter()
        .map(|c| format!("{c:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "schema_version": 1,
        "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
        "label": "nakui-ventas",
        "provides": ["Spawn", "Journal"],
        "payload": {"Wasm": {"module_sha256": [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32], "entry": "main"}},
        "supervision": "OneShot"
    }"#;

    #[test]
    fn parsea_y_resume_card() {
        let card = Card::from_json(SAMPLE).unwrap();
        let out = summarize(&card);
        assert!(out.contains("nakui-ventas"));
        assert!(out.contains("wasm (entry: main)"));
        assert!(out.contains("oneshot"));
        assert!(out.contains("Spawn"));
    }

    #[test]
    fn card_invalida_es_error() {
        // Bytes que no son una Card válida.
        let tmp = std::env::temp_dir().join("nahual-card-viewer-test-invalid.json");
        std::fs::write(&tmp, b"{not a card}").unwrap();
        assert!(matches!(load_card(&tmp), CardPreview::Error(_)));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn omite_campos_vacios() {
        let card = Card::from_json(SAMPLE).unwrap();
        let out = summarize(&card);
        // Sin `requires` declarado, no debe aparecer la fila.
        assert!(!out.contains("requires"));
    }
}
