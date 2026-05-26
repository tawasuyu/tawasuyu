//! Smoke test contra Gemini real. UNA request, prompt corto,
//! max_tokens muy bajo — para validar que el adapter habla con
//! `api.anthropic.com`... perdón, con `generativelanguage.googleapis.com`
//! sin gastar tokens. Lee `GEMINI_API_KEY` del env.
//!
//! Corré con:
//!   GEMINI_API_KEY=... cargo run -p pluma-llm-gemini --example smoke --release

use pluma_llm_core::{ChatClient, ChatRequest};
use pluma_llm_gemini::GeminiClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = GeminiClient::from_env()?;
    eprintln!("smoke :: usando modelo {}", cli.model_id());

    // Mínimo absoluto: una palabra a traducir, system corto.
    let req = ChatRequest::una_vuelta("Translate the single word \"hello\" to Quechua. Respond with just the word.", 30)
        .con_sistema("You output a single word with no punctuation.")
        .con_temperatura(0.1);

    let resp = cli.complete(&req).await?;
    println!("respuesta: {}", resp.content.trim());
    if let Some(u) = resp.usage {
        println!(
            "tokens: input={} output={} cache_read={} cache_creation={}",
            u.input_tokens,
            u.output_tokens,
            u.cache_read_input_tokens,
            u.cache_creation_input_tokens
        );
    }
    if let Some(s) = resp.stop_reason {
        println!("stop_reason: {}", s.0);
    }
    Ok(())
}
