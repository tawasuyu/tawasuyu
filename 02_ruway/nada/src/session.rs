use super::*;

/// Snapshot serializable de la sesion. Solo lo que es semantico — no
/// guardamos scroll positions ni caret per-tab (cambian todo el tiempo).
/// Path al archivo: $XDG_CONFIG_HOME/nada/session.json
/// (o el equivalente en Mac/Windows via directories crate).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Session {
    /// Paths absolutos de los tabs abiertos en el orden que se mostraban.
    open_paths: Vec<PathBuf>,
    /// Indice del tab activo dentro de open_paths.
    active: Option<usize>,
    /// Marks de bookmarks: tuplas (path, line).
    bookmarks: Vec<(PathBuf, usize)>,
    /// Nombre del tema activo (eg "Dark", "Aurora").
    theme_name: String,
}

/// Path donde leemos/escribimos la sesion. None si el SO no expone
/// un config dir conocido (raro).
pub(crate) fn session_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("net", "gioser", "nada")?;
    let dir = dirs.config_dir().to_path_buf();
    let _ = fs::create_dir_all(&dir);
    Some(dir.join("session.json"))
}

/// Construye el snapshot a partir del modelo actual.
pub(crate) fn snapshot_session(model: &Model) -> Session {
    Session {
        open_paths: model.tabs.iter().map(|t| t.path.clone()).collect(),
        active: model.active,
        bookmarks: model.bookmarks.marks.clone(),
        theme_name: model.theme.name.to_string(),
    }
}

/// Persiste la sesion best-effort. Cualquier error se logea pero no
/// rompe el editor (disco lleno, perms, etc. no deberian matar el flow).
pub(crate) fn save_session(model: &Model) {
    let Some(path) = session_path() else { return };
    let snap = snapshot_session(model);
    let Ok(json) = serde_json::to_string_pretty(&snap) else { return };
    // Write atomico: tmp + rename.
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, json).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

/// Carga la sesion previa. None si no existe, esta corrupta o no es
/// parseable — en cualquier caso el editor arranca limpio sin error.
pub(crate) fn load_session() -> Option<Session> {
    let path = session_path()?;
    let raw = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Aplica el snapshot al modelo recien construido. Filtra paths que
/// ya no existen, ajusta el active si quedo fuera de rango, y carga
/// el tema por nombre (fallback a dark si el nombre cambio).
pub(crate) fn restore_session(mut model: Model, sess: Session) -> Model {
    // Tema primero (independiente del resto).
    if let Some(t) = Theme::by_name(&sess.theme_name) {
        model.theme = t;
    }
    // Bookmarks: filtramos paths inexistentes silenciosamente.
    model.bookmarks.marks = sess
        .bookmarks
        .into_iter()
        .filter(|(p, _)| p.is_file())
        .collect();
    // Tabs: abrir cada path existente. open_path agrega al final si no
    // estaba ya abierto. El active se reasigna despues.
    let active_path = sess
        .active
        .and_then(|i| sess.open_paths.get(i).cloned());
    for path in sess.open_paths {
        if path.is_file() {
            model = open_path(model, path);
        }
    }
    // Posicionar active en el path que estaba activo previo (si sobrevivio).
    if let Some(ap) = active_path {
        if let Some(idx) = model.tab_idx_for(&ap) {
            model = activate_tab(model, idx);
        }
    }
    model
}
