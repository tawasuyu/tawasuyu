//! `llimphi-widget-hero` — shared-element transitions estilo Flutter Hero.
//!
//! El runtime de Llimphi tiene un [`HeroRegistry`](llimphi_ui::llimphi_compositor::HeroRegistry)
//! retenido entre frames: si la misma `key` aparece en un rect distinto entre
//! dos frames consecutivos, el runtime interpola `transform` para que el nodo
//! "vuele" del rect anterior al actual. Este widget es el envoltorio canónico
//! que marca al `child` como hero con la `key` indicada — la app no necesita
//! tocar `View::hero` a mano si compone con esto.
//!
//! Mantenemos las firmas previas (`hero_view`, `hero_quick`) para no romper
//! callers. Antes envolvían sólo con `animated_inout` (fade); ahora componen
//! `hero` + `animated_inout` para que un caller que reusa la misma `key` entre
//! rutas obtenga el fly real **y** el fade de aterrizaje juntos.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::View;
use llimphi_theme::motion;

/// Envuelve `child` como hero: si la misma `key` aparece en otro rect en un
/// frame siguiente, el runtime interpola `transform` para volar entre las dos
/// posiciones; el fade-in/out de `animated_inout` cubre el aterrizaje
/// (`motion::DRAMATIC`, 480 ms). `key` debe ser estable entre rebuilds.
pub fn hero_view<Msg: Clone + 'static>(key: u64, child: View<Msg>) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![child])
    .hero(key, motion::DRAMATIC)
    .animated_inout(key, motion::DRAMATIC)
}

/// Hero "rápido" — variante con `motion::SLOW` (320 ms). Para elementos
/// destacados pero menos protagónicos (un toast importante, un panel
/// que se monta).
pub fn hero_quick<Msg: Clone + 'static>(key: u64, child: View<Msg>) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![child])
    .hero(key, motion::SLOW)
    .animated_inout(key, motion::SLOW)
}
