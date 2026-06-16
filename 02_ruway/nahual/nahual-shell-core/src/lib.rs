//! `nahual-shell-core` — lógica agnóstica del shell nahual.
//!
//! Acá vive el **motor de búsqueda** (find léxico por nombre/contenido +
//! find semántico por embeddings), el `glob_match` y los tipos de resultado
//! (`FindHit`/`FindMode`/`SemIndex`). Es I/O puro + algoritmo: cero UI, cero
//! Llimphi, cero `Handle`. El frontend (`nahual-shell-llimphi`) maneja la
//! concurrencia (`Handle::spawn`), el `Model`/`Msg` y el ruteo; el algoritmo
//! vive acá para que cualquier frontend (CLI, web) lo reuse. (Regla 2.)

#![forbid(unsafe_code)]

/// Helpers puros de la acción IA (detección de texto, snippets, saneo de
/// nombres). El armado de prompts y la llamada al LLM viven en el frontend.
pub mod ai;
/// Operaciones de archivo del shell (crear/renombrar/borrar/copiar/mover) y su
/// cola. Agnósticas: ejecutan por `nahual_source_core::SourceMut`, sin UI.
pub mod ops;

use std::path::{Path, PathBuf};

// ─── Tipos de dominio del find ──────────────────────────────────────────

/// Modo del find recursivo: por **nombre** (glob sobre el nombre del archivo),
/// por **contenido** (substring dentro de archivos de texto) o **semántico**
/// (embeddings). Tab alterna entre los tres en la UI.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FindMode {
    Name,
    Content,
    /// Búsqueda por significado: embebe la consulta y los candidatos vía el
    /// daemon de verbo y rankea por coseno. Si el daemon no está, degrada a
    /// búsqueda por nombre.
    Semantic,
}

impl FindMode {
    pub fn label(self) -> &'static str {
        match self {
            FindMode::Name => "nombre",
            FindMode::Content => "contenido",
            FindMode::Semantic => "semántico",
        }
    }
    pub fn next(self) -> Self {
        match self {
            FindMode::Name => FindMode::Content,
            FindMode::Content => FindMode::Semantic,
            FindMode::Semantic => FindMode::Name,
        }
    }
}

/// Índice de embeddings de una carpeta, para que la búsqueda semántica sea
/// **instantánea** (sólo se embebe la consulta y se rankea contra estos
/// vectores cacheados, en vez de embeber todo el árbol por consulta). Se
/// construye en background con "Indexar carpeta…".
#[derive(Clone)]
pub struct SemIndex {
    /// Carpeta indexada (el índice sólo aplica a búsquedas posadas acá).
    pub root: PathBuf,
    /// `(ruta, vector)` de cada archivo indexado.
    pub entries: Vec<(PathBuf, Vec<f32>)>,
}

/// Un resultado del find recursivo: la ruta real + cómo mostrarla (relativa al
/// root) + un fragmento opcional (la línea que matcheó, en modo contenido).
#[derive(Clone)]
pub struct FindHit {
    pub path: PathBuf,
    pub display: String,
    pub snippet: Option<String>,
}

// ─── glob ───────────────────────────────────────────────────────────────

/// Match de glob simple, case-insensitive: `*` matchea cualquier secuencia
/// (incluida vacía); el resto es literal. Sin patrón (`*` solo o vacío) o sin
/// comodín, cae a "contiene" para que `foto` encuentre `mi_foto.png`.
pub fn glob_match(pat: &str, name: &str) -> bool {
    let pat = pat.to_lowercase();
    let name = name.to_lowercase();
    if !pat.contains('*') {
        return name.contains(&pat);
    }
    let parts: Vec<&str> = pat.split('*').collect();
    let mut pos = 0usize;
    // Ancla del primer/último fragmento: `*.png` exige terminar en ".png";
    // `foto*` exige empezar con "foto".
    if let Some(first) = parts.first() {
        if !first.is_empty() {
            if !name[pos..].starts_with(first) {
                return false;
            }
            pos += first.len();
        }
    }
    for (i, frag) in parts.iter().enumerate() {
        if frag.is_empty() {
            continue;
        }
        // El primer fragmento ya se ancló arriba.
        if i == 0 {
            continue;
        }
        match name[pos..].find(frag) {
            Some(off) => pos += off + frag.len(),
            None => return false,
        }
    }
    if let Some(last) = parts.last() {
        if !last.is_empty() && parts.len() > 1 {
            return name.ends_with(last);
        }
    }
    true
}

// ─── Topes del walk ───────────────────────────────────────────────────────

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
/// Tope de archivos candidatos a embeber por búsqueda semántica (embeber todo
/// el árbol sería carísimo; tomamos los primeros N del recorrido).
const SEMANTIC_MAX_CANDIDATES: usize = 200;
/// Cuántos resultados semánticos mostrar (los de mayor afinidad).
const SEMANTIC_TOP_N: usize = 40;
/// Tope de archivos a indexar (el índice se cachea, así que aceptamos más que
/// la búsqueda por-consulta: indexás una vez, buscás muchas veces).
const INDEX_MAX_CANDIDATES: usize = 1000;
/// Bytes de contenido que entran al texto a embeber de un archivo de texto.
const SEMANTIC_SNIPPET_BYTES: usize = 2048;

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
pub fn run_find(root: &Path, query: &str, mode: FindMode) -> Vec<FindHit> {
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

/// Coseno entre dos vectores crudos. `-1.0` (mínimo) si las dimensiones no
/// cuadran o alguno es nulo — así nunca rankea arriba un vector inválido.
fn cosine_slices(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return -1.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        -1.0
    } else {
        dot / (na * nb)
    }
}

/// Búsqueda **semántica**: rankea archivos por afinidad con la consulta vía
/// embeddings del daemon de verbo. Corre en el worker (runtime tokio efímero).
///
/// - Si `index` trae vectores pre-calculados de la carpeta (construidos con
///   "Indexar carpeta…"), sólo embebe la consulta y rankea contra ellos —
///   **instantáneo**, sin re-embeber el árbol.
/// - Si no, embebe los candidatos por consulta (más lento, acotado).
/// - Si el daemon no está, degrada a búsqueda por nombre (glob).
pub fn run_find_semantic(
    root: &Path,
    query: &str,
    index: Option<Vec<(PathBuf, Vec<f32>)>>,
) -> Vec<FindHit> {
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
        let consulta = match client.embed(query).await {
            Ok(v) => v,
            Err(_) => return run_find(root, query, FindMode::Name),
        };
        // Camino rápido: rankear contra el índice cacheado.
        if let Some(idx) = index.filter(|i| !i.is_empty()) {
            let mut scored: Vec<(f32, usize)> = idx
                .iter()
                .enumerate()
                .map(|(i, (_, v))| (cosine_slices(&consulta.values, v), i))
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            scored.truncate(SEMANTIC_TOP_N);
            return scored
                .into_iter()
                .map(|(score, i)| hit_for(root, &idx[i].0, Some(format!("afinidad {score:.2}"))))
                .collect();
        }
        // Camino lento: embeber candidatos por consulta.
        let candidatos = collect_candidates(root, SEMANTIC_MAX_CANDIDATES);
        if candidatos.is_empty() {
            return Vec::new();
        }
        let textos: Vec<String> = candidatos.iter().map(|(_, t)| t.clone()).collect();
        let vectores = match client.embed_batch(&textos).await {
            Ok(v) => v,
            Err(_) => return run_find(root, query, FindMode::Name),
        };
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

/// Construye el índice de embeddings de `root`: embebe todos los candidatos y
/// guarda `(ruta, vector)`. Corre en el worker. `None` si el daemon no está o
/// no hay candidatos — el find semántico seguirá funcionando por consulta.
pub fn build_index(root: &Path) -> Option<SemIndex> {
    use rimay_verbo::Provider;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
    rt.block_on(async move {
        let client = rimay_verbo::conectar().await.ok()?;
        let candidatos = collect_candidates(root, INDEX_MAX_CANDIDATES);
        if candidatos.is_empty() {
            return None;
        }
        let textos: Vec<String> = candidatos.iter().map(|(_, t)| t.clone()).collect();
        let vectores = client.embed_batch(&textos).await.ok()?;
        let entries: Vec<(PathBuf, Vec<f32>)> = candidatos
            .into_iter()
            .zip(vectores)
            .map(|((path, _), v)| (path, v.values))
            .collect();
        Some(SemIndex { root: root.to_path_buf(), entries })
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn glob_ancla_extension() {
        assert!(glob_match("*.png", "foto.png"));
        assert!(glob_match("*.PNG", "foto.png")); // case-insensitive
        assert!(!glob_match("*.png", "foto.jpg"));
        assert!(!glob_match("*.png", "png.txt"));
    }

    #[test]
    fn glob_prefijo_y_medio() {
        assert!(glob_match("foto*", "foto_001.png"));
        assert!(!glob_match("foto*", "mi_foto.png"));
        assert!(glob_match("img*2024*", "img_enero_2024_final.jpg"));
        assert!(!glob_match("img*2024*", "img_enero.jpg"));
    }

    #[test]
    fn glob_sin_comodin_contiene() {
        assert!(glob_match("foto", "mi_FOTO_grande.png"));
        assert!(!glob_match("foto", "imagen.png"));
    }

    #[test]
    fn cosine_slices_basico() {
        // Idénticos → 1.0; ortogonales → 0.0; dims distintas → -1.0.
        assert!((cosine_slices(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine_slices(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine_slices(&[1.0], &[1.0, 0.0]), -1.0);
    }

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

        let hits = run_find_semantic(root, "*.rs", None);
        // El fallback es run_find Name con la query como glob → sólo informe.rs.
        assert_eq!(hits.len(), 1);
        assert!(hits[0].display.contains("informe.rs"));
    }
}
