//! `llimphi-widget-fitted-box` — escala un subárbol al slot disponible.
//!
//! Análogo a `FittedBox` de Flutter: el caller le pasa el **tamaño
//! natural** del contenido y una **política de fit**, y el widget aplica
//! un `transform` afín al subárbol para que entre en el slot real
//! (medido por el seam [`LayoutBuilder`] del compositor). El subárbol
//! queda **centrado** y, salvo `BoxFit::Fill`, **preserva su aspect
//! ratio**.
//!
//! Por qué no sale solo: `taffy` dimensiona contenedores, pero el
//! contenido (texto, imagen, painter custom) no escala con su contenedor
//! — sólo se posiciona dentro. `FittedBox` aplica un escalado VISUAL
//! sobre el subárbol completo, así un canvas `paint_with` o un texto
//! grande caben en una celda chiquita sin que el caller los re-mida.
//!
//! El padre del widget tiene `clip(true)` para que ningún píxel se salga
//! del slot (sólo importa cuando el aspect del contenido NO coincide con
//! el slot bajo `BoxFit::None`, o nunca con `Contain`/`Fill`).
//!
//! ## API
//!
//! ```ignore
//! use llimphi_widget_fitted_box::{fitted_box, BoxFit};
//! // Una imagen 800×600 que tiene que caber en cualquier slot, preservando
//! // aspect (mostrar entera, posibles bandas).
//! fitted_box((800.0, 600.0), BoxFit::Contain, || my_image_view())
//! ```
//!
//! ## Funciones puras
//!
//! [`compute_fit`] devuelve `(sx, sy, dx, dy)` para un `(slot, inner,
//! fit)` dado y es testeable sin runtime. Útil para validar el algoritmo
//! y para casos donde el caller ya tiene una transformación propia que
//! quiere componer.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Size, Style},
};
use llimphi_ui::llimphi_raster::kurbo::Affine;
use llimphi_ui::View;

/// Política de encaje del contenido en el slot. Cubre los cinco modos
/// canónicos (Flutter `BoxFit`, CSS `object-fit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxFit {
    /// Preserva aspect — el contenido cabe ENTERO en el slot, dejando
    /// bandas si el aspect no coincide. Lo usual para mostrar imágenes
    /// sin recortar.
    Contain,
    /// Preserva aspect — el contenido CUBRE todo el slot, recortando lo
    /// que sobre. Lo usual para fondos.
    Cover,
    /// Estira para llenar el slot (puede deformar — no preserva aspect).
    Fill,
    /// No escala — el contenido se muestra a tamaño natural, centrado.
    /// Si es más grande que el slot, se recorta por el `clip` del padre.
    None,
    /// Como `Contain` pero **nunca agranda** — sólo achica si el
    /// contenido es más grande que el slot. Equivale a
    /// `min(Contain, None)`. Útil para evitar pixelar imágenes chicas.
    ScaleDown,
}

/// `(sx, sy, dx, dy)` para encajar un contenido de tamaño `inner` en un
/// slot `slot` bajo la política `fit`. El factor `(sx, sy)` se aplica
/// como escala (igual en x e y salvo en `Fill`), y `(dx, dy)` es el
/// offset desde la esquina superior-izquierda del slot al borde
/// superior-izquierdo del contenido **ya escalado**, que queda centrado.
///
/// Casos de borde:
/// - `inner.0 <= 0.0 || inner.1 <= 0.0` o `slot.0 <= 0.0 || slot.1 <=
///   0.0` ⇒ `(1.0, 1.0, 0.0, 0.0)` (identidad — defensa, no panic).
pub fn compute_fit(slot: (f32, f32), inner: (f32, f32), fit: BoxFit) -> (f32, f32, f32, f32) {
    let (sw, sh) = slot;
    let (iw, ih) = inner;
    if iw <= 0.0 || ih <= 0.0 || sw <= 0.0 || sh <= 0.0 {
        return (1.0, 1.0, 0.0, 0.0);
    }
    let (sx, sy) = match fit {
        BoxFit::Contain => {
            let s = (sw / iw).min(sh / ih);
            (s, s)
        }
        BoxFit::Cover => {
            let s = (sw / iw).max(sh / ih);
            (s, s)
        }
        BoxFit::Fill => (sw / iw, sh / ih),
        BoxFit::None => (1.0, 1.0),
        BoxFit::ScaleDown => {
            let s = (sw / iw).min(sh / ih).min(1.0);
            (s, s)
        }
    };
    let scaled_w = iw * sx;
    let scaled_h = ih * sy;
    let dx = (sw - scaled_w) * 0.5;
    let dy = (sh - scaled_h) * 0.5;
    (sx, sy, dx, dy)
}

/// Vista que escala el subárbol `inner` (tamaño natural `inner_size`) al
/// slot del padre bajo la política `fit`. `inner` es una closure porque
/// `View<Msg>` no es `Clone` — el seam `LayoutBuilder` puede invocar el
/// builder más de una vez en su resolución de dos pasadas.
///
/// El nodo retornado toma `width: 100%` y `height: 100%` por defecto —
/// el caller decide el tamaño envolviéndolo en un padre con `Style`
/// explícito.
pub fn fitted_box<Msg, F>(inner_size: (f32, f32), fit: BoxFit, inner: F) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn() -> View<Msg> + Send + Sync + 'static,
{
    let (iw, ih) = inner_size;
    View::<Msg>::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .clip(true)
    .layout_builder(move |c| {
        let slot = (c.max_width, c.max_height);
        let (sx, sy, dx, dy) = compute_fit(slot, (iw, ih), fit);
        // El runtime aplica el transform centrado en el nodo (convención
        // CSS `transform-origin: 50% 50%`). Un nodo de tamaño natural
        // (iw, ih) centrado en (iw/2, ih/2), tras `scale_non_uniform(sx,
        // sy)` queda con tamaño visual (iw*sx, ih*sy) pero todavía
        // centrado en (iw/2, ih/2). Para correrlo al centro del slot
        // (sw/2, sh/2) trasladamos por la diferencia de centros, que en
        // el caso de un nodo posicionado en (0,0) es exactamente `(dx,
        // dy) + (scaled_w-iw)/2 + (scaled_h-ih)/2`. Después de hacer la
        // cuenta: `delta = (sw - iw)/2`. (`dx + (scaled-iw)/2 =
        // (sw-iw)/2`.) Así el offset que pasamos es `((sw-iw)/2,
        // (sh-ih)/2)` antes del scale-around-center.
        let delta_x = (c.max_width - iw) * 0.5;
        let delta_y = (c.max_height - ih) * 0.5;
        let xf = Affine::translate((delta_x as f64, delta_y as f64))
            * Affine::scale_non_uniform(sx as f64, sy as f64);
        // Suprimir warnings sobre dx/dy no usados — están en la doc + tests.
        let _ = (dx, dy);

        let inner_node = (inner)();
        let inner_wrap = View::<Msg>::new(Style {
            position: llimphi_ui::llimphi_layout::taffy::Position::Absolute,
            inset: llimphi_ui::llimphi_layout::taffy::Rect {
                top: length(0.0_f32),
                left: length(0.0_f32),
                right: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
                bottom: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
            },
            size: Size { width: length(iw), height: length(ih) },
            ..Default::default()
        })
        .children(vec![inner_node])
        .transform(xf);

        View::<Msg>::new(Style {
            position: llimphi_ui::llimphi_layout::taffy::Position::Relative,
            size: Size {
                width: length(c.max_width),
                height: length(c.max_height),
            },
            ..Default::default()
        })
        .children(vec![inner_wrap])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contain_preserva_aspect_y_deja_bandas() {
        // Slot 200×100, contenido 200×200 (cuadrado) — Contain achica al
        // mínimo eje (100/200 = 0.5).
        let (sx, sy, dx, dy) = compute_fit((200.0, 100.0), (200.0, 200.0), BoxFit::Contain);
        assert!((sx - 0.5).abs() < 1e-3);
        assert!((sy - 0.5).abs() < 1e-3);
        // Contenido escalado 100×100 centrado en 200×100 → dx=50, dy=0.
        assert!((dx - 50.0).abs() < 1e-3);
        assert!(dy.abs() < 1e-3);
    }

    #[test]
    fn cover_preserva_aspect_y_recorta() {
        // Slot 200×200, contenido 200×100 (paisaje) — Cover toma el MÁXIMO
        // eje (200/100 = 2.0 en y), el x sobra.
        let (sx, sy, dx, dy) = compute_fit((200.0, 200.0), (200.0, 100.0), BoxFit::Cover);
        assert!((sx - 2.0).abs() < 1e-3);
        assert!((sy - 2.0).abs() < 1e-3);
        // Contenido escalado 400×200 centrado en 200×200 → dx=-100, dy=0.
        assert!((dx - (-100.0)).abs() < 1e-3);
        assert!(dy.abs() < 1e-3);
    }

    #[test]
    fn fill_estira_no_preserva_aspect() {
        let (sx, sy, dx, dy) = compute_fit((200.0, 100.0), (100.0, 200.0), BoxFit::Fill);
        assert!((sx - 2.0).abs() < 1e-3);
        assert!((sy - 0.5).abs() < 1e-3);
        // Sin offset — el contenido escalado cubre todo el slot.
        assert!(dx.abs() < 1e-3);
        assert!(dy.abs() < 1e-3);
    }

    #[test]
    fn none_mantiene_natural_y_centra() {
        // Slot 200×200, contenido 80×60 → sin escalar, centrado.
        let (sx, sy, dx, dy) = compute_fit((200.0, 200.0), (80.0, 60.0), BoxFit::None);
        assert!((sx - 1.0).abs() < 1e-6);
        assert!((sy - 1.0).abs() < 1e-6);
        assert!((dx - 60.0).abs() < 1e-3); // (200-80)/2
        assert!((dy - 70.0).abs() < 1e-3); // (200-60)/2
    }

    #[test]
    fn scale_down_no_agranda_solo_achica() {
        // Contenido chico (40×30) en slot grande (200×100) → no agranda,
        // queda 1.0 y centrado.
        let (sx, sy, _, _) = compute_fit((200.0, 100.0), (40.0, 30.0), BoxFit::ScaleDown);
        assert!((sx - 1.0).abs() < 1e-6);
        assert!((sy - 1.0).abs() < 1e-6);
        // Contenido grande (400×400) en slot chico (100×100) → achica como
        // Contain (100/400 = 0.25).
        let (sx2, sy2, _, _) =
            compute_fit((100.0, 100.0), (400.0, 400.0), BoxFit::ScaleDown);
        assert!((sx2 - 0.25).abs() < 1e-3);
        assert!((sy2 - 0.25).abs() < 1e-3);
    }

    #[test]
    fn entradas_invalidas_devuelven_identidad() {
        // Cualquier dimensión ≤ 0 ⇒ identidad sin offset (defensa).
        assert_eq!(compute_fit((0.0, 100.0), (50.0, 50.0), BoxFit::Contain), (1.0, 1.0, 0.0, 0.0));
        assert_eq!(compute_fit((100.0, 100.0), (0.0, 50.0), BoxFit::Cover), (1.0, 1.0, 0.0, 0.0));
        assert_eq!(compute_fit((100.0, 100.0), (50.0, -1.0), BoxFit::Fill), (1.0, 1.0, 0.0, 0.0));
    }
}
