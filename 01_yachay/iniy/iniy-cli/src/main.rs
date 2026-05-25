//! iniy — CLI del laboratorio semántico de creencias.
//!
//! Subcomandos planificados (MVP):
//!   ingest <ruta>           — carga un documento y lo chunkea
//!   extract <doc-id>        — extrae aserciones de los chunks
//!   nli <doc-id>            — computa la matriz NLI sobre los pares
//!   contradictions <doc-id> — top-N pares más contradictorios

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "iniy", about = "Laboratorio semántico de creencias")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// Ruta al archivo SQLite (default: ./iniy.db)
    #[arg(long, default_value = "iniy.db", global = true)]
    db: PathBuf,
}

#[derive(Subcommand)]
enum Cmd {
    /// Ingesta un archivo de texto y lo chunkea.
    Ingest {
        ruta: PathBuf,
        #[arg(long)]
        titulo: Option<String>,
    },
    /// Extrae aserciones atómicas de los chunks de un documento.
    Extract { doc_id: String },
    /// Computa la matriz NLI sobre los pares de aserciones.
    Nli { doc_id: String },
    /// Imprime las N aserciones más contradictorias entre sí.
    Contradictions {
        doc_id: String,
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    let _store = iniy_store::Store::abrir(&cli.db)?;

    match cli.cmd {
        Cmd::Ingest { ruta, titulo } => {
            let titulo = titulo.unwrap_or_else(|| ruta.file_stem().and_then(|s| s.to_str()).unwrap_or("sin-titulo").to_string());
            let doc = iniy_ingest::ingest_txt(&ruta, titulo)?;
            println!("doc-id: {}", doc.id.0);
            println!("chunks: {}", doc.chunks.len());
            tracing::warn!("persistencia de chunks aún no implementada — se ejecutó solo el parse");
        }
        Cmd::Extract { doc_id } => {
            tracing::warn!("extract aún no implementado");
            println!("(extract) doc_id={doc_id}");
        }
        Cmd::Nli { doc_id } => {
            tracing::warn!("nli aún no implementado");
            println!("(nli) doc_id={doc_id}");
        }
        Cmd::Contradictions { doc_id, top } => {
            tracing::warn!("contradictions aún no implementado");
            println!("(contradictions) doc_id={doc_id} top={top}");
        }
    }
    Ok(())
}
