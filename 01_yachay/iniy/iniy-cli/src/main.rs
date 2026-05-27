//! iniy — CLI del laboratorio semántico de creencias.
//!
//! Subcomandos planificados (MVP):
//!   ingest <ruta>           — carga un documento y lo chunkea
//!   list                    — lista documentos persistidos
//!   show <doc-id>           — imprime los chunks de un documento
//!   extract <doc-id>        — extrae aserciones de los chunks
//!   nli <doc-id>            — computa la matriz NLI sobre los pares
//!   contradictions <doc-id> — top-N pares más contradictorios

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iniy_core::DocId;
use std::path::PathBuf;
use std::str::FromStr;
use ulid::Ulid;

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
    /// Lista los documentos persistidos en la DB.
    List,
    /// Imprime los chunks de un documento.
    Show {
        doc_id: String,
        /// Trunca cada chunk a N caracteres (0 = sin truncar).
        #[arg(long, default_value_t = 120)]
        truncar: usize,
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

fn parse_doc_id(s: &str) -> Result<DocId> {
    let ulid = Ulid::from_str(s).with_context(|| format!("doc_id inválido (esperado Ulid): {s}"))?;
    Ok(DocId(ulid))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    let mut store = iniy_store::Store::abrir(&cli.db)?;

    match cli.cmd {
        Cmd::Ingest { ruta, titulo } => {
            let titulo = titulo.unwrap_or_else(|| ruta.file_stem().and_then(|s| s.to_str()).unwrap_or("sin-titulo").to_string());
            let doc = iniy_ingest::ingest_txt(&ruta, titulo)?;
            store.persistir_documento(&doc)?;
            println!("doc-id: {}", doc.id.0);
            println!("chunks: {}", doc.chunks.len());
            println!("persistido en: {}", cli.db.display());
        }
        Cmd::List => {
            let docs = store.listar_documentos()?;
            if docs.is_empty() {
                println!("(sin documentos — usa `iniy ingest <ruta>` para empezar)");
            } else {
                for d in docs {
                    println!("{}  {:>4} chunks  {}", d.id.0, d.n_chunks, d.titulo);
                }
            }
        }
        Cmd::Show { doc_id, truncar } => {
            let doc_id = parse_doc_id(&doc_id)?;
            let chunks = store.cargar_chunks(doc_id)?;
            if chunks.is_empty() {
                println!("(sin chunks — ¿doc_id correcto?)");
            } else {
                for c in chunks {
                    let t = if truncar > 0 && c.texto.chars().count() > truncar {
                        let mut s: String = c.texto.chars().take(truncar).collect();
                        s.push('…');
                        s
                    } else {
                        c.texto
                    };
                    println!("[{:>3}] {}", c.orden, t);
                }
            }
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
