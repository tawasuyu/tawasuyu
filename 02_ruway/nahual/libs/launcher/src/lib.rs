//! Yahweh shell — reduce el boot de un app GPUI temed a una línea.
//!
//! Las 4 (próximamente más) apps explorer del repo declaran el mismo
//! patrón: `Application::new + Theme::install_default + cx.open_window
//! + cx.activate(true)`. Sólo varían el título, el tamaño inicial y la
//! fábrica del root entity.
//!
//! Antes (~20 líneas):
//!
//! ```ignore
//! Application::new().run(|cx: &mut App| {
//!     Theme::install_default(cx);
//!     let bounds = Bounds::centered(None, gpui::size(px(900.), px(640.)), cx);
//!     cx.open_window(
//!         WindowOptions {
//!             window_bounds: Some(WindowBounds::Windowed(bounds)),
//!             titlebar: Some(gpui::TitlebarOptions {
//!                 title: Some(SharedString::from("Nakui — Event Log")),
//!                 ..Default::default()
//!             }),
//!             ..Default::default()
//!         },
//!         |_w, cx| cx.new(Explorer::new),
//!     ).expect("open window");
//!     cx.activate(true);
//! });
//! ```
//!
//! Ahora (1 línea):
//!
//! ```ignore
//! launch_app("Nakui — Event Log", (900., 640.), Explorer::new);
//! ```

use gpui::{
    App, AppContext, Application, Bounds, Context, Render, SharedString, TitlebarOptions,
    WindowBounds, WindowOptions, px,
};
use nahual_theme::Theme;

/// Configuración del primer (y normalmente único) ventana del app.
///
/// `size` es `(ancho, alto)` en píxeles lógicos. La ventana queda
/// centrada en el monitor primario.
pub struct AppLaunchConfig {
    pub title: SharedString,
    pub size: (f32, f32),
}

impl AppLaunchConfig {
    pub fn new(title: impl Into<SharedString>, size: (f32, f32)) -> Self {
        Self {
            title: title.into(),
            size,
        }
    }
}

/// Levanta un app GPUI con tema instalado y root entity construido.
///
/// El root debe implementar `Render`. La fábrica `root_factory` recibe
/// el `Context<T>` del nuevo entity para que pueda usar `cx.spawn`,
/// suscribirse a eventos, etc — lo mismo que en el patrón directo.
///
/// Bloquea el thread main hasta que se cierre la ventana
/// (`Application::run` no retorna).
pub fn launch_app<T, F>(title: impl Into<SharedString>, size: (f32, f32), root_factory: F)
where
    T: Render + 'static,
    F: FnOnce(&mut Context<T>) -> T + Send + 'static,
{
    launch_app_with(AppLaunchConfig::new(title, size), root_factory);
}

/// Variante que acepta un `AppLaunchConfig` armado afuera. Útil cuando
/// el config se calcula condicionalmente (env var para tamaño, etc).
pub fn launch_app_with<T, F>(config: AppLaunchConfig, root_factory: F)
where
    T: Render + 'static,
    F: FnOnce(&mut Context<T>) -> T + Send + 'static,
{
    Application::new().run(move |cx: &mut App| {
        Theme::install_default(cx);
        let bounds = Bounds::centered(None, gpui::size(px(config.size.0), px(config.size.1)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(config.title.clone()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_w, cx| cx.new(root_factory),
        )
        .expect("open window");
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_new_normalizes_inputs() {
        let c = AppLaunchConfig::new("My App", (800.0, 600.0));
        assert_eq!(c.title.as_ref(), "My App");
        assert_eq!(c.size, (800.0, 600.0));
    }

    #[test]
    fn config_accepts_owned_string_title() {
        let owned = String::from("Owned Title");
        let c = AppLaunchConfig::new(owned, (400.0, 300.0));
        assert_eq!(c.title.as_ref(), "Owned Title");
    }

    // No hay test de `launch_app` aquí: bloquea el thread main hasta
    // que la ventana se cierre, y en sandbox no hay DISPLAY. La
    // cobertura real es que cada explorer app lo invoque y arranque
    // (smoke test manual o con DISPLAY).
}
