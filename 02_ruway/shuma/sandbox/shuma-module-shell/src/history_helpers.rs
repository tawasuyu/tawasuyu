use super::*;

pub(crate) fn open_history() -> shuma_history::History {
    if let Some(path) = shuma_history::History::default_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(h) = shuma_history::History::open(&path) {
            return h;
        }
    }
    // Fallback: historial en /dev/null (existe siempre, append-only OK).
    shuma_history::History::open(std::path::PathBuf::from("/dev/null"))
        .unwrap_or_else(|_| panic!("no se pudo abrir ni /dev/null como history"))
}

/// Absorbe los historiales de bash/zsh al historial propio (incremental).
/// No-op si no hay fuentes en disco o si nada creció desde la última vez.
/// Devuelve cuántas líneas se importaron (0 = nada nuevo).
pub(crate) fn absorb_shell_histories(history: &mut shuma_history::History) -> usize {
    let sources = shuma_history::foreign::default_sources();
    if sources.is_empty() {
        return 0;
    }
    shuma_history::foreign::absorb_foreign(history, &sources).imported
}

/// Segundos unix actuales (0 si el reloj está antes de la época).
pub(crate) fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Milisegundos unix actuales — para el parpadeo del caret del input.
pub(crate) fn now_unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

