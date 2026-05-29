//! Trait `Widget` y mensajes de la app.
//!
//! Un widget mantiene su estado (lo que el tick refresca) y sabe pintarse
//! dentro de la barra. Es trait-object: la lista de widgets vive como
//! `Vec<Box<dyn Widget>>` por slot (left/center/right).

use llimphi_theme::Theme;
use llimphi_ui::View;

/// Mensajes que la app entiende. Los widgets que reaccionan a input los
/// emiten desde su `view()` o desde `on_key` del App.
#[derive(Clone, Debug)]
pub enum Msg {
    /// Refresh periódico (1 Hz) — los meters releen sus datos.
    Tick,
    /// Mostrar/ocultar el input quake.
    QuakeToggle,
    /// El input recibió una tecla normal.
    QuakeChar(char),
    /// Backspace en el input.
    QuakeBackspace,
    /// Enter en el input — submit del comando actual.
    QuakeSubmit,
    /// Cerrar la app.
    Quit,
}

/// Un control individual del panel. `tick` corre fuera del view (al `Msg::Tick`
/// global); `view` se llama cada frame con el tema actual.
///
/// El `as_any_mut` permite a la app rutear `Msg`s específicos al widget que
/// los entiende (caso quake_input). Sin él habría que crecer el trait con
/// un `on_msg` que casi todos los widgets ignorarían.
pub trait Widget: Send + 'static {
    /// Refresca datos internos. Default: no hace nada.
    fn tick(&mut self) {}
    /// Pinta el widget. El alto disponible es el del panel; el ancho
    /// es elegido por el widget vía el `Style` de su `View::new`.
    fn view(&self, theme: &Theme) -> View<Msg>;
    /// Acceso `Any` para downcast a un widget concreto cuando la app
    /// necesita mutarlo con un mensaje específico.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}
