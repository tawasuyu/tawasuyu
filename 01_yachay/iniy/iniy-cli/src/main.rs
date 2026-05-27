//! iniy — CLI del laboratorio semántico de creencias.
//!
//! Subcomandos (MVP heurístico):
//!   ingest <ruta> [--fuente N [--kind K]]
//!                           — carga un doc y lo chunkea; opcional: atribuir a fuente
//!   list                    — lista documentos persistidos
//!   show <doc-id>           — imprime los chunks de un documento
//!   extract <doc-id>        — extrae aserciones de los chunks (heurístico)
//!   nli <doc-id>            — computa la matriz NLI sobre los pares (léxico)
//!   contradictions <doc-id> — top-N pares más contradictorios
//!   fuentes                 — lista fuentes con conteo de docs/aserciones
//!   attribute <doc-id> N    — re-atribuye un doc existente a la fuente con nombre N
//!   testimonio "<query>"    — qué dice el corpus sobre <query>: apoyos / contradicciones
//!                             con la opinión autoral de cada aserción y su fuente

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iniy_core::{Asercion, AsercionId, DocId, Implicacion, Opinion};
use iniy_extract::{Extractor, ExtractorHeuristico};
use iniy_graph::GrafoCreencias;
use iniy_nli::{relacion_lexica, MotorNli, MotorNliLexico};
use iniy_nli_llm::MotorNliLlm;
use iniy_store::AsercionAtribuida;
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

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum BackendNli {
    Lexico,
    Llm,
}

#[derive(Subcommand)]
enum Cmd {
    /// Ingesta un archivo de texto y lo chunkea.
    Ingest {
        ruta: PathBuf,
        #[arg(long)]
        titulo: Option<String>,
        /// Nombre de la fuente (autor, escuela, tradición, observación…).
        #[arg(long)]
        fuente: Option<String>,
        /// Tipo de fuente: "autor", "escuela", "tradición", "observación", etc.
        #[arg(long)]
        kind: Option<String>,
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
    /// Computa la matriz NLI sobre los pares de aserciones. Si `doc_id` se
    /// omite, recorre TODO el corpus (cross-doc) — necesario para que el
    /// grafo conecte aserciones que viven en documentos / fuentes distintas.
    Nli {
        doc_id: Option<String>,
        #[arg(long, default_value_t = 0.30)]
        umbral: f32,
        /// Backend NLI: `lexico` (default, instantáneo, sin red) o `llm`
        /// (vía `pluma_llm::from_env`).
        #[arg(long, default_value = "lexico")]
        backend: BackendNli,
    },
    /// Imprime las N aserciones más contradictorias entre sí.
    Contradictions {
        doc_id: String,
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    /// Lista las fuentes con conteo de docs y aserciones.
    Fuentes,
    /// Re-atribuye un documento ya persistido a una fuente (creada si no existe).
    Attribute {
        doc_id: String,
        fuente: String,
        #[arg(long)]
        kind: Option<String>,
    },
    /// "¿Qué dice el corpus sobre X?" — agrupa aserciones que apoyan o contradicen
    /// el query, con la opinión autoral y la fuente de cada una.
    Testimonio {
        query: String,
        #[arg(long, default_value_t = 0.20)]
        umbral: f32,
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    /// Propaga la opinión autoral de una aserción semilla por el grafo NLI,
    /// con descuento de Jøsang por el score de cada arista.
    Propagar {
        asercion_id: String,
    },
    /// Fusiona las opiniones del corpus sobre la query: incorpora APOYAN
    /// (descontados por entailment) y CONTRADICEN (invertidos + descontados).
    /// Devuelve la opinión consensuada + lista de fuentes que contribuyen.
    Consenso {
        query: String,
        #[arg(long, default_value_t = 0.20)]
        umbral: f32,
    },
}

fn parse_doc_id(s: &str) -> Result<DocId> {
    let ulid = Ulid::from_str(s).with_context(|| format!("doc_id inválido (esperado Ulid): {s}"))?;
    Ok(DocId(ulid))
}

fn parse_asercion_id(s: &str) -> Result<AsercionId> {
    let ulid = Ulid::from_str(s).with_context(|| format!("asercion_id inválido (esperado Ulid): {s}"))?;
    Ok(AsercionId(ulid))
}

fn truncar(s: &str, n: usize) -> String {
    if n == 0 || s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
}

fn etiqueta_fuente(att: &AsercionAtribuida) -> String {
    match &att.fuente {
        Some(f) => match &f.kind {
            Some(k) => format!("{} [{}] / {}", f.nombre, k, att.doc_titulo),
            None => format!("{} / {}", f.nombre, att.doc_titulo),
        },
        None => format!("(sin fuente) / {}", att.doc_titulo),
    }
}

fn fila_opinion(op: &Opinion) -> String {
    format!("b={:.2} d={:.2} u={:.2}", op.creencia, op.descreencia, op.incertidumbre)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    let mut store = iniy_store::Store::abrir(&cli.db)?;

    match cli.cmd {
        Cmd::Ingest { ruta, titulo, fuente, kind } => {
            let titulo = titulo.unwrap_or_else(|| ruta.file_stem().and_then(|s| s.to_str()).unwrap_or("sin-titulo").to_string());
            let doc = iniy_ingest::ingest_txt(&ruta, titulo)?;
            let fuente_id = match fuente.as_deref() {
                Some(n) => Some(store.obtener_o_crear_fuente(n, kind.as_deref())?),
                None => None,
            };
            store.persistir_documento(&doc, fuente_id)?;
            println!("doc-id: {}", doc.id.0);
            println!("chunks: {}", doc.chunks.len());
            if let Some(n) = fuente.as_deref() {
                println!("fuente: {} {}", n, kind.as_deref().map(|k| format!("[{k}]")).unwrap_or_default());
            }
            println!("persistido en: {}", cli.db.display());
        }
        Cmd::List => {
            let docs = store.listar_documentos()?;
            if docs.is_empty() {
                println!("(sin documentos — usa `iniy ingest <ruta>` para empezar)");
            } else {
                for d in docs {
                    let fuente = d.fuente.as_ref().map(|f| f.nombre.as_str()).unwrap_or("(sin fuente)");
                    println!("{}  {:>4} chunks  [{}]  {}", d.id.0, d.n_chunks, fuente, d.titulo);
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
            let mut total: Vec<Asercion> = Vec::new();
            for c in &chunks {
                let mut a = extractor.extraer(c).await?;
                total.append(&mut a);
            }
            store.persistir_aserciones(&total)?;
            println!("aserciones extraídas: {}  (de {} chunks)", total.len(), chunks.len());
            for a in total.iter().take(8) {
                println!("  · {}  {}", fila_opinion(&a.opinion_autoral), truncar(&a.texto, 90));
            }
            if total.len() > 8 {
                println!("  … (+{} más, persistidas)", total.len() - 8);
            }
        }
        Cmd::Nli { doc_id, umbral, backend } => {
            let (aserciones, alcance) = match doc_id {
                Some(d) => {
                    let id = parse_doc_id(&d)?;
                    (store.cargar_aserciones(id)?, format!("doc {}", id.0))
                }
                None => {
                    let atribuidas = store.cargar_aserciones_atribuidas_todas()?;
                    (atribuidas.into_iter().map(|a| a.asercion).collect(), "todo el corpus (cross-doc)".to_string())
                }
            };
            if aserciones.len() < 2 {
                anyhow::bail!("se necesitan ≥2 aserciones (corre `iniy extract` primero)");
            }
            println!("nli sobre {alcance}: {} aserciones", aserciones.len());
            let motor: Box<dyn MotorNli> = match backend {
                BackendNli::Lexico => Box::new(MotorNliLexico { umbral_overlap: umbral }),
                BackendNli::Llm => {
                    let chat = pluma_llm::from_env()
                        .map_err(|e| anyhow::anyhow!("no pude inicializar LLM: {e}"))?;
                    println!("backend LLM: {}", chat.model_id());
                    Box::new(MotorNliLlm::nuevo(chat))
                }
            };
            let total = aserciones.len() * (aserciones.len() - 1) / 2;
            let mut imps = Vec::new();
            let mut no_neutrales = 0usize;
            let mut hechos = 0usize;
            for i in 0..aserciones.len() {
                for j in (i + 1)..aserciones.len() {
                    let rel = motor.evaluar(&aserciones[i], &aserciones[j]).await?;
                    hechos += 1;
                    if matches!(backend, BackendNli::Llm) && hechos % 10 == 0 {
                        eprintln!("  ... {hechos}/{total}");
                    }
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
            println!("persistido.");
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
        Cmd::Fuentes => {
            let lista = store.listar_fuentes()?;
            if lista.is_empty() {
                println!("(sin fuentes — usa `iniy ingest --fuente <nombre>` o `iniy attribute`)");
            } else {
                for r in lista {
                    let kind = r.fuente.kind.as_deref().map(|k| format!(" [{k}]")).unwrap_or_default();
                    println!("{}  {:>3} docs  {:>4} aserciones  {}{}",
                        r.fuente.id.0, r.n_docs, r.n_aserciones, r.fuente.nombre, kind);
                }
            }
        }
        Cmd::Attribute { doc_id, fuente, kind } => {
            let doc_id = parse_doc_id(&doc_id)?;
            let fuente_id = store.obtener_o_crear_fuente(&fuente, kind.as_deref())?;
            store.asignar_fuente_a_doc(doc_id, Some(fuente_id))?;
            println!("doc {} ahora atribuido a «{}»", doc_id.0, fuente);
        }
        Cmd::Propagar { asercion_id } => {
            let seed_id = parse_asercion_id(&asercion_id)?;
            let todas = store.cargar_aserciones_atribuidas_todas()?;
            let seed = todas.iter().find(|a| a.asercion.id == seed_id)
                .with_context(|| format!("aserción {asercion_id} no encontrada en el corpus"))?;
            let imps = store.cargar_implicaciones_todas()?;
            let mut g = GrafoCreencias::nuevo();
            for a in &todas { g.agregar_asercion(&a.asercion); }
            for i in imps { g.agregar_implicacion(i); }
            let propagado = g.propagar(seed_id, seed.asercion.opinion_autoral);

            println!("propagación desde:");
            println!("  {}", etiqueta_fuente(seed));
            println!("  «{}»", truncar(&seed.asercion.texto, 140));
            println!("  inicial: {}", fila_opinion(&seed.asercion.opinion_autoral));
            println!();
            println!("opinión inducida sobre {} aserciones alcanzables:", propagado.len() - 1);

            let mut alcanzadas: Vec<(&AsercionAtribuida, Opinion)> = propagado.iter()
                .filter(|(id, _)| **id != seed_id)
                .filter_map(|(id, op)| todas.iter().find(|a| a.asercion.id == *id).map(|a| (a, *op)))
                .collect();
            // Las más polarizadas (lejos de neutro) primero.
            alcanzadas.sort_by(|a, b| {
                let pa = (a.1.creencia - a.1.descreencia).abs();
                let pb = (b.1.creencia - b.1.descreencia).abs();
                pb.partial_cmp(&pa).unwrap_or(std::cmp::Ordering::Equal)
            });
            for (att, op) in alcanzadas {
                println!("  · {}", fila_opinion(&op));
                println!("    {}", etiqueta_fuente(att));
                println!("    «{}»", truncar(&att.asercion.texto, 130));
            }
        }
        Cmd::Consenso { query, umbral } => {
            let todas = store.cargar_aserciones_atribuidas_todas()?;
            if todas.is_empty() {
                anyhow::bail!("corpus vacío de aserciones");
            }
            let mut contribuciones: Vec<(Opinion, &AsercionAtribuida, &'static str, f32)> = Vec::new();
            for att in todas.iter() {
                let rel = relacion_lexica(&query, &att.asercion.texto, umbral);
                if rel.entailment > 0.0 {
                    contribuciones.push((att.asercion.opinion_autoral.descontar(rel.entailment), att, "apoya", rel.entailment));
                } else if rel.contradiction > 0.0 {
                    contribuciones.push((att.asercion.opinion_autoral.invertir().descontar(rel.contradiction), att, "contradice", rel.contradiction));
                }
            }
            if contribuciones.is_empty() {
                println!("consenso sobre «{}»: (corpus en silencio — nadie habla con suficiente overlap léxico)", query);
                return Ok(());
            }
            let ops: Vec<Opinion> = contribuciones.iter().map(|c| c.0).collect();
            let fusion = Opinion::fusionar_muchas(&ops);
            println!("consenso sobre: «{}»", query);
            println!("  fuentes que hablan: {}", contribuciones.len());
            println!("  opinión fusionada: {}", fila_opinion(&fusion));
            println!("  probabilidad esperada: {:.2}", fusion.probabilidad_esperada());
            println!();
            println!("contribuciones:");
            for (op, att, signo, score) in contribuciones {
                println!("  · {signo} (score={:.2}) → {}", score, fila_opinion(&op));
                println!("    {}", etiqueta_fuente(att));
                println!("    «{}»", truncar(&att.asercion.texto, 130));
            }
        }
        Cmd::Testimonio { query, umbral, top } => {
            let todas = store.cargar_aserciones_atribuidas_todas()?;
            if todas.is_empty() {
                anyhow::bail!("corpus vacío de aserciones (corre `iniy extract` sobre algún doc primero)");
            }

            let mut apoyan: Vec<(f32, AsercionAtribuida)> = Vec::new();
            let mut contradicen: Vec<(f32, AsercionAtribuida)> = Vec::new();
            for att in todas.iter() {
                let rel = relacion_lexica(&query, &att.asercion.texto, umbral);
                if rel.entailment > 0.0 {
                    apoyan.push((rel.entailment, att.clone()));
                } else if rel.contradiction > 0.0 {
                    contradicen.push((rel.contradiction, att.clone()));
                }
            }
            // Orden: primero por score NLI descendente, luego por creencia descendente
            // dentro del mismo score (los más confiados arriba).
            let cmp = |a: &(f32, AsercionAtribuida), b: &(f32, AsercionAtribuida)| {
                b.0.partial_cmp(&a.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.1.asercion.opinion_autoral.creencia.partial_cmp(&a.1.asercion.opinion_autoral.creencia).unwrap_or(std::cmp::Ordering::Equal))
            };
            apoyan.sort_by(cmp);
            contradicen.sort_by(cmp);

            println!("postura sobre: «{}»", query);
            println!("  motor léxico · umbral={:.2} · top={} · scanned={} aserciones",
                umbral, top, todas.len());
            println!();
            println!("APOYAN ({}):", apoyan.len());
            if apoyan.is_empty() {
                println!("  (nadie en el corpus apoya con suficiente overlap léxico)");
            } else {
                for (score, att) in apoyan.into_iter().take(top) {
                    println!("  · score={:.2}  {}", score, fila_opinion(&att.asercion.opinion_autoral));
                    println!("    {}", etiqueta_fuente(&att));
                    println!("    «{}»", truncar(&att.asercion.texto, 140));
                }
            }
            println!();
            println!("CONTRADICEN ({}):", contradicen.len());
            if contradicen.is_empty() {
                println!("  (nadie en el corpus contradice con suficiente overlap léxico)");
            } else {
                for (score, att) in contradicen.into_iter().take(top) {
                    println!("  · score={:.2}  {}", score, fila_opinion(&att.asercion.opinion_autoral));
                    println!("    {}", etiqueta_fuente(&att));
                    println!("    «{}»", truncar(&att.asercion.texto, 140));
                }
            }
        }
    }
    Ok(())
}
