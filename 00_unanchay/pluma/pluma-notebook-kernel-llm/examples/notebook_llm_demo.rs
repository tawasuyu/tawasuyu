//! Demo CLI: notebook con cuatro celdas LLM ejecutado con
//! `pluma_notebook_exec::run_all`.
//!
//! ```bash
//! # Sin keys: mock pre-poblado.
//! cargo run -p pluma-notebook-kernel-llm --example notebook_llm_demo --release
//!
//! # Con Gemini:
//! GEMINI_API_KEY=... PLUMA_LLM_BACKEND=gemini \
//!   cargo run -p pluma-notebook-kernel-llm --example notebook_llm_demo --release
//!
//! # Ollama local:
//! PLUMA_LLM_BACKEND=ollama PLUMA_LLM_MODEL=llama3.1 \
//!   cargo run -p pluma-notebook-kernel-llm --example notebook_llm_demo --release
//! ```

use std::sync::Arc;

use pluma_llm::from_env as llm_from_env;
use pluma_llm_core::ChatClient;
use pluma_notebook_core::cell::{CellKind, OutputPayload};
use pluma_notebook_core::notebook::Notebook;
use pluma_notebook_exec::run_all;
use pluma_notebook_kernel_llm::LlmKernel;

const TEXTO_FUENTE: &str = "El cóndor cruzó el cielo del valle al amanecer. \
Las llamas pastaban entre los pastizales del altiplano. Una mujer joven \
tejía un telar bajo el alero.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let chat = construir_chat();
    eprintln!("notebook_llm_demo :: LLM = {}", chat.model_id());

    let kernel = LlmKernel::from_arc(chat);

    let mut nb = Notebook::new();
    let c_md = nb.push(CellKind::Markdown, format!("# Texto fuente\n\n{TEXTO_FUENTE}"));
    let c_qu = nb.push(
        CellKind::Code { language: "llm-traducir-qu".into() },
        TEXTO_FUENTE,
    );
    let c_formal = nb.push(
        CellKind::Code { language: "llm-tono-formal".into() },
        TEXTO_FUENTE,
    );
    let c_resumen = nb.push(
        CellKind::Code { language: "llm-resumir-20".into() },
        TEXTO_FUENTE,
    );
    // Las tres celdas LLM dependen conceptualmente del markdown fuente —
    // anotamos la dependencia para que el DAG lo refleje aunque no
    // afecte la ejecución (las celdas LLM tienen su source propio).
    nb.add_dependency(c_qu, c_md);
    nb.add_dependency(c_formal, c_md);
    nb.add_dependency(c_resumen, c_md);

    let reporte = run_all(&mut nb, &kernel)
        .await
        .expect("notebook tiene orden topológico");

    println!("\n=== ejecución ===");
    println!(
        "ejecutadas: {}   fallidas: {}   skipped: {}",
        reporte.executed.len(),
        reporte.failed.len(),
        reporte.skipped.len()
    );
    for &id in &reporte.failed {
        eprintln!("FAILED celda {id}");
    }

    println!("\n=== outputs ===");
    for cell in nb.cells() {
        let lang = match &cell.kind {
            CellKind::Markdown => "markdown",
            CellKind::Code { language } => language.as_str(),
            CellKind::Embed { module } => module.as_str(),
        };
        match cell.last_output.as_ref().map(|o| &o.payload) {
            Some(OutputPayload::Text(t)) => {
                println!("\n[{}/{}]\n{}", cell.id, lang, t);
            }
            Some(otro) => {
                println!("\n[{}/{}] payload no-textual: {:?}", cell.id, lang, otro);
            }
            None => {
                println!("\n[{}/{}] sin output", cell.id, lang);
            }
        }
    }

    Ok(())
}

fn construir_chat() -> Arc<dyn ChatClient> {
    let usa_mock = std::env::var("ANTHROPIC_API_KEY").is_err()
        && std::env::var("GEMINI_API_KEY").is_err()
        && std::env::var("GOOGLE_API_KEY").is_err()
        && std::env::var("DEEPSEEK_API_KEY").is_err()
        && std::env::var("PLUMA_LLM_BACKEND")
            .map(|s| s.to_lowercase() != "ollama")
            .unwrap_or(true);
    if usa_mock {
        // Reglas por system: cada acción del LlmKernel pone un system
        // distinto, así el mock distingue traducir / tono / resumir
        // aun cuando el `user` (el texto fuente) es el mismo.
        let mock = pluma_llm_mock::MockChatClient::default()
            .con_model_id("mock-nb")
            .con_respuesta_si_system(
                "traductor profesional al qu",
                "Kuntur wayqu hanaqpachatakta pacha paqarinpi pasarqa. \
                 Llamaqakuna qulla suyup q'achupinpi mikhusharqaku. Sipas \
                 warmiq away wasiq hawanpi awayta ruwasharqa.",
            )
            .con_respuesta_si_system(
                "tono formal",
                "El cóndor surcó con majestuosidad el cielo del valle al \
                 alba. Las llamas se alimentaban con sosiego en los \
                 pastizales del altiplano. Una joven mujer trabajaba el \
                 telar al amparo del alero.",
            )
            .con_respuesta_si_system(
                "Resumes",
                "Amanecer andino: cóndor, llamas, tejedora.",
            );
        return Arc::new(mock);
    }
    llm_from_env().expect("from_env")
}
