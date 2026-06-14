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
use iniy_core::{Asercion, AsercionId, DocId, FuenteId, Implicacion, Opinion};
use iniy_extract::{Extractor, ExtractorHeuristico};
use iniy_graph::GrafoCreencias;
use iniy_nli::{relacion_lexica, MotorNli, MotorNliLexico};
use iniy_nli_llm::MotorNliLlm;
use iniy_store::AsercionAtribuida;
use rimay_verbo_core::{EmbeddingVector, Provider};
use std::sync::Arc;
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
    /// Ingesta un archivo (TXT/MD/PDF/EPUB/PNG/JPG/TIF) y lo chunkea.
    /// PDFs sin texto digital y archivos de imagen disparan OCR vía
    /// tesseract + pdftoppm.
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
        /// Idioma(s) para OCR cuando aplica, formato tesseract: "spa",
        /// "eng", "spa+eng", "lat", "qu" (quechua si está instalado), …
        #[arg(long, default_value = "spa+eng")]
        lang: String,
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
    /// Bulk-extract: recorre TODOS los documentos sin aserciones y los
    /// procesa con el extractor heurístico. Reporta progreso cada 100
    /// docs. Para corpus masivos (post-Wikipedia bulk dump).
    ExtractAll {
        /// Frecuencia de progreso (docs procesados entre reportes).
        #[arg(long, default_value_t = 100)]
        cada: usize,
        /// Detener tras N docs (default: hasta agotarlos).
        #[arg(long)]
        max: Option<usize>,
    },
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
        /// Pre-filtra pares por similitud coseno de embeddings; solo los
        /// pares con cos ≥ `--umbral-embeddings` se mandan al backend.
        /// Reduce drásticamente las llamadas LLM en corpus grandes.
        /// Default OFF (todos los pares al backend).
        #[arg(long)]
        prefiltro_embeddings: bool,
        /// Umbral de cos para pre-filtrado. Default 0.55 (multilingual-e5-small).
        #[arg(long, default_value_t = 0.55)]
        umbral_embeddings: f32,
        /// Usa ANN (HNSW vía instant-distance) para encontrar los k
        /// vecinos más cercanos por embedding y solo evaluar NLI sobre
        /// esos pares. Escala a millones de aserciones (vs O(N²) del
        /// pre-filtro lineal). Requiere --prefiltro-embeddings implícito.
        #[arg(long)]
        ann: bool,
        /// k vecinos a recuperar con ANN.
        #[arg(long, default_value_t = 20)]
        ann_k: usize,
    },
    /// Imprime las N aserciones más contradictorias entre sí. Si `doc_id` se
    /// omite, recorre TODO el corpus (cross-doc) — necesario para ver las
    /// contradicciones que cruzan fuentes distintas, que es el caso interesante
    /// de una auditoría (igual que `nli`, que ya es cross-doc por default).
    Contradictions {
        doc_id: Option<String>,
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
    /// Marca una aserción como cita de otra fuente (≠ fuente del doc).
    /// Ej.: una aserción de un doc de "Wikipedia" que dice «Aristóteles
    /// sostenía que…» se cita a Aristóteles. `--unset` deshace.
    Cite {
        asercion_id: String,
        /// Nombre de la fuente citada. Omitir con --unset para quitar la cita.
        fuente: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        unset: bool,
    },
    /// RAG con atribución: recupera del corpus las aserciones léxicamente
    /// más relevantes para la pregunta, construye un prompt con la
    /// evidencia citada, y le pide al LLM una respuesta. La respuesta
    /// debe citar cada afirmación con [N] referenciando la fuente.
    Ask {
        pregunta: String,
        #[arg(long, default_value_t = 10)]
        top: usize,
        #[arg(long, default_value_t = 0.20)]
        umbral: f32,
        /// Filtra evidencia por tag.
        #[arg(long)]
        tag: Option<String>,
        /// Máximo de tokens de la respuesta.
        #[arg(long, default_value_t = 600)]
        max_tokens: u32,
    },
    /// Exporta toda la DB a un archivo JSON (federación / backup).
    Export {
        archivo: PathBuf,
        /// Pretty-print con indentación (default: minificado).
        #[arg(long)]
        pretty: bool,
    },
    /// Importa un dump JSON producido por otra instancia.
    /// INSERT OR IGNORE: entidades con id ya presente se respetan.
    Import {
        archivo: PathBuf,
    },
    /// Exporta la DB a otro archivo SQLite (vía VACUUM INTO).
    /// Más compacto y rápido que JSON; preserva el binary layout.
    ExportSqlite {
        archivo: PathBuf,
    },
    /// Importa otra DB SQLite mergeando con la actual (ATTACH + INSERT OR IGNORE).
    ImportSqlite {
        archivo: PathBuf,
    },
    /// Overview cuantitativo del corpus: conteos por tabla, distribución
    /// NLI, top fuentes por reputación, top tags. Útil para entender
    /// "qué tengo" tras imports masivos o tras una sesión larga.
    Stats,
    /// Vista cronológica del corpus: documentos ordenados por timestamp
    /// de ingesta, ascendente. Útil para ver cómo creció la "creencia
    /// del corpus" en el tiempo.
    Timeline {
        /// Filtra a docs creados >= esta epoch Unix (segundos).
        #[arg(long)]
        desde: Option<i64>,
        /// Filtra a docs creados <= esta epoch Unix (segundos).
        #[arg(long)]
        hasta: Option<i64>,
        /// Filtra por tag.
        #[arg(long)]
        tag: Option<String>,
    },
    /// Reputación de cada fuente (persistida en la tabla `reputaciones`).
    /// Lee la tabla; si está vacía o pasaste --recalcular, primero
    /// recalcula desde el grafo NLI y persiste vía UPSERT.
    Reputacion {
        #[arg(long)]
        recalcular: bool,
    },
    /// Asocia un tag (temática libre) a un documento. El tag se crea si
    /// no existe. Los tags propagan por doc a sus aserciones, así que
    /// `testimonio --tag X` filtra a aserciones de docs etiquetados con X.
    Tag {
        doc_id: String,
        tag: String,
    },
    /// Quita un tag de un documento.
    Untag {
        doc_id: String,
        tag: String,
    },
    /// Lista todos los tags del corpus con conteo de docs. Sin args
    /// muestra el global; con doc-id muestra solo los del doc.
    Tags {
        doc_id: Option<String>,
    },
    /// "¿Qué dice el corpus sobre X?" — agrupa aserciones que apoyan o contradicen
    /// el query, con la opinión autoral y la fuente de cada una.
    Testimonio {
        query: String,
        #[arg(long, default_value_t = 0.20)]
        umbral: f32,
        #[arg(long, default_value_t = 10)]
        top: usize,
        /// Filtra a aserciones de documentos con este tag.
        #[arg(long)]
        tag: Option<String>,
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
        /// Pesa cada contribución por la reputación de su fuente:
        /// descuento extra = (1 + score_reputación) / 2 (∈ [0,1]).
        /// Fuentes contradictorias con el corpus pesan menos.
        #[arg(long)]
        pesar_reputacion: bool,
        /// Filtra a aserciones de documentos con este tag.
        #[arg(long)]
        tag: Option<String>,
        /// Imprime la cadena completa de derivación por cada contribución:
        /// opinión autoral → invertir (si contradice) → descontar por NLI →
        /// descontar por reputación → fusión final.
        #[arg(long)]
        trace: bool,
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
    let marca_cita = if att.citada { " (citada)" } else { "" };
    match &att.fuente {
        Some(f) => match &f.kind {
            Some(k) => format!("{} [{}]{} / {}", f.nombre, k, marca_cita, att.doc_titulo),
            None => format!("{}{} / {}", f.nombre, marca_cita, att.doc_titulo),
        },
        None => format!("(sin fuente) / {}", att.doc_titulo),
    }
}

fn fila_opinion(op: &Opinion) -> String {
    format!("b={:.2} d={:.2} u={:.2}", op.creencia, op.descreencia, op.incertidumbre)
}

fn formato_bytes(n: u64) -> String {
    const UNIDADES: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut x = n as f64;
    let mut u = 0;
    while x >= 1024.0 && u < UNIDADES.len() - 1 {
        x /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{:.0} {}", x, UNIDADES[u])
    } else {
        format!("{:.2} {}", x, UNIDADES[u])
    }
}

/// Formatea Unix epoch como YYYY-MM-DD HH:MM:SS UTC, manual (sin chrono).
fn formato_fecha(unix: i64) -> String {
    // Conversión Unix → fecha calendárica (gregoriano) en UTC.
    // Días desde Unix epoch (1970-01-01) en UTC.
    let days = unix.div_euclid(86_400);
    let secs_dia = unix.rem_euclid(86_400);
    let h = secs_dia / 3600;
    let m = (secs_dia % 3600) / 60;
    let s = secs_dia % 60;
    // Algoritmo de Howard Hinnant (civil_from_days), simplificado.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, m, s)
}

/// Wrapper newtype para que `Vec<f32>` implemente `instant_distance::Point`.
/// Distancia = 1 - cosine similarity (HNSW espera distancia métrica creciente).
#[derive(Clone)]
struct HnswPoint(Vec<f32>);

impl instant_distance::Point for HnswPoint {
    fn distance(&self, other: &Self) -> f32 {
        let (a, b) = (&self.0, &other.0);
        let mut dot = 0.0_f32;
        let mut na = 0.0_f32;
        let mut nb = 0.0_f32;
        for i in 0..a.len().min(b.len()) {
            dot += a[i] * b[i];
            na += a[i] * a[i];
            nb += b[i] * b[i];
        }
        let denom = (na.sqrt() * nb.sqrt()).max(1e-9);
        (1.0 - dot / denom).max(0.0)
    }
}

/// Intenta fastembed (multilingual-e5-small, local, ~120MB ONNX descargado al
/// primer uso). Si falla (sin internet en el primer arranque, ONNX runtime
/// roto, etc.) cae a un mock determinista con warning. El mock NO da
/// similaridades semánticas reales — solo permite probar el flujo.
async fn construir_provider_embeddings() -> Result<Arc<dyn Provider>> {
    let fastembed_result = tokio::task::spawn_blocking(|| {
        rimay_verbo_fastembed::FastembedProvider::try_default()
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking falló: {e}"))?;
    match fastembed_result {
        Ok(p) => Ok(Arc::new(p)),
        Err(e) => {
            eprintln!("warning: fastembed falló ({e}); cayendo a MockProvider — el pre-filtro NO será semántico");
            Ok(Arc::new(rimay_verbo_mock::MockProvider::default()))
        }
    }
}


#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    let mut store = iniy_store::Store::abrir(&cli.db)?;

    match cli.cmd {
        Cmd::Ingest { ruta, titulo, fuente, kind, lang } => {
            let titulo = titulo.unwrap_or_else(|| ruta.file_stem().and_then(|s| s.to_str()).unwrap_or("sin-titulo").to_string());
            let doc = iniy_ingest::ingest_path_lang(&ruta, titulo, &lang)?;
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
            let mut extraidas = Vec::new();
            for c in &chunks {
                let mut a = extractor.extraer_con_atribucion(c).await?;
                extraidas.append(&mut a);
            }
            let aserciones: Vec<Asercion> = extraidas.iter().map(|e| e.asercion.clone()).collect();
            store.persistir_aserciones(&aserciones)?;
            // Atribuir cualquier cita detectada.
            let mut citas_aplicadas = 0usize;
            for e in &extraidas {
                if let Some(nombre) = &e.fuente_citada_nombre {
                    let fid = store.obtener_o_crear_fuente(nombre, None)?;
                    store.asignar_fuente_citada(e.asercion.id, Some(fid))?;
                    citas_aplicadas += 1;
                }
            }
            println!("aserciones extraídas: {}  (de {} chunks)", aserciones.len(), chunks.len());
            if citas_aplicadas > 0 {
                println!("citas inline detectadas: {citas_aplicadas} (\"Según X, …\" / \"Para X, …\")");
            }
            for (a, e) in aserciones.iter().zip(extraidas.iter()).take(8) {
                let cita = e.fuente_citada_nombre.as_deref().map(|n| format!(" → cita «{n}»")).unwrap_or_default();
                println!("  · {}{}  {}", fila_opinion(&a.opinion_autoral), cita, truncar(&a.texto, 90));
            }
            if aserciones.len() > 8 {
                println!("  … (+{} más, persistidas)", aserciones.len() - 8);
            }
        }
        Cmd::ExtractAll { cada, max } => {
            let pendientes = store.documentos_sin_aserciones()?;
            let total = pendientes.len();
            if total == 0 {
                println!("(todos los docs ya tienen aserciones extraídas)");
                return Ok(());
            }
            let limite = max.unwrap_or(total).min(total);
            println!("extract bulk: {limite} de {total} docs pendientes");
            let extractor = ExtractorHeuristico::default();
            let t0 = std::time::Instant::now();
            let mut total_asercs = 0usize;
            let mut total_citas = 0usize;
            for (k, doc_id) in pendientes.into_iter().take(limite).enumerate() {
                let chunks = store.cargar_chunks(doc_id)?;
                if chunks.is_empty() { continue; }
                let mut extraidas = Vec::new();
                for c in &chunks {
                    let mut a = extractor.extraer_con_atribucion(c).await?;
                    extraidas.append(&mut a);
                }
                let aserciones: Vec<Asercion> = extraidas.iter().map(|e| e.asercion.clone()).collect();
                if aserciones.is_empty() { continue; }
                store.persistir_aserciones(&aserciones)?;
                for e in &extraidas {
                    if let Some(nombre) = &e.fuente_citada_nombre {
                        let fid = store.obtener_o_crear_fuente(nombre, None)?;
                        store.asignar_fuente_citada(e.asercion.id, Some(fid))?;
                        total_citas += 1;
                    }
                }
                total_asercs += aserciones.len();
                let n = k + 1;
                if n.is_multiple_of(cada) || n == limite {
                    let secs = t0.elapsed().as_secs_f64();
                    let rate = n as f64 / secs.max(0.001);
                    eprintln!("  [{n}/{limite}]  Σ {total_asercs} aserciones · {total_citas} citas · {rate:.0} docs/s");
                }
            }
            let secs = t0.elapsed().as_secs_f64();
            println!();
            println!("done. {total_asercs} aserciones extraídas en {secs:.1}s ({:.0} docs/s).",
                limite as f64 / secs.max(0.001));
            if total_citas > 0 {
                println!("{total_citas} fuentes citadas inline detectadas («Según X, …» / «X afirma que …»).");
            }
        }
        Cmd::Nli { doc_id, umbral, backend, prefiltro_embeddings, umbral_embeddings, ann, ann_k } => {
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

            // ANN implica pre-cómputo de embeddings.
            let necesita_embeddings = prefiltro_embeddings || ann;
            let embeddings: Option<Vec<EmbeddingVector>> = if necesita_embeddings {
                let provider = construir_provider_embeddings().await?;
                println!("embeddings: {} ({}d)", provider.model_id().name, provider.model_id().dimension);
                let textos: Vec<String> = aserciones.iter().map(|a| a.texto.clone()).collect();
                let vecs = provider.embed_batch(&textos).await
                    .map_err(|e| anyhow::anyhow!("embed_batch falló: {e}"))?;
                Some(vecs)
            } else {
                None
            };

            // Estrategia de selección de pares.
            let pares_candidatos: Vec<(usize, usize)> = if ann {
                let vecs = embeddings.as_ref().expect("ann implica embeddings");
                println!("construyendo índice HNSW (k={ann_k}, {} aserciones)...", aserciones.len());
                let t0 = std::time::Instant::now();
                let pts: Vec<HnswPoint> = vecs.iter().map(|v| HnswPoint(v.values.clone())).collect();
                let valores: Vec<usize> = (0..aserciones.len()).collect();
                let hnsw = instant_distance::Builder::default().build(pts.clone(), valores);
                println!("  índice construido en {:.1}s", t0.elapsed().as_secs_f64());
                let mut pares = std::collections::HashSet::<(usize, usize)>::new();
                let mut search = instant_distance::Search::default();
                for (i, p) in pts.iter().enumerate() {
                    let vecinos = hnsw.search(p, &mut search).take(ann_k + 1);  // +1 porque el primero es uno mismo
                    for item in vecinos {
                        let j = *item.value;
                        if j == i { continue; }
                        let (a, b) = if i < j { (i, j) } else { (j, i) };
                        pares.insert((a, b));
                    }
                }
                let mut v: Vec<_> = pares.into_iter().collect();
                v.sort();
                println!("  pares ANN únicos: {}", v.len());
                v
            } else {
                // Modo lineal: todos los pares (i, j) con i < j.
                let mut v = Vec::with_capacity(aserciones.len() * (aserciones.len() - 1) / 2);
                for i in 0..aserciones.len() {
                    for j in (i + 1)..aserciones.len() {
                        v.push((i, j));
                    }
                }
                v
            };

            let total = pares_candidatos.len();
            let mut imps = Vec::with_capacity(total);
            let mut no_neutrales = 0usize;
            let mut hechos = 0usize;
            let mut salteados_por_prefiltro = 0usize;
            for (i, j) in pares_candidatos {
                let rel = if let Some(vs) = &embeddings {
                    let cos = vs[i].cosine(&vs[j])
                        .map_err(|e| anyhow::anyhow!("cosine falló: {e}"))?;
                    if prefiltro_embeddings && cos < umbral_embeddings {
                        salteados_por_prefiltro += 1;
                        iniy_core::RelacionNli { entailment: 0.0, contradiction: 0.0, neutral: 1.0 }
                    } else {
                        motor.evaluar(&aserciones[i], &aserciones[j]).await?
                    }
                } else {
                    motor.evaluar(&aserciones[i], &aserciones[j]).await?
                };
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
            store.persistir_implicaciones(&imps)?;
            println!("pares evaluados: {}", imps.len());
            if prefiltro_embeddings {
                println!("pares salteados por embeddings: {} ({:.0}%)",
                    salteados_por_prefiltro,
                    100.0 * salteados_por_prefiltro as f32 / total.max(1) as f32);
            }
            println!("relaciones no triviales: {}  (entailment o contradiction > 0)", no_neutrales);
            let n_reps = store.recalcular_reputaciones()?;
            println!("persistido. reputaciones recalculadas: {n_reps} fuentes.");
        }
        Cmd::Contradictions { doc_id, top } => {
            let (aserciones, imps, alcance) = match doc_id {
                Some(d) => {
                    let id = parse_doc_id(&d)?;
                    let a = store.cargar_aserciones(id)?;
                    if a.is_empty() {
                        anyhow::bail!("doc sin aserciones (corre `iniy extract` y luego `iniy nli`)");
                    }
                    (a, store.cargar_implicaciones_del_doc(id)?, format!("doc {}", id.0))
                }
                None => {
                    let a: Vec<_> = store
                        .cargar_aserciones_atribuidas_todas()?
                        .into_iter()
                        .map(|x| x.asercion)
                        .collect();
                    if a.is_empty() {
                        anyhow::bail!("corpus sin aserciones (corre `iniy extract` y luego `iniy nli`)");
                    }
                    (a, store.cargar_implicaciones_todas()?, "todo el corpus (cross-doc)".to_string())
                }
            };
            if imps.is_empty() {
                anyhow::bail!("sin implicaciones (corre `iniy nli` primero)");
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
                println!("top {} contradicciones en {} (de {} aserciones):", topn.len(), alcance, grafo.cantidad_aserciones());
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
        Cmd::ExportSqlite { archivo } => {
            store.exportar_sqlite(&archivo)?;
            println!("exportado a SQLite: {}", archivo.display());
        }
        Cmd::ImportSqlite { archivo } => {
            let stats = store.importar_sqlite(&archivo)?;
            println!("nuevos / omitidos por colisión de id:");
            println!("  fuentes:       {}  /  {}", stats.fuentes, stats.fuentes_omitidas);
            println!("  documentos:    {}  /  {}", stats.documentos, stats.documentos_omitidos);
            println!("  chunks:        {}  /  {}", stats.chunks, stats.chunks_omitidos);
            println!("  aserciones:    {}  /  {}", stats.aserciones, stats.aserciones_omitidas);
            println!("  implicaciones: {}  /  {}", stats.implicaciones, stats.implicaciones_omitidas);
            println!("  tags(rel):     {}  /  {}", stats.tags, stats.tags_omitidos);
            if stats.implicaciones > 0 {
                let n = store.recalcular_reputaciones()?;
                println!("reputaciones recalculadas: {n} fuentes.");
            }
        }
        Cmd::Export { archivo, pretty } => {
            let dump = store.exportar_todo()?;
            let json = if pretty {
                serde_json::to_string_pretty(&dump)?
            } else {
                serde_json::to_string(&dump)?
            };
            std::fs::write(&archivo, json)?;
            println!("exportado a {} :", archivo.display());
            println!("  {} fuentes, {} docs, {} chunks, {} aserciones, {} implicaciones, {} tags",
                dump.fuentes.len(), dump.documentos.len(), dump.chunks.len(),
                dump.aserciones.len(), dump.implicaciones.len(), dump.documento_tags.len());
        }
        Cmd::Import { archivo } => {
            let json = std::fs::read_to_string(&archivo)?;
            let dump: iniy_store::DbDump = serde_json::from_str(&json)?;
            println!("importando dump v{} (exportado at unix {})", dump.iniy_version, dump.exportado_at);
            let stats = store.importar_dump(&dump)?;
            println!("nuevos / omitidos por colisión de id:");
            println!("  fuentes:       {}  /  {}", stats.fuentes, stats.fuentes_omitidas);
            println!("  documentos:    {}  /  {}", stats.documentos, stats.documentos_omitidos);
            println!("  chunks:        {}  /  {}", stats.chunks, stats.chunks_omitidos);
            println!("  aserciones:    {}  /  {}", stats.aserciones, stats.aserciones_omitidas);
            println!("  implicaciones: {}  /  {}", stats.implicaciones, stats.implicaciones_omitidas);
            println!("  tags(rel):     {}  /  {}", stats.tags, stats.tags_omitidos);
            // Reputaciones son derivadas — recalcular después de import si el grafo creció.
            if stats.implicaciones > 0 {
                let n = store.recalcular_reputaciones()?;
                println!("reputaciones recalculadas: {n} fuentes.");
            }
        }
        Cmd::Ask { pregunta, top, umbral, tag, max_tokens } => {
            use pluma_llm_core::{ChatRequest, ChatMessage};
            let todas = match tag.as_deref() {
                Some(t) => store.cargar_aserciones_atribuidas_por_tag(t)?,
                None => store.cargar_aserciones_atribuidas_todas()?,
            };
            if todas.is_empty() {
                anyhow::bail!("corpus vacío (¿tag inexistente?)");
            }
            // Ranking léxico: cualquier relación NLI (entailment o contradiction) significa
            // "habla del tema". Ordenamos por max(entailment, contradiction) desc.
            let mut ranked: Vec<(f32, &AsercionAtribuida, &'static str)> = Vec::new();
            for att in todas.iter() {
                let rel = relacion_lexica(&pregunta, &att.asercion.texto, umbral);
                let score = rel.entailment.max(rel.contradiction);
                if score > 0.0 {
                    let signo = if rel.entailment >= rel.contradiction { "apoya" } else { "contradice" };
                    ranked.push((score, att, signo));
                }
            }
            ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            ranked.truncate(top);
            if ranked.is_empty() {
                println!("(corpus en silencio sobre «{}» — nada con overlap léxico ≥ {:.2})", pregunta, umbral);
                return Ok(());
            }

            let evidencia: String = ranked.iter().enumerate().map(|(i, (_score, att, signo))| {
                let fuente = match &att.fuente {
                    Some(f) => match &f.kind {
                        Some(k) => format!("{} [{}]", f.nombre, k),
                        None => f.nombre.clone(),
                    },
                    None => "(sin fuente)".to_string(),
                };
                let op = &att.asercion.opinion_autoral;
                format!(
                    "[{}] (Fuente: {} · {} la pregunta · b={:.2} d={:.2} u={:.2}) {}",
                    i + 1, fuente, signo, op.creencia, op.descreencia, op.incertidumbre,
                    att.asercion.texto.trim()
                )
            }).collect::<Vec<_>>().join("\n");

            let system = r#"Eres un asistente de razonamiento crítico. Respondes preguntas usando ESTRICTAMENTE la evidencia que se te provee del corpus. Reglas:

1. Cita cada afirmación de tu respuesta con [N], donde N es el número entre corchetes de la evidencia.
2. Si distintas fuentes se contradicen, dilo explícitamente y muestra ambos lados.
3. Si la evidencia es insuficiente para responder, di "el corpus es insuficiente para responder esto" y explica qué falta.
4. NO inventes ni completes con conocimiento externo no presente en la evidencia.
5. La opinión autoral (b/d/u) de cada aserción es informativa: una aserción con d alto significa que la fuente AFIRMA LA NEGACIÓN — interprétalo correctamente.
6. Respuesta concisa (3-6 oraciones), en el mismo idioma de la pregunta."#;

            let chat = pluma_llm::from_env()
                .map_err(|e| anyhow::anyhow!("LLM no inicializado: {e}"))?;
            println!("backend: {}", chat.model_id());
            println!();

            let user = format!("Pregunta: {pregunta}\n\nEvidencia del corpus:\n{evidencia}\n\nResponde citando con [N]:");
            let req = ChatRequest {
                system: Some(system.to_string()),
                messages: vec![ChatMessage::user(user)],
                max_tokens,
                temperature: 0.2,
            };
            let resp = chat.complete(&req).await
                .map_err(|e| anyhow::anyhow!("LLM falló: {e}"))?;

            println!("─── respuesta ────────────────────────────────────────────");
            println!("{}", resp.content.trim());
            println!();
            println!("─── evidencia usada ({} aserciones) ──────────────────────", ranked.len());
            for (i, (score, att, signo)) in ranked.iter().enumerate() {
                println!("[{}] {} (score={:.2}) · {}", i + 1, signo, score, etiqueta_fuente(att));
                println!("    «{}»", truncar(&att.asercion.texto, 160));
            }
            if let Some(u) = resp.usage {
                println!();
                println!("(tokens: input={} cache_read={} output={})",
                    u.input_tokens, u.cache_read_input_tokens, u.output_tokens);
            }
        }
        Cmd::Stats => {
            let s = store.stats()?;
            // Tamaño de la DB en disco.
            let tamano_db = std::fs::metadata(&cli.db).ok().map(|m| m.len());
            println!("─── corpus en {} ───", cli.db.display());
            if let Some(b) = tamano_db {
                println!("  tamaño DB:        {}", formato_bytes(b));
            }
            println!();
            println!("  conteos:");
            println!("    fuentes:        {:>10}", s.n_fuentes);
            println!("    documentos:     {:>10}", s.n_documentos);
            println!("    chunks:         {:>10}", s.n_chunks);
            println!("    aserciones:     {:>10}", s.n_aserciones);
            println!("    implicaciones:  {:>10}", s.n_implicaciones);
            println!("    tags:           {:>10}", s.n_tags);
            println!("    doc-tags (rel): {:>10}", s.n_documento_tags);
            if s.n_implicaciones > 0 {
                println!();
                println!("  distribución NLI (clase dominante):");
                let pct = |n: u64| 100.0 * n as f32 / s.n_implicaciones as f32;
                println!("    entailment:    {:>10}  ({:>5.1}%)", s.nli_entail, pct(s.nli_entail));
                println!("    contradiction: {:>10}  ({:>5.1}%)", s.nli_contra, pct(s.nli_contra));
                println!("    neutral:       {:>10}  ({:>5.1}%)", s.nli_neutral, pct(s.nli_neutral));
            }
            if let (Some(p), Some(u)) = (s.primero_unix, s.ultimo_unix) {
                println!();
                println!("  rango temporal:");
                println!("    primer doc:  {}", formato_fecha(p));
                println!("    último doc:  {}", formato_fecha(u));
            }
            // Top fuentes por reputación.
            let reps = store.cargar_reputaciones_todas().unwrap_or_default();
            if !reps.is_empty() {
                let fuentes_idx: std::collections::HashMap<FuenteId, iniy_store::FuenteResumen> = store
                    .listar_fuentes()?.into_iter().map(|f| (f.fuente.id, f)).collect();
                let mut con_nombres: Vec<_> = reps.iter().filter_map(|r| {
                    fuentes_idx.get(&r.fuente_id).map(|f| (r.score, f.fuente.nombre.clone()))
                }).collect();
                con_nombres.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                println!();
                println!("  top 5 fuentes por reputación:");
                for (sc, n) in con_nombres.iter().take(5) {
                    println!("    {:+.2}  {}", sc, n);
                }
                if con_nombres.len() > 5 {
                    println!();
                    println!("  bottom 5 fuentes por reputación:");
                    for (sc, n) in con_nombres.iter().rev().take(5) {
                        println!("    {:+.2}  {}", sc, n);
                    }
                }
            }
            // Top tags.
            let tags = store.listar_tags_con_conteo().unwrap_or_default();
            if !tags.is_empty() {
                println!();
                println!("  top 10 tags:");
                for (t, n) in tags.iter().take(10) {
                    println!("    {:>5}  {}", n, t);
                }
            }
        }
        Cmd::Timeline { desde, hasta, tag } => {
            let docs = store.listar_cronologicamente(desde, hasta, tag.as_deref())?;
            if docs.is_empty() {
                println!("(corpus vacío en ese rango / con ese tag)");
                return Ok(());
            }
            let mut acum_asercs = 0u64;
            println!("timeline ({} docs):", docs.len());
            println!();
            for d in docs {
                acum_asercs += d.n_aserciones as u64;
                let fecha = formato_fecha(d.creado_unix);
                let fuente = d.fuente.as_ref().map(|f| f.nombre.as_str()).unwrap_or("(sin fuente)");
                let tags_str = if d.tags.is_empty() { String::new() } else { format!(" #{}", d.tags.join(" #")) };
                println!("  {fecha}  +{:>4} asercs (Σ={acum_asercs})  {}  «{}»{tags_str}",
                    d.n_aserciones, fuente, d.titulo);
            }
        }
        Cmd::Reputacion { recalcular } => {
            let mut persistidas = store.cargar_reputaciones_todas()?;
            if persistidas.is_empty() || recalcular {
                let n = store.recalcular_reputaciones()?;
                println!("(recalculado: {} fuentes)", n);
                persistidas = store.cargar_reputaciones_todas()?;
            }
            if persistidas.is_empty() {
                println!("(corpus vacío — sin reputaciones que calcular)");
                return Ok(());
            }
            // Resolver fuente + n_aserciones por id.
            let fuentes_idx: HashMap<FuenteId, iniy_store::FuenteResumen> = store
                .listar_fuentes()?
                .into_iter()
                .map(|f| (f.fuente.id, f))
                .collect();
            // Orden: score desc, nombre asc para empates.
            persistidas.sort_by(|a, b| b.score.partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal));
            println!("reputación de fuentes (persistida; --recalcular para refrescar):");
            println!();
            for r in &persistidas {
                let Some(fres) = fuentes_idx.get(&r.fuente_id) else { continue; };
                let f = &fres.fuente;
                let kind = f.kind.as_deref().map(|k| format!(" [{k}]")).unwrap_or_default();
                println!("  {:+.2}  {}{}  ({} aserciones)", r.score, f.nombre, kind, fres.n_aserciones);
                println!("        recibe: {}↑ apoyos · {}↓ contradicciones", r.apoyada, r.contradicha);
                println!("         emite: {}↑ apoyos · {}↓ contradicciones", r.apoya, r.contradice);
            }
        }
        Cmd::Tag { doc_id, tag } => {
            let did = parse_doc_id(&doc_id)?;
            store.taggear_doc(did, &tag)?;
            println!("doc {} tagged: «{}»", did.0, tag);
        }
        Cmd::Untag { doc_id, tag } => {
            let did = parse_doc_id(&doc_id)?;
            store.destaggear_doc(did, &tag)?;
            println!("doc {} sin tag «{}»", did.0, tag);
        }
        Cmd::Tags { doc_id } => {
            match doc_id {
                Some(d) => {
                    let did = parse_doc_id(&d)?;
                    let tags = store.tags_de_doc(did)?;
                    if tags.is_empty() {
                        println!("(doc sin tags)");
                    } else {
                        for t in tags { println!("  · {t}"); }
                    }
                }
                None => {
                    let lista = store.listar_tags_con_conteo()?;
                    if lista.is_empty() {
                        println!("(sin tags — usa `iniy tag <doc-id> <tag>`)");
                    } else {
                        for (t, n) in lista {
                            println!("  {:>3} docs  {}", n, t);
                        }
                    }
                }
            }
        }
        Cmd::Cite { asercion_id, fuente, kind, unset } => {
            let aid = parse_asercion_id(&asercion_id)?;
            if unset {
                store.asignar_fuente_citada(aid, None)?;
                println!("cita removida de aserción {}", aid.0);
            } else {
                let nombre = fuente.context("falta el nombre de la fuente (o --unset)")?;
                let fid = store.obtener_o_crear_fuente(&nombre, kind.as_deref())?;
                store.asignar_fuente_citada(aid, Some(fid))?;
                println!("aserción {} citada a «{}»", aid.0, nombre);
            }
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
        Cmd::Consenso { query, umbral, pesar_reputacion, tag, trace } => {
            let todas = match tag.as_deref() {
                Some(t) => store.cargar_aserciones_atribuidas_por_tag(t)?,
                None => store.cargar_aserciones_atribuidas_todas()?,
            };
            if todas.is_empty() {
                anyhow::bail!("corpus vacío (¿tag inexistente?)");
            }
            let reputaciones: std::collections::HashMap<FuenteId, f32> = if pesar_reputacion {
                let persistidas = store.cargar_reputaciones_todas()?;
                if persistidas.is_empty() {
                    // Lazy: si nunca se calcularon, hacerlo ahora.
                    store.recalcular_reputaciones()?;
                    store.cargar_reputaciones_todas()?.into_iter()
                        .map(|r| (r.fuente_id, r.score)).collect()
                } else {
                    persistidas.into_iter().map(|r| (r.fuente_id, r.score)).collect()
                }
            } else {
                std::collections::HashMap::new()
            };
            // Cada contribución guarda la cadena completa para trace:
            // (autoral, post_signo, post_nli, post_rep, att, signo, nli_score, rep_score).
            struct Contrib<'a> {
                autoral: Opinion,
                post_signo: Opinion,    // tras invertir si contradice
                post_nli: Opinion,      // tras descontar por NLI score
                final_op: Opinion,      // tras descontar por reputación (= post_nli si !pesar_reputacion)
                att: &'a AsercionAtribuida,
                signo: &'static str,
                nli_score: f32,
                rep_score: Option<f32>,
            }
            let mut contribuciones: Vec<Contrib> = Vec::new();
            for att in todas.iter() {
                let rel = relacion_lexica(&query, &att.asercion.texto, umbral);
                let (signo, score, post_signo) = if rel.entailment > 0.0 {
                    ("apoya", rel.entailment, att.asercion.opinion_autoral)
                } else if rel.contradiction > 0.0 {
                    ("contradice", rel.contradiction, att.asercion.opinion_autoral.invertir())
                } else {
                    continue;
                };
                let post_nli = post_signo.descontar(score);
                let mut final_op = post_nli;
                let mut rep_aplicada = None;
                if pesar_reputacion {
                    if let Some(fid) = att.fuente.as_ref().map(|f| f.id) {
                        let rep = reputaciones.get(&fid).copied().unwrap_or(0.0);
                        let peso = ((1.0 + rep) / 2.0).clamp(0.0, 1.0);
                        final_op = post_nli.descontar(peso);
                        rep_aplicada = Some(rep);
                    }
                }
                contribuciones.push(Contrib {
                    autoral: att.asercion.opinion_autoral,
                    post_signo,
                    post_nli,
                    final_op,
                    att,
                    signo,
                    nli_score: score,
                    rep_score: rep_aplicada,
                });
            }
            if contribuciones.is_empty() {
                println!("consenso sobre «{}»: (corpus en silencio — nadie habla con suficiente overlap léxico)", query);
                return Ok(());
            }
            let ops: Vec<Opinion> = contribuciones.iter().map(|c| c.final_op).collect();
            let fusion = Opinion::fusionar_muchas(&ops);
            println!("consenso sobre: «{}»", query);
            println!("  fuentes que hablan: {}", contribuciones.len());
            if pesar_reputacion {
                println!("  (opiniones pesadas por reputación de fuente)");
            }
            println!("  opinión fusionada: {}", fila_opinion(&fusion));
            println!("  probabilidad esperada: {:.2}", fusion.probabilidad_esperada());
            println!();
            if trace {
                println!("trace de la derivación:");
                for (k, c) in contribuciones.iter().enumerate() {
                    println!();
                    println!("  [{}]  {}", k + 1, etiqueta_fuente(c.att));
                    println!("       «{}»", truncar(&c.att.asercion.texto, 130));
                    println!("       autoral:    {}", fila_opinion(&c.autoral));
                    if c.signo == "contradice" {
                        println!("       invertir:   {}    (porque {} con query)", fila_opinion(&c.post_signo), c.signo);
                    }
                    println!("       NLI(·{:.2}): {}    ({}, score {:.2})",
                        c.nli_score, fila_opinion(&c.post_nli), c.signo, c.nli_score);
                    if let Some(rep) = c.rep_score {
                        let peso = ((1.0 + rep) / 2.0).clamp(0.0, 1.0);
                        println!("       rep(·{:.2}): {}    (reputación {:+.2} → peso {:.2})",
                            peso, fila_opinion(&c.final_op), rep, peso);
                    }
                }
                println!();
                println!("  fusionar_muchas([{}]) → {}", contribuciones.len(), fila_opinion(&fusion));
            } else {
                println!("contribuciones:");
                for c in &contribuciones {
                    let suf = c.rep_score.map(|r| format!(" · reputación={:+.2}", r)).unwrap_or_default();
                    println!("  · {} (score={:.2}){suf} → {}", c.signo, c.nli_score, fila_opinion(&c.final_op));
                    println!("    {}", etiqueta_fuente(c.att));
                    println!("    «{}»", truncar(&c.att.asercion.texto, 130));
                }
            }
        }
        Cmd::Testimonio { query, umbral, top, tag } => {
            let todas = match tag.as_deref() {
                Some(t) => store.cargar_aserciones_atribuidas_por_tag(t)?,
                None => store.cargar_aserciones_atribuidas_todas()?,
            };
            if todas.is_empty() {
                let hint = match tag.as_deref() {
                    Some(t) => format!("ningún doc con tag «{t}»"),
                    None => "corpus vacío de aserciones (corre `iniy extract` sobre algún doc primero)".into(),
                };
                anyhow::bail!(hint);
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
