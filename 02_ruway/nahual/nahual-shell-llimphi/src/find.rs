//! Find recursivo del shell nahual (Ctrl+F): camina el árbol bajo la carpeta
//! actual en un worker (`Handle::spawn`, sin colgar la UI) y lista los matches,
//! por **nombre** (glob) o por **contenido** (substring en archivos de texto).
//! Es la base sobre la que enchufar la búsqueda **semántica** (embeddings vía
//! el daemon de verbo) como un tercer modo.

use std::path::{Path, PathBuf};

use llimphi_ui::Handle;

use crate::modelo::{posix_nav, FindHit, FindMode, FindState, Model, Msg};
use crate::update::glob_match;

/// Tope de resultados de una búsqueda — acota la lista y el trabajo del worker.
const MAX_HITS: usize = 500;
/// Tope de entradas visitadas — backstop para árboles enormes (no caminamos
/// el filesystem entero si el usuario abre el find en `/`).
const MAX_VISITED: usize = 200_000;
/// Profundidad máxima del recorrido.
const MAX_DEPTH: usize = 24;
/// Tope de bytes leídos por archivo en modo contenido (los matches útiles
/// están al principio; no slurpeamos un log de 1 GB).
const CONTENT_BYTES_MAX: usize = 512 * 1024;

/// Carpetas que nunca vale la pena caminar (ruido + costo): VCS, builds, deps.
fn dir_ignorada(name: &str) -> bool {
    matches!(name, ".git" | "target" | "node_modules" | ".cache" | "__pycache__")
}

/// ¿La extensión sugiere texto grepeble? Filtro barato antes de leer bytes en
/// modo contenido (no grepeamos binarios).
fn es_texto(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase) {
        Some(ext) => matches!(
            ext.as_str(),
            "rs" | "toml" | "md" | "txt" | "json" | "yaml" | "yml" | "html" | "css" | "js"
                | "ts" | "py" | "c" | "h" | "cpp" | "hpp" | "go" | "java" | "sh" | "lua"
                | "rb" | "php" | "sql" | "xml" | "ini" | "cfg" | "conf" | "log" | "csv"
                | "tsv" | "rhai" | "wat"
        ),
        None => false,
    }
}

/// Camina `root` recursivamente acumulando matches según `mode`/`query`. Corre
/// en un worker — es puro I/O sincrónico, acotado por los topes de arriba.
pub(crate) fn run_find(root: &Path, query: &str, mode: FindMode) -> Vec<FindHit> {
    let mut hits: Vec<FindHit> = Vec::new();
    let mut visited = 0usize;
    // BFS por niveles con un stack explícito (evita recursión profunda).
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if hits.len() >= MAX_HITS || visited >= MAX_VISITED {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            visited += 1;
            if hits.len() >= MAX_HITS || visited >= MAX_VISITED {
                break;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            // Saltamos ocultos y carpetas de ruido.
            if name.starts_with('.') && name != "." {
                if path.is_dir() {
                    continue;
                }
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                // En modo nombre, una carpeta que matchea también es un hit
                // (antes de mover `path` al stack).
                if mode == FindMode::Name && glob_match(query, &name) {
                    hits.push(hit_for(root, &path, None));
                }
                if !dir_ignorada(&name) && depth + 1 <= MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            match mode {
                FindMode::Content => {
                    if es_texto(&path) {
                        if let Some(snippet) = grep_first(&path, query) {
                            hits.push(hit_for(root, &path, Some(snippet)));
                        }
                    }
                }
                // Nombre (y el fallback de Semantic, que se rutea acá): glob.
                FindMode::Name | FindMode::Semantic => {
                    if glob_match(query, &name) {
                        hits.push(hit_for(root, &path, None));
                    }
                }
            }
        }
    }
    hits
}

/// Construye un `FindHit` con la ruta mostrada relativa al root.
fn hit_for(root: &Path, path: &Path, snippet: Option<String>) -> FindHit {
    let display = path
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    FindHit { path: path.to_path_buf(), display, snippet }
}

/// Primera línea de `path` que contiene `needle` (case-insensitive), recortada.
/// `None` si no hay match o el archivo no se lee.
fn grep_first(path: &Path, needle: &str) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; CONTENT_BYTES_MAX];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    let texto = String::from_utf8_lossy(&buf);
    let needle_low = needle.to_lowercase();
    for line in texto.lines() {
        if line.to_lowercase().contains(&needle_low) {
            let trimmed = line.trim();
            let corto: String = trimmed.chars().take(120).collect();
            return Some(corto);
        }
    }
    None
}

/// Dispatcher de los `Msg` del find. Devuelve el modelo mutado; lanza el worker
/// vía `handle.spawn` cuando hay que buscar.
pub(crate) fn apply_find(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
    let mut m = model;
    match msg {
        Msg::FindOpen => {
            let root = PathBuf::from(m.cur().current_id().as_str());
            m.find = Some(FindState::new(root));
        }
        Msg::FindClose => {
            m.find = None;
        }
        Msg::FindInput(s) => {
            if let Some(f) = m.find.as_mut() {
                f.query.push_str(&s);
            }
        }
        Msg::FindBackspace => {
            if let Some(f) = m.find.as_mut() {
                f.query.pop();
            }
        }
        Msg::FindToggleMode => {
            if let Some(f) = m.find.as_mut() {
                f.mode = f.mode.next();
                // Cambiar de modo invalida lo corrido (otra semántica).
                f.ran = None;
            }
        }
        Msg::FindNav(d) => {
            if let Some(f) = m.find.as_mut() {
                let n = f.results.len() as i32;
                if n > 0 {
                    f.selected = (f.selected as i32 + d).rem_euclid(n) as usize;
                }
            }
        }
        Msg::FindSubmit => return find_submit(m, handle),
        Msg::FindResults { gen, hits } => {
            if let Some(f) = m.find.as_mut() {
                // Sólo aceptamos los resultados de la búsqueda vigente.
                if gen == f.gen {
                    f.results = hits;
                    f.selected = 0;
                    f.searching = false;
                }
            }
        }
        _ => {}
    }
    m
}

/// Enter en el find: si la búsqueda vigente ya corrió con esta `(query, mode)`
/// y hay resultados, abre el seleccionado; si no, lanza la búsqueda.
fn find_submit(mut m: Model, handle: &Handle<Msg>) -> Model {
    let Some(f) = m.find.as_mut() else { return m };
    let query = f.query.trim().to_string();
    if query.is_empty() {
        return m;
    }
    let ya_corrio = f.ran.as_ref() == Some(&(query.clone(), f.mode));
    if ya_corrio && !f.results.is_empty() {
        // Abrir el resultado: navega a su carpeta contenedora y selecciona el
        // archivo (deja que el doble-flujo normal lo previsualice).
        let hit = f.results[f.selected].clone();
        m.find = None;
        abrir_resultado(&mut m, &hit.path);
        return m;
    }
    // Lanzar la búsqueda en un worker.
    f.gen += 1;
    f.searching = true;
    f.ran = Some((query.clone(), f.mode));
    let gen = f.gen;
    let mode = f.mode;
    let root = f.root.clone();
    handle.spawn(move || {
        let hits = match mode {
            FindMode::Semantic => run_find_semantic(&root, &query),
            _ => run_find(&root, &query, mode),
        };
        Msg::FindResults { gen, hits }
    });
    m
}

/// Búsqueda **semántica**: embebe la consulta y los archivos candidatos vía el
/// daemon de verbo y rankea por coseno. Corre en el worker (forja un runtime
/// tokio efímero). Si el daemon no está corriendo o falla, degrada a búsqueda
/// por nombre (glob) para que el modo siempre devuelva algo útil.
pub(crate) fn run_find_semantic(root: &Path, query: &str) -> Vec<FindHit> {
    use rimay_verbo::Provider;
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        return run_find(root, query, FindMode::Name);
    };
    rt.block_on(async move {
        let client = match rimay_verbo::conectar().await {
            Ok(c) => c,
            Err(_) => {
                eprintln!("[nahual] find semántico: daemon de verbo no disponible — caigo a nombre");
                return run_find(root, query, FindMode::Name);
            }
        };
        // Candidatos: archivos bajo el root (acotado), con un texto a embeber
        // (nombre + snippet de contenido si es texto).
        let candidatos = collect_candidates(root, SEMANTIC_MAX_CANDIDATES);
        if candidatos.is_empty() {
            return Vec::new();
        }
        let consulta = match client.embed(query).await {
            Ok(v) => v,
            Err(_) => return run_find(root, query, FindMode::Name),
        };
        let textos: Vec<String> = candidatos.iter().map(|(_, t)| t.clone()).collect();
        let vectores = match client.embed_batch(&textos).await {
            Ok(v) => v,
            Err(_) => return run_find(root, query, FindMode::Name),
        };
        // Rankeo por coseno descendente.
        let mut scored: Vec<(f32, usize)> = vectores
            .iter()
            .enumerate()
            .filter_map(|(i, v)| consulta.cosine(v).ok().map(|s| (s, i)))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(SEMANTIC_TOP_N);
        scored
            .into_iter()
            .map(|(score, i)| {
                let (path, _) = &candidatos[i];
                hit_for(root, path, Some(format!("afinidad {score:.2}")))
            })
            .collect()
    })
}

/// Tope de archivos candidatos a embeber por búsqueda semántica (embeber todo
/// el árbol sería carísimo; tomamos los primeros N del recorrido).
const SEMANTIC_MAX_CANDIDATES: usize = 200;
/// Cuántos resultados semánticos mostrar (los de mayor afinidad).
const SEMANTIC_TOP_N: usize = 40;
/// Bytes de contenido que entran al texto a embeber de un archivo de texto.
const SEMANTIC_SNIPPET_BYTES: usize = 2048;

/// Junta hasta `max` archivos bajo `root` con el texto a embeber de cada uno:
/// el nombre + (si es texto) un snippet del contenido. Misma poda de ruido que
/// el walk literal.
fn collect_candidates(root: &Path, max: usize) -> Vec<(PathBuf, String)> {
    use std::io::Read;
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= max {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            if out.len() >= max {
                break;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if !name.starts_with('.') && !dir_ignorada(&name) && depth + 1 <= MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if name.starts_with('.') {
                continue;
            }
            let mut texto = name.clone();
            if es_texto(&path) {
                if let Ok(mut f) = std::fs::File::open(&path) {
                    let mut buf = vec![0u8; SEMANTIC_SNIPPET_BYTES];
                    if let Ok(n) = f.read(&mut buf) {
                        buf.truncate(n);
                        texto.push('\n');
                        texto.push_str(&String::from_utf8_lossy(&buf));
                    }
                }
            }
            out.push((path, texto));
        }
    }
    out
}

/// Navega el panel enfocado a la carpeta que contiene `path` y selecciona el
/// archivo (o la carpeta misma si el hit es un dir). Reusa el `posix_nav` del
/// shell para sembrar el breadcrumb completo.
fn abrir_resultado(m: &mut Model, path: &Path) {
    let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
    m.cur_pane_mut().nav_stack = vec![posix_nav(dir)];
    m.cur_pane_mut().marked.clear();
    m.canvas = None;
    // Selecciona el archivo dentro de la carpeta (su id ES la ruta POSIX).
    let id = path.to_string_lossy().into_owned();
    m.cur_mut().select_id(&id);
    crate::helpers::apply_format(m);
    crate::helpers::record_history(m);
    // Revela la carpeta en el árbol lateral.
    for anc in crate::helpers::ancestors_set(dir) {
        m.tree_expanded.insert(anc);
    }
    crate::helpers::ensure_children_for_expanded(&mut m.tree_children, &m.tree_expanded);
    // Abre el preview del archivo seleccionado.
    m.viewer_open = true;
    crate::helpers::refresh_preview(m);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn find_por_nombre_y_contenido() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("alpha.txt"), b"hola mundo\nsegunda linea").unwrap();
        fs::write(root.join("sub/beta.rs"), b"fn main() { /* token magico */ }").unwrap();
        fs::write(root.join("sub/gamma.png"), b"\x89PNG binario").unwrap();
        // Carpeta de ruido: no debe aparecer.
        fs::create_dir(root.join("target")).unwrap();
        fs::write(root.join("target/no.txt"), b"token magico").unwrap();

        // Por nombre: glob *.rs encuentra beta.rs y nada más.
        let by_name = run_find(root, "*.rs", FindMode::Name);
        assert_eq!(by_name.len(), 1);
        assert!(by_name[0].display.contains("beta.rs"));

        // Por contenido: "magico" matchea beta.rs (texto) pero NO el de target/
        // (carpeta ignorada) ni el png (no es texto).
        let by_content = run_find(root, "magico", FindMode::Content);
        assert_eq!(by_content.len(), 1, "sólo beta.rs, target/ se ignora");
        assert!(by_content[0].snippet.as_deref().unwrap().contains("token magico"));
    }

    #[test]
    fn semantico_degrada_a_nombre_sin_daemon() {
        // En el entorno de test no hay daemon de verbo corriendo, así que la
        // búsqueda semántica debe degradar a búsqueda por nombre — sin panic y
        // devolviendo los matches de nombre (no cuelga ni explota).
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("informe.rs"), b"contenido").unwrap();
        fs::write(root.join("otro.txt"), b"contenido").unwrap();

        let hits = run_find_semantic(root, "*.rs");
        // El fallback es run_find Name con la query como glob → sólo informe.rs.
        assert_eq!(hits.len(), 1);
        assert!(hits[0].display.contains("informe.rs"));
    }
}
