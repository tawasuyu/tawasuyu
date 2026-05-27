//! iniy — CLI del laboratorio semántico de creencias.
//!
//! Subcomandos (MVP heurístico):
//!   ingest <ruta>           — carga un documento y lo chunkea
//!   list                    — lista documentos persistidos
//!   show <doc-id>           — imprime los chunks de un documento
//!   extract <doc-id>        — extrae aserciones de los chunks (heurístico)
//!   nli <doc-id>            — computa la matriz NLI sobre los pares (léxico)
//!   contradictions <doc-id> — top-N pares más contradictorios

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iniy_core::{DocId, Implicacion};
use iniy_extract::{Extractor, ExtractorHeuristico};
use iniy_graph::GrafoCreencias;
use iniy_nli::{MotorNli, MotorNliLexico};
use std::collections::HashMap;
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
        #[arg(long, default_value_t = 120)]
        truncar: usize,
    },
    /// Extrae aserciones atómicas de los chunks de un documento.
    Extract { doc_id: String },
    /// Computa la matriz NLI sobre los pares de aserciones del documento.
    Nli {
        doc_id: String,
        /// Umbral de overlap léxico [0,1].
        #[arg(long, default_value_t = 0.30)]
        umbral: f32,
    },
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

fn truncar(s: &str, n: usize) -> String {
    if n == 0 || s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
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
        Cmd::Show { doc_id, truncar: trunc } => {
            let doc_id = parse_doc_id(&doc_id)?;
            let chunks = store.cargar_chunks(doc_id)?;
            if chunks.is_empty() {
                println!("(sin chunks — ¿doc_id correcto?)");
            } else {
                for c in chunks {
                    println!("[{:>3}] {}", c.orden, truncar(&c.texto, trunc));
                }
            }
        }
        Cmd::Extract { doc_id } => {
            let doc_id = parse_doc_id(&doc_id)?;
            let chunks = store.cargar_chunks(doc_id)?;
            if chunks.is_empty() {
                anyhow::bail!("doc no tiene chunks (¿doc_id correcto, ya hiciste ingest?)");
            }
            let extractor = ExtractorHeuristico::default();
            let mut total = Vec::new();
            for c in &chunks {
                let mut a = extractor.extraer(c).await?;
                total.append(&mut a);
            }
            store.persistir_aserciones(&total)?;
            println!("aserciones extraídas: {}  (de {} chunks)", total.len(), chunks.len());
            for a in total.iter().take(8) {
                println!("  · b={:.2} d={:.2} u={:.2}  {}",
                    a.opinion_autoral.creencia, a.opinion_autoral.descreencia, a.opinion_autoral.incertidumbre,
                    truncar(&a.texto, 90));
            }
            if total.len() > 8 {
                println!("  … (+{} más, persistidas)", total.len() - 8);
            }
        }
        Cmd::Nli { doc_id, umbral } => {
            let doc_id = parse_doc_id(&doc_id)?;
            let aserciones = store.cargar_aserciones(doc_id)?;
            if aserciones.len() < 2 {
                anyhow::bail!("se necesitan ≥2 aserciones (corre `iniy extract` primero)");
            }
            let motor = MotorNliLexico { umbral_overlap: umbral };
            let mut imps = Vec::new();
            let mut no_neutrales = 0usize;
            for i in 0..aserciones.len() {
                for j in (i + 1)..aserciones.len() {
                    let rel = motor.evaluar(&aserciones[i], &aserciones[j]).await?;
                    if rel.contradiction > 0.0 || rel.entailment > 0.0 {
                        no_neutrales += 1;
                    }
                    imps.push(Implicacion {
                        premisa: aserciones[i].id,
                        hipotesis: aserciones[j].id,
                        relacion: rel,
                    });
                }
            }
            store.persistir_implicaciones(&imps)?;
            println!("pares evaluados: {}", imps.len());
            println!("relaciones no triviales: {}  (entailment o contradiction > 0)", no_neutrales);
            println!("persistido. corre `iniy contradictions {doc_id}` para ver el top.",
                doc_id = doc_id.0);
        }
        Cmd::Contradictions { doc_id, top } => {
            let doc_id = parse_doc_id(&doc_id)?;
            let aserciones = store.cargar_aserciones(doc_id)?;
            if aserciones.is_empty() {
                anyhow::bail!("doc sin aserciones (corre `iniy extract` y luego `iniy nli`)");
            }
            let imps = store.cargar_implicaciones_del_doc(doc_id)?;
            if imps.is_empty() {
                anyhow::bail!("doc sin implicaciones (corre `iniy nli` primero)");
            }
            let textos: HashMap<_, _> = aserciones.iter().map(|a| (a.id, a.texto.clone())).collect();
            let mut grafo = GrafoCreencias::nuevo();
            for a in &aserciones {
                grafo.agregar_asercion(a);
            }
            for i in imps {
                grafo.agregar_implicacion(i);
            }
            let topn = grafo.top_contradicciones(top);
            if topn.is_empty() {
                println!("(sin contradicciones detectadas — el corpus parece coherente bajo el motor léxico)");
            } else {
                println!("top {} contradicciones (de {} aserciones):", topn.len(), grafo.cantidad_aserciones());
                for (k, imp) in topn.iter().enumerate() {
                    let p = textos.get(&imp.premisa).cloned().unwrap_or_default();
                    let h = textos.get(&imp.hipotesis).cloned().unwrap_or_default();
                    println!("\n#{}  contradiction={:.2}", k + 1, imp.relacion.contradiction);
                    println!("  A: {}", truncar(&p, 140));
                    println!("  B: {}", truncar(&h, 140));
                }
            }
        }
    }
    Ok(())
}
