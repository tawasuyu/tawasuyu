//! Tahuantinsuyu — binario standalone.
//!
//! Boot:
//! 1. `cosmobiologia_card::spawn_sidecar()` se presenta al Init brahman
//!    (fire-and-forget; si no hay Init, la app sigue standalone).
//! 2. Abre la DB SQLite en `$XDG_DATA_HOME/cosmobiologia/charts.db`
//!    (fallback a `~/.local/share/cosmobiologia/charts.db`).
//! 3. Levanta GPUI con [`nahual_theme::Theme::install_default`].
//! 4. Compone el shell: [`Shell`] dueño del tree (izq), canvas (centro)
//!    y panel (abajo). Cablea las suscripciones cross-widget.
//!
//! ## Layout
//!
//! ```text
//!  ┌───────────┬────────────────────────────────────────┐
//!  │           │                                        │
//!  │   tree    │              canvas                    │
//!  │  (groups, │  (rueda / thumbnails)                  │
//!  │   contacts,│                                       │
//!  │   charts) │                                        │
//!  │           │                                        │
//!  ├───────────┴────────────────────────────────────────┤
//!  │              control panel (módulos)               │
//!  └─────────────────────────────────────────────────────┘
//! ```

mod shell;

use std::path::PathBuf;

use gpui::{
    App, AppContext, Application, Bounds, SharedString, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};

use cosmobiologia_store::Store;
use nahual_theme::Theme;

use crate::shell::Shell;

const DB_FILENAME: &str = "charts.db";
const APP_TITLE: &str = "Tahuantinsuyu";

fn main() {
    // Sidecar brahman primero — si el Init está corriendo, nos presentamos.
    cosmobiologia_card::spawn_sidecar();
    // Service socket: thread separado escuchando ComputeRequest. Otros
    // módulos brahman pueden conectar y pedir cómputos de cartas
    // natales sin GUI. Si el bind falla (socket ya tomado, sin
    // permisos), loggea warn y la app sigue corriendo standalone.
    let service_socket = cosmobiologia_card::service::default_service_socket();
    eprintln!("[cosmobiologia] service socket → {}", service_socket.display());
    cosmobiologia_card::service::spawn_service_thread(service_socket);

    // DB en directorio de datos del usuario.
    let db_path = resolve_db_path();
    let store = match Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[cosmobiologia] no se pudo abrir la DB en {:?}: {} — usando memoria",
                db_path, e
            );
            Store::in_memory().expect("in-memory store")
        }
    };

    Application::new().run(move |cx: &mut App| {
        Theme::install_default(cx);

        let bounds = Bounds::centered(None, size(px(1400.0), px(900.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from(APP_TITLE)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |_w, cx| cx.new(|cx| Shell::new(store.clone(), cx)),
        )
        .expect("open window");
        cx.activate(true);
    });
}

fn resolve_db_path() -> PathBuf {
    if let Some(dirs) = directories::ProjectDirs::from("net", "gioser", "cosmobiologia") {
        let dir = dirs.data_dir().to_path_buf();
        let _ = std::fs::create_dir_all(&dir);
        return dir.join(DB_FILENAME);
    }
    PathBuf::from(DB_FILENAME)
}
