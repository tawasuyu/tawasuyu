//! `puriy-llimphi` — chrome + viewport del navegador sobre Llimphi.
//!
//! Punto de entrada: [`run`]. Toma una URL inicial, abre ventana Llimphi
//! con una pestaña, y delega al motor `puriy-engine` para parsear y
//! computar el [`BoxTree`](puriy_engine::BoxTree). El chrome cablea:
//!
//! - Address bar editable por pestaña (Enter navega, Esc cancela).
//! - Scroll vertical (wheel + PgUp/Dn + ArrowUp/Dn + Home/End).
//! - Links `<a href>` clickeables — disparan `Msg::Navigate`.
//! - Historial por pestaña: Alt+←/Alt+→ (back/forward).
//! - Pestañas múltiples: Ctrl+T (nueva), Ctrl+W (cerrar),
//!   Ctrl+Tab / Ctrl+Shift+Tab (rotar), click en pestaña la activa.
//!
//! Bold se simula con `font_size × 1.1` mientras `llimphi-text` no exponga
//! el eje weight.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};

use llimphi_layout::taffy::prelude::{
    auto, fr, length, percent, AlignContent, AlignItems, AlignSelf, BoxSizing, Dimension,
    FlexDirection, FlexWrap, JustifyContent, LengthPercentageAuto, Position as TaffyPosition, Rect,
    Size, Style,
};
use llimphi_layout::taffy::{
    Display as TaffyDisplay, GridAutoFlow as TaffyGridAutoFlow, GridPlacement, GridTemplateComponent,
    Line as TaffyLine, TrackSizingFunction,
};
use llimphi_raster::kurbo::{
    Affine, BezPath as KurboBezPath, Line, Point, Rect as KurboRect, RoundedRect, Stroke,
};
use llimphi_raster::peniko::{
    Blob, Color, ColorStop, ColorStops, Fill, Gradient, GradientKind,
    ImageAlphaType, ImageBrush as PenikoImage, ImageData, ImageFormat,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_motion::{animate, motion, Tween};
use llimphi_clipboard::SystemClipboard;
// El trait `Clipboard` aporta `get`/`set` sobre `SystemClipboard` — lo usamos
// para puentear `navigator.clipboard` (Fase 7.176) con el portapapeles real.
use llimphi_widget_text_editor::Clipboard as _;
use llimphi_theme::Theme;
use llimphi_module_allichay::AllichayState;

use puriy_engine::{
    AlignItems as CssAlignItems, AlignSelf as CssAlignSelf,
    BackgroundPosition, BackgroundRepeat, BackgroundSize, BorderLineStyle, BoxNode, BoxShadow,
    BoxSizing as CssBoxSizing, BoxTree, Display, Engine, FlexDirection as CssFlexDirection,
    AlignContent as CssAlignContent, FlexWrap as CssFlexWrap, GridAutoFlow, GridTrackSize,
    JustifyContent as CssJustifyContent, LengthVal,
    LinearGradient, Overflow, PointerEvents, Position as CssPosition, TextAlign,
    TextDecorationLine, TextDecorationStyle, VerticalAlign, Visibility,
};

mod canvas;
use canvas::{collect_dom_image_pixels, render_canvas, refresh_canvas_frames, CanvasFrame};
mod render;
use render::*;
mod chrome;
use chrome::*;
mod jsbridge;
use jsbridge::*;
mod nav;
use nav::*;
mod container;
use container::*;
mod settings;
use settings::*;

mod model;
pub use model::*;
mod update;
#[cfg(test)]
mod tests;

const TABS_H: f32 = 30.0;
const LINE_PX: f32 = 24.0;
const NEW_TAB_URL: &str = "about:blank";

/// Punto de entrada — abre ventana Llimphi con una pestaña en `url` sin
/// profile (caches/historial efímeros). Prefiere `run_with_profile` si
/// el caller ya levantó un Profile.
pub fn run(url: String) {
    PURIY_URL.with(|cell| *cell.borrow_mut() = Some(url));
    llimphi_ui::run::<Puriy>();
}

/// Punto de entrada con Profile cableado. El chrome graba en
/// `profile.history` cada navegación exitosa, deja Ctrl+D para
/// bookmarkear, y persiste a `profile_path` después de cada cambio
/// (best-effort, errores silenciosos).
pub fn run_with_profile(
    url: String,
    profile: std::sync::Arc<std::sync::Mutex<puriy_core::Profile>>,
    profile_path: std::path::PathBuf,
) {
    PURIY_URL.with(|cell| *cell.borrow_mut() = Some(url));
    PURIY_PROFILE.with(|cell| *cell.borrow_mut() = Some(profile));
    PURIY_PROFILE_PATH.with(|cell| *cell.borrow_mut() = Some(profile_path));
    llimphi_ui::run::<Puriy>();
}

thread_local! {
    static PURIY_URL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// Viewport actual de la ventana en px físicos (lo actualiza
    /// `Msg::Resize` desde el `on_resize` del runtime). `run_scripts_on_tab`
    /// lo lee para que `window.innerWidth`/`innerHeight` reflejen el tamaño
    /// real ya en la primera ejecución de scripts. Default = `initial_size`.
    static PURIY_VIEWPORT: std::cell::Cell<(f32, f32)> = const { std::cell::Cell::new((1100.0, 760.0)) };
    /// Factor de escala (DPI) actual de la ventana, el `scale_factor` de
    /// winit. Lo actualiza `Msg::ScaleFactor` (desde `on_scale_factor`) y
    /// `run_scripts_on_tab` lo lee para que `window.devicePixelRatio` sea
    /// correcto ya en la primera ejecución de scripts. Default = 1.0.
    static PURIY_DPR: std::cell::Cell<f64> = const { std::cell::Cell::new(1.0) };
    static PURIY_PROFILE: std::cell::RefCell<Option<std::sync::Arc<std::sync::Mutex<puriy_core::Profile>>>> = const { std::cell::RefCell::new(None) };
    static PURIY_PROFILE_PATH: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Devuelve la handle al Profile compartido si el chrome se arrancó vía
/// `run_with_profile`. `None` en el path `run(url)` (efímero).
fn profile_handle() -> Option<std::sync::Arc<std::sync::Mutex<puriy_core::Profile>>> {
    PURIY_PROFILE.with(|c| c.borrow().clone())
}

fn profile_path() -> Option<std::path::PathBuf> {
    PURIY_PROFILE_PATH.with(|c| c.borrow().clone())
}

/// Persiste el Profile a disco si está cableado. Silencioso ante I/O
/// errors — el usuario no necesita ver mensajes del flush.
fn persist_profile() {
    let (Some(handle), Some(path)) = (profile_handle(), profile_path()) else {
        return;
    };
    let Ok(p) = handle.lock() else { return };
    let _ = puriy_core::store::save(&path, &p);
}
