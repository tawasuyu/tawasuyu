//! Acción LLM del shell nahual: "Preguntar a la IA sobre la selección". Arma
//! un prompt con el contexto de lo seleccionado (un archivo + snippet, una
//! carpeta + listado, o la marca múltiple) y lo manda a `pluma-llm` en un
//! worker (runtime tokio efímero). Sin credenciales, la fachada cae a Mock —
//! la acción no falla, sólo responde con el backend que haya.

use std::path::Path;

use llimphi_ui::Handle;

use crate::modelo::{AiState, Model, Msg};

/// Bytes de contenido de un archivo de texto que entran al prompt.
const AI_SNIPPET_BYTES: usize = 4096;
/// Tope de tokens de la respuesta.
const AI_MAX_TOKENS: u32 = 500;

/// ¿La extensión sugiere texto legible para incluir su contenido en el prompt?
fn es_texto(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some(
            "rs" | "toml" | "md" | "txt" | "json" | "yaml" | "yml" | "html" | "css" | "js"
                | "ts" | "py" | "c" | "h" | "cpp" | "go" | "sh" | "lua" | "rb" | "sql" | "xml"
                | "ini" | "cfg" | "conf" | "csv" | "rhai"
        )
    )
}

/// Construye `(título, prompt)` para la IA según lo seleccionado en el panel
/// enfocado. `None` si no hay nada accionable.
pub(crate) fn contexto_para_ia(m: &Model) -> Option<(String, String)> {
    let nav = m.cur();
    let pane = m.cur_pane();
    // Caso 1: marca múltiple → resumir el conjunto por sus nombres.
    if !pane.marked.is_empty() {
        let nombres: Vec<String> = nav
            .children()
            .iter()
            .filter(|n| pane.marked.contains(&n.id))
            .map(|n| n.name.clone())
            .collect();
        let titulo = format!("IA · {} elementos seleccionados", nombres.len());
        let prompt = format!(
            "Estos son nombres de archivos seleccionados en un explorador de \
             archivos:\n{}\n\nEn español y en un párrafo breve: ¿qué tienen en \
             común y para qué parecen servir en conjunto?",
            nombres.join("\n")
        );
        return Some((titulo, prompt));
    }
    // Caso 2: un nodo bajo el cursor.
    let node = nav.selected_node()?;
    if node.is_container {
        // Carpeta → describir por su listado.
        let id_path = Path::new(&node.id);
        let listado: Vec<String> = std::fs::read_dir(id_path)
            .into_iter()
            .flatten()
            .flatten()
            .take(80)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let titulo = format!("IA · carpeta {}", node.name);
        let prompt = format!(
            "Contenido de la carpeta «{}»:\n{}\n\nEn español y en un párrafo \
             breve: ¿qué tipo de proyecto o conjunto de datos parece ser y qué \
             contiene?",
            node.name,
            listado.join("\n")
        );
        return Some((titulo, prompt));
    }
    // Archivo: nombre + (si es texto) un snippet de su contenido.
    let id_path = Path::new(&node.id);
    let titulo = format!("IA · {}", node.name);
    let mut prompt = format!("Archivo: «{}».", node.name);
    if id_path.is_file() && es_texto(id_path) {
        if let Some(sample) = leer_snippet(id_path, AI_SNIPPET_BYTES) {
            prompt.push_str(&format!("\n\nContenido (recortado):\n{sample}"));
        }
    }
    prompt.push_str(
        "\n\nEn español y en un párrafo breve: ¿qué es este archivo y qué hace o \
         para qué sirve?",
    );
    Some((titulo, prompt))
}

/// Lee hasta `max` bytes del inicio de `path` como texto (lossy).
fn leer_snippet(path: &Path, max: usize) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Manda `prompt` a `pluma-llm` (autodetecta backend; Mock sin credenciales) y
/// devuelve el texto de la respuesta. Bloquea en un runtime tokio efímero —
/// pensado para correr dentro de un worker de `Handle::spawn`, no en la UI.
pub(crate) fn ask_llm(prompt: String) -> Result<String, String> {
    use pluma_llm::pluma_llm_core::{ChatClient, ChatRequest};
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    rt.block_on(async move {
        let client: std::sync::Arc<dyn ChatClient> =
            pluma_llm::from_env().map_err(|e| format!("backend LLM: {e}"))?;
        let req = ChatRequest::una_vuelta(prompt, AI_MAX_TOKENS);
        client
            .complete(&req)
            .await
            .map(|r| r.content)
            .map_err(|e| format!("LLM: {e}"))
    })
}

/// Dispatcher de los `Msg` de IA. Lanza el worker en `AiAsk`.
pub(crate) fn apply_ai(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
    let mut m = model;
    match msg {
        Msg::AiAsk => {
            m.context_menu = None;
            if let Some((titulo, prompt)) = contexto_para_ia(&m) {
                m.ai = Some(AiState { titulo, respuesta: None, pendiente: true });
                handle.spawn(move || Msg::AiResult(ask_llm(prompt)));
            }
        }
        Msg::AiResult(res) => {
            if let Some(ai) = m.ai.as_mut() {
                ai.pendiente = false;
                ai.respuesta = Some(match res {
                    Ok(texto) => texto,
                    Err(e) => format!("(error) {e}"),
                });
            }
        }
        Msg::AiClose => {
            m.ai = None;
        }
        _ => {}
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end del plumbing con el backend por defecto (Mock sin
    /// credenciales): un prompt produce una respuesta no vacía sin colgar.
    #[test]
    fn ask_llm_responde_con_mock() {
        // Forzamos Mock para que el test sea determinista aunque el entorno
        // tenga alguna API key suelta.
        std::env::set_var("PLUMA_LLM_BACKEND", "mock");
        let r = ask_llm("¿Qué es un archivo de texto?".to_string());
        assert!(r.is_ok(), "el plumbing LLM no debería fallar con Mock: {r:?}");
        assert!(!r.unwrap().trim().is_empty(), "Mock debería responder algo");
    }
}
