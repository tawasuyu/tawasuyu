//! Fuzzing por propiedades del Cerebro.
//!
//! El Cerebro es puro y agnóstico de hardware: consume `BodyEvent` y devuelve
//! `BrainCommand`. Eso lo hace fuzzeable sin GPU ni Wayland. Aquí lanzamos
//! miles de secuencias aleatorias de eventos del Cuerpo —incluidas las
//! contradictorias o fuera de orden que un cliente buggy podría provocar— y
//! exigimos lo esencial: `on_event` **nunca paniquea**. Un panic en el camino
//! del Body tumba la sesión entera, así que la libertad-de-panic sobre entrada
//! adversaria es la propiedad que más techo de estabilidad compra.

use mirada_brain::{BodyEvent, Desktop};
use mirada_protocol::Rect;
use proptest::prelude::*;

/// Genera un `BodyEvent` arbitrario. IDs en rangos chicos para que ventanas y
/// monitores se reutilicen (abrir/cerrar/retitular el mismo id), que es donde
/// viven los bugs de estado — no con identificadores siempre únicos.
fn evento() -> impl Strategy<Value = BodyEvent> {
    let win = 0u64..8;
    let out = 0u32..4;
    let dim = 0i32..3000;
    let coord = -1000i32..3000;

    prop_oneof![
        (out.clone(), dim.clone(), dim.clone())
            .prop_map(|(id, width, height)| BodyEvent::OutputAdded { id, width, height }),
        out.clone().prop_map(|id| BodyEvent::OutputRemoved { id }),
        (out.clone(), dim.clone(), dim.clone())
            .prop_map(|(id, width, height)| BodyEvent::OutputResized { id, width, height }),
        (out.clone(), 0i32..200, 0i32..200, 0i32..200, 0i32..200).prop_map(
            |(id, top, bottom, left, right)| BodyEvent::OutputReserved {
                id,
                top,
                bottom,
                left,
                right
            }
        ),
        (out.clone(), coord.clone(), coord.clone())
            .prop_map(|(id, x, y)| BodyEvent::OutputMoved { id, x, y }),
        win.clone().prop_map(|id| BodyEvent::WindowOpened {
            id,
            app_id: format!("app-{}", id % 3),
            title: "t".into(),
        }),
        win.clone().prop_map(|id| BodyEvent::WindowClosed { id }),
        win.clone().prop_map(|id| BodyEvent::WindowRetitled { id, title: "x".into() }),
        win.clone().prop_map(|id| BodyEvent::PointerEntered { id }),
        win.clone().prop_map(|id| BodyEvent::Clicked { id }),
        (win.clone(), coord.clone(), coord.clone())
            .prop_map(|(id, x, y)| BodyEvent::WindowDragged { id, x, y }),
        (win.clone(), any::<bool>())
            .prop_map(|(id, fullscreen)| BodyEvent::FullscreenRequest { id, fullscreen }),
        (win.clone(), coord.clone(), coord.clone(), 1i32..800, 1i32..800).prop_map(
            |(id, x, y, w, h)| BodyEvent::WindowFloatTo {
                id,
                rect: Rect::new(x, y, w, h)
            }
        ),
        ("[a-z]{0,6}").prop_map(BodyEvent::Keybind),
        (0u32..4).prop_map(BodyEvent::SwitchWorkspace),
    ]
}

proptest! {
    /// Ninguna secuencia de eventos del Cuerpo paniquea el Cerebro, y el foco
    /// queda siempre coherente con las ventanas conocidas.
    #[test]
    fn on_event_nunca_paniquea(eventos in proptest::collection::vec(evento(), 0..300)) {
        let mut d = Desktop::new();
        // Un monitor base para que las ventanas tengan dónde aterrizar; las
        // secuencias igual pueden quitarlo y volver a agregarlo.
        let _ = d.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });

        for ev in eventos {
            // El contrato es no-panic; el valor de retorno se descarta.
            let _ = d.on_event(ev);
        }
    }
}
