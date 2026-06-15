//! Ruteo del find recursivo del shell nahual (Ctrl+F): traduce los `Msg` del
//! find a mutaciones del `Model` y lanza el worker (`Handle::spawn`) que corre
//! el **motor de búsqueda agnóstico** de `nahual-shell-core` (find por nombre /
//! contenido / semántico). El algoritmo no vive acá (Regla 2): este archivo es
//! sólo la capa Llimphi (concurrencia + Model/Msg + abrir el resultado).

use std::path::{Path, PathBuf};

use llimphi_ui::Handle;
use nahual_shell_core::{build_index, run_find, run_find_semantic};

use crate::modelo::{posix_nav, FindMode, FindState, Model, Msg};

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
        Msg::SemIndexBuild => {
            // Indexa la carpeta del find (o la actual si el find no está abierto).
            let root = m
                .find
                .as_ref()
                .map(|f| f.root.clone())
                .unwrap_or_else(|| PathBuf::from(m.cur().current_id().as_str()));
            m.sem_indexing = true;
            handle.spawn(move || Msg::SemIndexReady(build_index(&root).map(Box::new)));
        }
        Msg::SemIndexReady(idx) => {
            m.sem_indexing = false;
            m.sem_index = idx.map(|b| *b);
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
    // Si hay un índice de embeddings para esta carpeta, la semántica usa el
    // camino rápido (sólo embebe la consulta y rankea contra los vectores
    // cacheados). Si no, embebe por consulta.
    let index: Option<Vec<(PathBuf, Vec<f32>)>> = match (&mode, &m.sem_index) {
        (FindMode::Semantic, Some(idx)) if idx.root == root => Some(idx.entries.clone()),
        _ => None,
    };
    handle.spawn(move || {
        let hits = match mode {
            FindMode::Semantic => run_find_semantic(&root, &query, index),
            _ => run_find(&root, &query, mode),
        };
        Msg::FindResults { gen, hits }
    });
    m
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
