//! iniy-wiki — importador de Wikipedia vía API REST.
//!
//! Descarga artículos, búsquedas o categorías de Wikipedia y los persiste
//! en una DB de iniy como Documentos con fuente "Wikipedia [<lang>]".
//! Cada artículo importado puede taggearse opcionalmente para filtrar
//! después con `iniy testimonio --tag X`.
//!
//! Sin descargar el dump completo (eso sería miles de GB): solo lo que
//! pidas explícitamente. Subset-friendly.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use iniy_ingest::{Chunk, Documento};
use iniy_core::{ChunkId, DocId};
use iniy_store::Store;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "iniy-wiki", about = "Importa Wikipedia a una DB de iniy")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// Ruta al archivo SQLite (default: ./iniy.db)
    #[arg(long, default_value = "iniy.db", global = true)]
    db: PathBuf,

    /// Idioma de Wikipedia: es / en / pt / qu / etc. Default es.
    #[arg(long, default_value = "es", global = true)]
    lang: String,

    /// Tags separados por coma a aplicar a cada artículo importado.
    #[arg(long, global = true)]
    tags: Option<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Descarga un artículo por título exacto. Ej.: `iniy-wiki article "Aristóteles"`.
    Article {
        titulo: String,
    },
    /// Descarga varios artículos por título.
    Articles {
        titulos: Vec<String>,
    },
    /// Busca y descarga los top N resultados.
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        max: usize,
    },
    /// Descarga miembros de una categoría. Ej.:
    /// `iniy-wiki category "Filósofos de la Antigua Grecia" --max 20`.
    Category {
        nombre: String,
        #[arg(long, default_value_t = 20)]
        max: usize,
    },
    /// Bulk import desde un dump XML local de Wikipedia.
    /// Archivos típicos: eswiki-latest-pages-articles.xml.bz2
    /// (~3-5GB comprimido, ~20GB descomprimido) en https://dumps.wikimedia.org/.
    /// Streamea el bz2 sin descomprimirlo a disco; SAX-parse el XML;
    /// procesa solo páginas del namespace 0 (artículos), excluye redirects
    /// y disambiguation. Reportes de progreso cada 1000.
    Dump {
        archivo: PathBuf,
        /// Máximo de artículos a procesar (default: sin límite).
        #[arg(long)]
        max: Option<usize>,
        /// Saltar los primeros N artículos (para resumir tras interrupción).
        #[arg(long, default_value_t = 0)]
        skip: usize,
        /// Tamaño mínimo del wikitext para considerar el artículo
        /// (descarta stubs).
        #[arg(long, default_value_t = 500)]
        min_chars: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    let mut store = Store::abrir(&cli.db)?;
    let cliente = reqwest::Client::builder()
        .user_agent("iniy-wiki/0.1 (https://git.gioser.net/tawasuyu/tawasuyu)")
        .build()?;
    let api_url = format!("https://{}.wikipedia.org/w/api.php", cli.lang);
    let tags: Vec<String> = cli.tags
        .map(|s| s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect())
        .unwrap_or_default();

    let fuente_nombre = format!("Wikipedia [{}]", cli.lang);
    let fuente_id = store.obtener_o_crear_fuente(&fuente_nombre, Some("enciclopedia colaborativa"))?;

    // Caso especial: bulk dump XML local (no usa API HTTP, no usa `titulos`).
    if let Cmd::Dump { archivo, max, skip, min_chars } = &cli.cmd {
        return procesar_dump_xml(archivo, *max, *skip, *min_chars, fuente_id, &tags, &mut store);
    }

    let titulos: Vec<String> = match cli.cmd {
        Cmd::Article { titulo } => vec![titulo],
        Cmd::Articles { titulos } => titulos,
        Cmd::Search { query, max } => buscar(&cliente, &api_url, &query, max).await?,
        Cmd::Category { nombre, max } => categoria_miembros(&cliente, &api_url, &nombre, max).await?,
        Cmd::Dump { .. } => unreachable!(),
    };
    if titulos.is_empty() {
        println!("(sin artículos para importar)");
        return Ok(());
    }
    println!("importando {} artículo(s) de {}...", titulos.len(), api_url);

    for (i, titulo) in titulos.iter().enumerate() {
        eprint!("  [{}/{}] {}... ", i + 1, titulos.len(), titulo);
        match importar_articulo(&cliente, &api_url, titulo, fuente_id, &tags, &mut store).await {
            Ok(doc_id) => eprintln!("doc-id={}", doc_id.0),
            Err(e) => eprintln!("FALLÓ: {e}"),
        }
    }
    println!("listo. corre `iniy --db {} extract <doc-id>` y `iniy nli` después.", cli.db.display());
    Ok(())
}

/// Descarga el wikitext de `titulo` y lo persiste como Documento.
async fn importar_articulo(
    cli: &reqwest::Client,
    api_url: &str,
    titulo: &str,
    fuente_id: iniy_core::FuenteId,
    tags: &[String],
    store: &mut Store,
) -> Result<DocId> {
    let resp: serde_json::Value = cli.get(api_url)
        .query(&[
            ("action", "parse"),
            ("page", titulo),
            ("format", "json"),
            ("formatversion", "2"),
            ("prop", "wikitext"),
            ("redirects", "1"),
        ])
        .send().await?
        .error_for_status()?
        .json().await?;

    if let Some(err) = resp.get("error") {
        anyhow::bail!("API error: {}", err);
    }
    let wikitext = resp
        .pointer("/parse/wikitext")
        .and_then(|v| v.as_str())
        .with_context(|| format!("respuesta sin /parse/wikitext: {resp}"))?;
    let titulo_real = resp
        .pointer("/parse/title")
        .and_then(|v| v.as_str())
        .unwrap_or(titulo)
        .to_string();

    let texto = limpiar_wikitext(wikitext);
    let doc = doc_desde_texto(texto, titulo_real);
    store.persistir_documento(&doc, Some(fuente_id))?;
    for t in tags {
        store.taggear_doc(doc.id, t)?;
    }
    Ok(doc.id)
}

async fn buscar(cli: &reqwest::Client, api_url: &str, query: &str, max: usize) -> Result<Vec<String>> {
    let resp: serde_json::Value = cli.get(api_url)
        .query(&[
            ("action", "opensearch"),
            ("search", query),
            ("limit", &max.to_string()),
            ("format", "json"),
            ("redirects", "resolve"),
        ])
        .send().await?
        .error_for_status()?
        .json().await?;
    let titulos = resp.get(1)
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("respuesta inesperada de opensearch: {resp}"))?
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    Ok(titulos)
}

async fn categoria_miembros(cli: &reqwest::Client, api_url: &str, nombre: &str, max: usize) -> Result<Vec<String>> {
    let cmtitle = if nombre.starts_with("Category:") || nombre.starts_with("Categoría:") {
        nombre.to_string()
    } else {
        format!("Categoría:{}", nombre)
    };
    let resp: serde_json::Value = cli.get(api_url)
        .query(&[
            ("action", "query"),
            ("list", "categorymembers"),
            ("cmtitle", &cmtitle),
            ("cmlimit", &max.to_string()),
            ("cmtype", "page"),
            ("format", "json"),
            ("formatversion", "2"),
        ])
        .send().await?
        .error_for_status()?
        .json().await?;
    let titulos = resp
        .pointer("/query/categorymembers")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("respuesta inesperada de categorymembers: {resp}"))?
        .iter()
        .filter_map(|m| m.get("title").and_then(|t| t.as_str()).map(|s| s.to_string()))
        .collect();
    Ok(titulos)
}

/// Procesa un dump XML bz2 de Wikipedia en streaming.
/// Sin descomprimir a disco; sin cargar el XML completo en memoria.
/// Reporta progreso cada 1000 artículos. SQLite en autocommit mode
/// con batches de 50 docs por transacción.
fn procesar_dump_xml(
    archivo: &std::path::Path,
    max: Option<usize>,
    skip: usize,
    min_chars: usize,
    fuente_id: iniy_core::FuenteId,
    tags: &[String],
    store: &mut Store,
) -> Result<()> {
    use bzip2::read::BzDecoder;
    use quick_xml::events::Event;
    use quick_xml::Reader;
    use std::fs::File;
    use std::io::BufReader;
    use std::time::Instant;

    let f = File::open(archivo)
        .with_context(|| format!("abriendo {}", archivo.display()))?;
    let leido = BufReader::with_capacity(1 << 20, f);
    // Decompresor — solo si el archivo es .bz2; si es .xml plano, leemos directo.
    let leido: Box<dyn std::io::BufRead> = if archivo.extension()
        .and_then(|s| s.to_str()).map(|s| s == "bz2").unwrap_or(false)
    {
        Box::new(BufReader::with_capacity(1 << 20, BzDecoder::new(leido)))
    } else {
        Box::new(leido)
    };

    let mut xml = Reader::from_reader(leido);
    xml.trim_text(true);

    // State machine.
    let mut buf = Vec::with_capacity(1 << 16);
    let mut path = Vec::<String>::with_capacity(8);   // pila de tags abiertos
    let mut titulo = String::new();
    let mut ns = String::new();
    let mut wikitext = String::new();
    let mut es_redirect = false;
    let mut leyendo_text = false;
    let mut procesados = 0usize;        // pages page=ns0 no-redirect contadas
    let mut persistidos = 0usize;
    let inicio = Instant::now();

    println!("procesando {}...", archivo.display());

    loop {
        match xml.read_event_into(&mut buf) {
            Err(e) => anyhow::bail!("XML error en pos {}: {e}", xml.buffer_position()),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                // Detectar <text ... >
                if name == "text" && path.last().map(|s| s.as_str()) == Some("revision") {
                    leyendo_text = true;
                    wikitext.clear();
                }
                if name == "page" {
                    titulo.clear();
                    ns.clear();
                    wikitext.clear();
                    es_redirect = false;
                }
                path.push(name);
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "text" { leyendo_text = false; }
                path.pop();
                if name == "page" {
                    // Filtros.
                    if ns != "0" || es_redirect {
                        continue;
                    }
                    // Disambiguation heurístico: el wikitext contiene plantilla
                    // "{{desambiguación}}" o "{{disambiguation}}" cerca del inicio.
                    let inicio_wt: String = wikitext.chars().take(500).collect::<String>().to_lowercase();
                    if inicio_wt.contains("desambiguación") || inicio_wt.contains("disambiguation") {
                        continue;
                    }
                    procesados += 1;
                    if procesados <= skip {
                        continue;
                    }
                    if wikitext.chars().count() < min_chars {
                        continue;
                    }
                    let texto = limpiar_wikitext(&wikitext);
                    if texto.trim().is_empty() {
                        continue;
                    }
                    let doc = doc_desde_texto(texto, titulo.clone());
                    if doc.chunks.is_empty() {
                        continue;
                    }
                    store.persistir_documento(&doc, Some(fuente_id))?;
                    for t in tags {
                        store.taggear_doc(doc.id, t)?;
                    }
                    persistidos += 1;
                    if persistidos.is_multiple_of(1000) {
                        let secs = inicio.elapsed().as_secs_f64();
                        let rate = persistidos as f64 / secs;
                        println!("  {} persistidos · {:.0} art/s · {:.1}s",
                            persistidos, rate, secs);
                    }
                    if let Some(m) = max {
                        if persistidos >= m {
                            break;
                        }
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "redirect" {
                    es_redirect = true;
                }
            }
            Ok(Event::Text(t)) => {
                let txt = t.unescape().unwrap_or_default();
                match path.last().map(|s| s.as_str()) {
                    Some("title") => titulo.push_str(&txt),
                    Some("ns") => ns.push_str(&txt),
                    Some("text") if leyendo_text => wikitext.push_str(&txt),
                    _ => {}
                }
            }
            Ok(Event::CData(t)) => {
                if leyendo_text {
                    if let Ok(s) = std::str::from_utf8(t.as_ref()) {
                        wikitext.push_str(s);
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    let secs = inicio.elapsed().as_secs_f64();
    println!();
    println!("done. {} artículos persistidos en {:.1}s ({:.0} art/s)",
        persistidos, secs, persistidos as f64 / secs.max(0.001));
    println!("(procesados namespace=0 no-redirect: {})", procesados);
    println!();
    println!("siguiente paso: `iniy extract <doc>` por doc o un script bulk,");
    println!("luego `iniy nli --prefiltro-embeddings --umbral-embeddings 0.7`.");
    Ok(())
}

/// Limpieza heurística de wikitext → texto plano. MVP feo: cubre los
/// constructos más comunes pero no es un parser completo. Suficiente
/// para que `iniy extract` tenga oraciones limpias para chunkear.
pub fn limpiar_wikitext(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut depth_template = 0i32;  // anidamiento de {{...}}
    let mut depth_table = 0i32;     // anidamiento de {|...|}
    let mut in_ref = false;
    let mut in_html = false;

    // Pasada 1: char-por-char para manejar bloques.
    while let Some(c) = chars.next() {
        // Templates {{...}}
        if c == '{' && chars.peek() == Some(&'{') {
            chars.next();
            depth_template += 1;
            continue;
        }
        if c == '}' && chars.peek() == Some(&'}') && depth_template > 0 {
            chars.next();
            depth_template -= 1;
            continue;
        }
        if depth_template > 0 {
            continue;
        }
        // Tablas {|...|}
        if c == '{' && chars.peek() == Some(&'|') {
            chars.next();
            depth_table += 1;
            continue;
        }
        if c == '|' && chars.peek() == Some(&'}') && depth_table > 0 {
            chars.next();
            depth_table -= 1;
            continue;
        }
        if depth_table > 0 {
            continue;
        }
        // <ref>...</ref>
        if c == '<' {
            // Mirar adelante para tags conocidas.
            let resto: String = chars.clone().take(8).collect();
            if resto.starts_with("ref") || resto.starts_with("/ref") {
                in_ref = !resto.starts_with("/ref"); // open o close
                // Consumir hasta '>'.
                for d in chars.by_ref() {
                    if d == '>' {
                        break;
                    }
                }
                if in_ref {
                    // Consumir el cuerpo y la tag de cierre.
                    let mut buf = String::new();
                    while let Some(d) = chars.next() {
                        buf.push(d);
                        if buf.ends_with("</ref>") {
                            in_ref = false;
                            break;
                        }
                    }
                }
                continue;
            }
            // HTML genérico: descarta hasta '>'.
            in_html = true;
            continue;
        }
        if in_html {
            if c == '>' { in_html = false; }
            continue;
        }
        out.push(c);
    }

    // Pasada 2: links [[...]] y formato '''...''' / ''...''.
    let mut out2 = String::with_capacity(out.len());
    let mut it = out.chars().peekable();
    while let Some(c) = it.next() {
        if c == '[' && it.peek() == Some(&'[') {
            it.next();
            let mut buf = String::new();
            while let Some(d) = it.next() {
                if d == ']' && it.peek() == Some(&']') {
                    it.next();
                    break;
                }
                buf.push(d);
            }
            // [[Texto]] o [[Slug|Texto]].
            let display = buf.split('|').next_back().unwrap_or(&buf);
            out2.push_str(display);
            continue;
        }
        if c == '\'' && it.peek() == Some(&'\'') {
            // Negrita ''' o cursiva '': consumir las apóstrofes seguidas.
            it.next();
            while it.peek() == Some(&'\'') { it.next(); }
            continue;
        }
        out2.push(c);
    }

    // Pasada 2.5: normalizar entidades HTML comunes que sobreviven al cleanup.
    let out2 = out2
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–");

    // Pasada 3: encabezados == X == y normalizar saltos.
    let mut out3 = String::with_capacity(out2.len());
    for linea in out2.lines() {
        let trim = linea.trim();
        // Quitar = de encabezados manteniendo el texto.
        let limpia = trim.trim_start_matches('=').trim_end_matches('=').trim();
        // Bullets * o # → quitar prefijo.
        let limpia = limpia.trim_start_matches(|c: char| matches!(c, '*' | '#' | ':' | ';')).trim();
        if !limpia.is_empty() {
            out3.push_str(limpia);
            out3.push('\n');
            out3.push('\n');
        }
    }
    out3
}

fn doc_desde_texto(contenido: String, titulo: String) -> Documento {
    let doc_id = DocId::nuevo();
    let chunks: Vec<Chunk> = contenido
        .split("\n\n")
        .map(str::trim)
        .filter(|s| s.len() >= 40)
        .enumerate()
        .map(|(i, t)| Chunk {
            id: ChunkId::nuevo(),
            doc_id,
            orden: i as u32,
            texto: t.to_string(),
        })
        .collect();
    Documento { id: doc_id, titulo, chunks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limpia_templates() {
        let r = limpiar_wikitext("Texto {{cita|Algo}} importante.");
        assert!(!r.contains("{{"));
        assert!(!r.contains("cita"));
        assert!(r.contains("Texto") && r.contains("importante"));
    }

    #[test]
    fn limpia_links_simples_y_aliados() {
        let r = limpiar_wikitext("Visita [[París]] y [[Roma|Roma capital]].");
        assert!(!r.contains("[["));
        assert!(r.contains("París"));
        assert!(r.contains("Roma capital"));
        assert!(!r.contains("Roma|"));
    }

    #[test]
    fn limpia_refs() {
        let r = limpiar_wikitext("Afirmación <ref>fuente xyz</ref> verificada.");
        assert!(!r.contains("ref"));
        assert!(!r.contains("xyz"));
        assert!(r.contains("Afirmación"));
        assert!(r.contains("verificada"));
    }

    #[test]
    fn limpia_negrita_y_cursiva() {
        let r = limpiar_wikitext("''cursiva'' y '''negrita''' en línea.");
        assert!(!r.contains("'"));
        assert!(r.contains("cursiva"));
        assert!(r.contains("negrita"));
    }

    #[test]
    fn limpia_encabezados() {
        let r = limpiar_wikitext("== Sección ==\n\nTexto.");
        assert!(r.contains("Sección"));
        assert!(!r.contains("=="));
    }
}
