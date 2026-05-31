//! Trait `Widget` y mensajes de la app.
//!
//! Un widget mantiene su estado (lo que el tick refresca) y sabe pintarse
//! dentro de la barra. Es trait-object: la lista de widgets vive como
//! `Vec<Box<dyn Widget>>` por slot (left/center/right).

use llimphi_theme::Theme;
use llimphi_ui::{KeyEvent, View};

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
    /// Resultado de una llamada a IA (texto o mensaje de error).
    QuakeIaResult(Result<String, String>),
    /// Foco al input de la shuma_bar (toggle del overlay full).
    ShumaToggle,
    /// Carácter al input de la shuma_bar.
    ShumaChar(char),
    /// Backspace en shuma_bar.
    ShumaBackspace,
    /// Enter en shuma_bar — ejecuta el comando.
    ShumaSubmit,
    /// Resultado de un comando shell ejecutado por shuma_bar
    /// (stdout o mensaje de error).
    ShumaResult(Result<String, String>),
    /// Abrir/cerrar un menú raíz de la barra (`None` = cerrar todos).
    MenuOpen(Option<usize>),
    /// Comando elegido en la barra de menú (id `menu.<verbo>` mapeado a
    /// una acción real de la app).
    MenuCommand(String),
    /// Cerrar cualquier menú/dropdown abierto.
    CloseMenus,
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
    /// Acceso `Any` de sólo lectura — para que la app loop pueda
    /// preguntar sobre estado interno sin tener que abrirlo como mut
    /// (p. ej. "¿está abierto el quake?").
    fn as_any(&self) -> &dyn std::any::Any;
    /// Si el widget reconoce esta tecla (vía su prop `hotkey` o lógica
    /// interna), devuelve el `Msg` a despachar. Default: nada. La app
    /// loop consulta todos los widgets antes de hacer su routing
    /// estándar (input al quake si está abierto, etc.).
    fn try_key(&self, _event: &KeyEvent) -> Option<Msg> {
        None
    }
}
