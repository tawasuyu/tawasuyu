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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    let mut store = Store::abrir(&cli.db)?;
    let cliente = reqwest::Client::builder()
        .user_agent("iniy-wiki/0.1 (https://gitea.gioser.net/sergio/gioser)")
        .build()?;
    let api_url = format!("https://{}.wikipedia.org/w/api.php", cli.lang);
    let tags: Vec<String> = cli.tags
        .map(|s| s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect())
        .unwrap_or_default();

    let fuente_nombre = format!("Wikipedia [{}]", cli.lang);
    let fuente_id = store.obtener_o_crear_fuente(&fuente_nombre, Some("enciclopedia colaborativa"))?;

    let titulos: Vec<String> = match cli.cmd {
        Cmd::Article { titulo } => vec![titulo],
        Cmd::Articles { titulos } => titulos,
        Cmd::Search { query, max } => buscar(&cliente, &api_url, &query, max).await?,
        Cmd::Category { nombre, max } => categoria_miembros(&cliente, &api_url, &nombre, max).await?,
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
