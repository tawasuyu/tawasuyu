//! Puente para hospedar la **shuma COMPLETA** (Model + chrome con dientes/
//! sesiones) en pata, con paridad total al standalone — vs el `shuma.rs` actual
//! que sólo monta un `shuma_module_shell::State` (una sesión, sin rails).
//!
//! Es la pieza (a).4 de la extracción (ver memoria `project_pata_shuma_paridad`):
//! la shuma se quedó agnóstica (escrita en su propio `Msg`/`View<Msg>`) y pata
//! la adapta con los primitivos `Handle::lift` + `View::map` de llimphi, sin
//! reimplementar nada (Regla 2). Acá vive el **puente genérico sobre el `Msg`
//! del host**: construir el Model, engancharle los efectos al loop del host,
//! rutearle eventos y pintarlo elevado al `Msg` de pata.
//!
//! **Estado: scaffold compilable, todavía NO cableado al App vivo de pata.** El
//! `shuma.rs` actual sigue siendo el integration por defecto (cero regresión);
//! el live-wire final (rediseñar barra/drawer para mostrar dientes/sesiones sin
//! perder el input inline) se hace con pantalla para validar a ojo.

use llimphi_ui::{Handle, KeyEvent, Modifiers, View, WheelDelta};
use shuma_shell_llimphi as shuma;

pub use shuma::{Model, Msg};

/// Envoltorio del `Msg` de la shuma con un `Debug` **opaco**. El `Msg` de pata
/// deriva `Debug` (convención del repo), pero el `Msg` de la shuma no lo
/// implementa —arrastra tipos de widgets de terminal/llimphi que no lo derivan—.
/// Este newtype cierra la brecha sin tocar la shuma: pata transporta
/// `Msg::ShumaFull(FullMsg(..))` y `Debug` sólo imprime el discriminante.
#[derive(Clone)]
pub struct FullMsg(pub Msg);

impl std::fmt::Debug for FullMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("shuma::Msg(..)")
    }
}

/// Construye el `Model` de la shuma completa (puro, sin efectos del host),
/// marcado como **hospedado en barra**: el input de la sesión activa lo pinta
/// pata en la barra (ver [`active_input_view`]) y el canvas omite el suyo.
pub fn new() -> Model {
    let mut m = shuma::new_model();
    shuma::set_hosted_in_bar(&mut m, true);
    m
}

/// Engancha los efectos de shuma (ticks, watcher de config, rail, contenedores)
/// al loop del **host** vía un `Handle` lifteado: cada `shuma::Msg` se eleva a
/// `H` con `lift` antes de despacharse al loop de pata. Llamar una vez tras
/// `new()`.
pub fn wire_effects<H, F>(model: &mut Model, handle: &Handle<H>, lift: F)
where
    H: Send + 'static,
    F: Fn(Msg) -> H + Send + Sync + 'static,
{
    let sub = handle.lift(lift);
    shuma::spawn_host_effects(model, &sub);
}

/// Aplica un `shuma::Msg` al `Model`. El `handle` del host se liftea para que
/// los efectos async de shuma (LLM/contenedores/explorer/…) vuelvan al loop de
/// pata. Devuelve el `Model` actualizado (patrón Elm: `m = update(m, msg, …)`).
pub fn update<H, F>(model: Model, msg: Msg, handle: &Handle<H>, lift: F) -> Model
where
    H: Send + 'static,
    F: Fn(Msg) -> H + Send + Sync + 'static,
{
    let sub = handle.lift(lift);
    shuma::update(model, msg, &sub)
}

/// Vista principal de shuma elevada al `Msg` del host: los eventos del árbol de
/// shuma vuelven como `lift(shuma::Msg)`.
pub fn view<H, F>(model: &Model, lift: F) -> View<H>
where
    H: 'static,
    F: Fn(Msg) -> H + Send + Sync + 'static,
{
    shuma::view(model).map(lift)
}

/// Overlay (modales/menús/dropdowns) de shuma elevado, si hay.
pub fn view_overlay<H, F>(model: &Model, lift: F) -> Option<View<H>>
where
    H: 'static,
    F: Fn(Msg) -> H + Send + Sync + 'static,
{
    shuma::view_overlay(model).map(|v| v.map(lift))
}

/// Traduce una tecla a un `shuma::Msg` según el foco interno de shuma.
pub fn on_key(model: &Model, e: &KeyEvent) -> Option<Msg> {
    shuma::on_key(model, e)
}

/// Traduce la rueda a un `shuma::Msg`.
pub fn on_wheel(
    model: &Model,
    delta: WheelDelta,
    cursor: (f32, f32),
    modifiers: Modifiers,
) -> Option<Msg> {
    shuma::on_wheel(model, delta, cursor, modifiers)
}

/// Reacciona a un resize del área hospedada.
pub fn on_resize(model: &Model, width: u32, height: u32) -> Option<Msg> {
    shuma::on_resize(model, width, height)
}

/// Input vivo de la sesión activa elevado al `Msg` del host, para hospedarlo en
/// la barra de pata (el cabezal ES este input, no un placeholder). `None` si la
/// sesión activa no es un shell (form de nueva sesión / sin sesiones).
pub fn active_input_view<H, F>(
    model: &Model,
    theme: &llimphi_theme::Theme,
    lift: F,
) -> Option<View<H>>
where
    H: 'static,
    F: Fn(Msg) -> H + Send + Sync + 'static,
{
    shuma::active_input_view(model, theme).map(|v| v.map(lift))
}

/// `true` si el `Msg` envuelto es el "focalizar el input" de la sesión activa —
/// pata abre su drawer al recibirlo (espeja el auto-open de FocusInput bare).
pub fn msg_is_focus_input(msg: &FullMsg) -> bool {
    shuma::msg_is_focus_input(&msg.0)
}

/// Variante sobre el `Msg` sin envolver (el path layer-shell trabaja con el
/// `Msg` crudo de la shuma, no con [`FullMsg`]).
pub fn msg_is_focus_input_raw(msg: &Msg) -> bool {
    shuma::msg_is_focus_input(msg)
}
