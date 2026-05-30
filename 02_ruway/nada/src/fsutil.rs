//! Filesystem: scan/walk del árbol, mtimes, git status, labels.
#![allow(unused_imports)]
use crate::prelude::*;
use crate::*;
use crate::actions::*;
use crate::fsutil::*;
use crate::view::*;
use crate::session::*;
use crate::clipboard::*;
use crate::keys::*;
use crate::update::*;
pub(crate) fn scan_root(root: &Path) -> Vec<TreeNode> {
    let mut out: Vec<TreeNode> = Vec::new();
    visit_dir(root, 0, false, &mut out);
    out
}

/// Walk recursivo: todos los archivos bajo `root`, excluyendo dotfiles,
/// `target/` y `node_modules/`. Devuelve paths absolutos. Cap a 50k para
/// que un mal directorio no funda RAM.
pub(crate) const PICKER_FILE_CAP: usize = 50_000;
pub(crate) fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= PICKER_FILE_CAP {
            break;
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else { continue };
            if name_str.starts_with('.') || name_str == "target" || name_str == "node_modules" {
                continue;
            }
            let path = entry.path();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                stack.push(path);
            } else {
                out.push(path);
                if out.len() >= PICKER_FILE_CAP {
                    break;
                }
            }
        }
    }
    out.sort();
    out
}

pub(crate) fn visit_dir(dir: &Path, depth: usize, into_expanded: bool, out: &mut Vec<TreeNode>) {
    let _ = into_expanded;
    let mut entries: Vec<(PathBuf, bool)> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| !n.starts_with('.') && n != "target" && n != "node_modules")
                    .unwrap_or(false)
            })
            .map(|e| {
                let p = e.path();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                (p, is_dir)
            })
            .collect(),
        Err(_) => return,
    };
    // Directorios primero, luego archivos; ambos alfabéticos.
    entries.sort_by(|a, b| match (a.1, b.1) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.0.file_name().cmp(&b.0.file_name()),
    });

    for (path, is_dir) in entries {
        out.push(TreeNode {
            path: path.clone(),
            depth,
            is_dir,
            expanded: false,
        });
    }
}


pub(crate) fn file_mtime(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Ejecuta `git status --porcelain -z` desde `root` y devuelve un mapa
/// `path absoluto → marca corta` (`M` modified, `A` added, `D` deleted,
/// `?` untracked, `R` renamed, `U` unmerged). Si no es un repo git o
/// `git` falla, devuelve mapa vacío. Bloqueante; corre en un hilo.
pub(crate) fn query_git_status(root: &Path) -> GitStatusMap {
    use std::process::Command;
    let mut out = GitStatusMap::new();
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("status")
        .arg("--porcelain")
        .arg("-z")
        .output()
    else {
        return out;
    };
    if !output.status.success() {
        return out;
    }
    // Formato -z: entradas separadas por NUL. Cada entrada empieza con
    // "XY path" donde XY son 2 chars de estado (X=index, Y=worktree).
    // Renames usan dos paths separados por otro NUL: "R  newname\0oldname".
    let mut iter = output.stdout.split(|b| *b == 0).peekable();
    while let Some(entry) = iter.next() {
        if entry.len() < 4 {
            continue;
        }
        let xy = &entry[..2];
        let rest = &entry[3..];
        let mark = pick_git_mark(xy);
        let path_str = match std::str::from_utf8(rest) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let abs = root.join(path_str);
        out.insert(abs, mark);
        // Rename consume el path "old" siguiente.
        if xy[0] == b'R' || xy[1] == b'R' {
            iter.next();
        }
    }
    out
}

pub(crate) fn pick_git_mark(xy: &[u8]) -> char {
    // Prioridad simple: untracked > conflict > added > deleted > modified.
    if xy == b"??" { return '?'; }
    if xy[0] == b'U' || xy[1] == b'U' || xy == b"AA" || xy == b"DD" {
        return 'U';
    }
    if xy[0] == b'A' || xy[1] == b'A' { return 'A'; }
    if xy[0] == b'D' || xy[1] == b'D' { return 'D'; }
    if xy[0] == b'R' || xy[1] == b'R' { return 'R'; }
    'M'
}

/// Mueve `path` al frente de la cola LRU; deduplica. Trunca a
/// `RECENT_FILES_CAP` para no crecer sin límite.
pub(crate) fn push_recent(q: &mut std::collections::VecDeque<PathBuf>, path: &Path) {
    if let Some(pos) = q.iter().position(|p| p == path) {
        q.remove(pos);
    }
    q.push_front(path.to_path_buf());
    while q.len() > RECENT_FILES_CAP {
        q.pop_back();
    }
}

/// Construye un Vec de paths con los recientes al frente (en orden LRU)
/// + el resto de `all_files` filtrado para no duplicar. El picker filtra
/// linealmente y mantiene el orden — el user ve sus archivos recientes
/// arriba antes de tipear nada.
pub(crate) fn files_with_recents_first(
    recents: &std::collections::VecDeque<PathBuf>,
    all_files: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(all_files.len());
    let mut seen: std::collections::HashSet<&Path> = std::collections::HashSet::new();
    for p in recents {
        if seen.insert(p.as_path()) {
            out.push(p.clone());
        }
    }
    for p in all_files {
        if seen.insert(p.as_path()) {
            out.push(p.clone());
        }
    }
    out
}

/// Resumen "git: 3M 1?" para la status bar. Vacío si no hay cambios.
pub(crate) fn git_summary(map: &GitStatusMap) -> String {
    if map.is_empty() {
        return String::new();
    }
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<char, usize> = BTreeMap::new();
    for &m in map.values() {
        *counts.entry(m).or_insert(0) += 1;
    }
    let parts: Vec<String> = counts.iter().map(|(c, n)| format!("{n}{c}")).collect();
    format!("git: {}", parts.join(" "))
}

/// Compara mtimes en disco vs `Tab.last_mtime` para cada tab. Si difiere:
/// - tab no-dirty → recarga buffer desde disco y actualiza el LSP.
/// - tab dirty → status warn (una sola vez vía `external_warned`).
pub(crate) fn detect_external_changes(m: &mut Model) {
    let n = m.tabs.len();
    for idx in 0..n {
        let path = m.tabs[idx].path.clone();
        let disk = file_mtime(&path);
        let known = m.tabs[idx].last_mtime;
        // Si nunca tuvimos mtime, lo seteamos y seguimos.
        if known.is_none() {
            m.tabs[idx].last_mtime = disk;
            continue;
        }
        if disk == known {
            // Sin cambios — limpiamos la alerta si el user salvó/aceptó
            // afuera (el caso típico es que disk == known otra vez
            // tras un save manual).
            m.tabs[idx].external_warned = false;
            continue;
        }
        if !m.tabs[idx].dirty {
            if let Ok(content) = fs::read_to_string(&path) {
                m.tabs[idx].editor.set_text(&content);
                m.tabs[idx].last_mtime = disk;
                m.tabs[idx].external_warned = false;
                m.lsp.did_change(&path, &content);
                m.status = format!(
                    "recargado · {} cambió en disco",
                    relative_to(&m.root, &path),
                );
            }
        } else if !m.tabs[idx].external_warned {
            m.tabs[idx].external_warned = true;
            m.status = format!(
                "⚠ {} cambió en disco — guardar sobreescribe; Ctrl+S forzaría",
                relative_to(&m.root, &path),
            );
        }
    }
}

pub(crate) fn row_label(n: &TreeNode) -> String {
    let name = n.path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
    // Sin prefijo Unicode/emoji — el chevron del tree widget ya distingue
    // dirs (v/>) de archivos (espacio). Las fuentes default no tienen
    // glyphs para 📁/📄 y dibujan cuadrados de fallback.
    if n.is_dir {
        format!("{name}/")
    } else {
        name.to_owned()
    }
}

/// Variante del label que antepone la marca git del archivo (si existe).
/// Para dirs, agrega `*` si algún descendiente está marcado.
pub(crate) fn row_label_with_git(n: &TreeNode, git: &GitStatusMap) -> String {
    let base = row_label(n);
    if n.is_dir {
        let has = git.keys().any(|p| p.starts_with(&n.path));
        if has { format!("* {base}") } else { base }
    } else {
        match git.get(&n.path) {
            Some(c) => format!("{c} {base}"),
            None => base,
        }
    }
}

pub(crate) fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

pub(crate) fn language_for_path(path: &Path) -> Language {
    let ext = path.extension().and_then(OsStr::to_str).unwrap_or("");
    Language::from_cell_language(ext)
}

// ---------------------------------------------------------------------
// Clipboard backend (arboard)
// ---------------------------------------------------------------------

