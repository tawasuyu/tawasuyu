//! `llimphi-widget-hero` — entrada con firma "hero".
//!
//! En Flutter, `Hero` anima un elemento compartido entre rutas (la
//! misma key en la página origen y destino → el runtime interpola
//! rect+contenido). Implementar eso fielmente requiere un `HeroRegistry`
//! retenido entre frames con captura de rect anterior + transform
//! interpolado — territorio de runtime, no de widget.
//!
//! Lo que sí podemos dar como widget hoy: la **firma cinética de
//! aterrizaje** que Flutter Hero deja al final del trayecto — un fade-in
//! suave con cuerpo (DRAMATIC). Compone `View::animated_enter` con la
//! duración correcta y el wrapping mínimo. Para hacer dos elementos
//! compartidos, el caller usa la misma `key` y deja que `animated_enter`/
//! `animated_exit` los enlace en tiempo (el "fly" real no se anima, pero
//! la sensación de "aparece como protagonista" sí).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::View;
use llimphi_theme::motion;

/// Envuelve `child` con la firma hero: anim de entrada+salida en
/// `motion::DRAMATIC` (480 ms). `key` debe ser estable entre frames.
pub fn hero_view<Msg: Clone + 'static>(key: u64, child: View<Msg>) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![child])
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
    .animated_inout(key, motion::SLOW)
}
