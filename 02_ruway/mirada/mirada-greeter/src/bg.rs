//! Despachador del fondo animado del greeter. El fondo es **enchufable**: cada
//! variante de [`state::BgAnim`] es una función pura con la misma firma
//! (`paint(scene, ts, rect, t, color)`). Para sumar una animación nueva
//! —p. ej. un reproductor ASCII estilo *termflix*— basta un módulo con esa
//! firma y un brazo más acá y en [`state::BgAnim`].

use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

use crate::state::BgAnim;
use crate::{aurora, fire, lightning, plasma, rain, stars, waves};

/// Pinta un frame de la animación `anim` sobre `rect`. `t` es el reloj en
/// segundos; `bright` el color base ya resuelto (RGB del tema o de la paleta).
pub fn paint(
    anim: BgAnim,
    scene: &mut vello::Scene,
    ts: &mut Typesetter,
    rect: PaintRect,
    t: f32,
    bright: (u8, u8, u8),
) {
    match anim {
        BgAnim::Matrix => rain::paint(scene, ts, rect, t, bright),
        BgAnim::Stars => stars::paint(scene, ts, rect, t, bright),
        BgAnim::Waves => waves::paint(scene, ts, rect, t, bright),
        BgAnim::Fire => fire::paint(scene, ts, rect, t, bright),
        BgAnim::Plasma => plasma::paint(scene, ts, rect, t, bright),
        BgAnim::Aurora => aurora::paint(scene, ts, rect, t, bright),
        BgAnim::Lightning => lightning::paint(scene, ts, rect, t, bright),
    }
}
