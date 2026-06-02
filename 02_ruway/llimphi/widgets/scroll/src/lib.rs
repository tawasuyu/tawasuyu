//! `llimphi-widget-scroll` — área de scroll vertical reutilizable.
//!
//! Hasta ahora cada app rearmaba el scroll a mano: `App::on_wheel` + un
//! offset en el `Model` + `clip` + virtualización. Este widget empaqueta
//! ese patrón en un solo builder, **sin estado propio** (el offset sigue
//! viviendo en el `Model`, fiel al bucle Elm):
//!
//! - **viewport clipeado** de alto fijo (`viewport_len`),
//! - **contenido desplazado** `-offset` px (overflow recortado),
//! - **barra de scroll arrastrable** a la derecha (sólo si el contenido
//!   excede el viewport),
//! - **rueda autocontenida** vía [`View::on_scroll`]: girar la rueda con
//!   el cursor sobre el área emite un `Msg` sin que la app rutee nada por
//!   su `on_wheel` global.
//!
//! El caller debe conocer el **alto total del contenido** (`content_len`)
//! y el **alto visible** (`viewport_len`) — igual que `list`/`grid` ya
//! piden la ventana visible. Para contenido de filas uniformes es
//! `n_filas * alto_fila`.
//!
//! ## Convención del callback `on_scroll`
//!
//! `on_scroll` recibe el **delta en px** a sumar al offset (no el offset
//! absoluto): tanto la rueda como el arrastre de la barra emiten deltas,
//! y el caller acumula + clampea en su `update` con [`clamp_offset`]. Es
//! la misma idea que el `splitter` (el handler de drag se reusa durante
//! todo el arrastre, así que un offset absoluto capturado se quedaría
//! viejo; el delta-por-evento siempre es correcto).
//!
//! ```ignore
//! // view:
//! scroll_y(
//!     model.offset,
//!     model.rows.len() as f32 * ROW_H,
//!     panel_h,
//!     lista_view,
//!     Msg::ScrollBy,            // Fn(f32) -> Msg, arg = delta px
//!     &ScrollPalette::default(),
//! )
//! // update:
//! Msg::ScrollBy(d) => {
//!     m.offset = clamp_offset(m.offset + d, content_len, viewport_len);
//! }
//! ```
//!
//! Para llevar una selección a la vista (teclado), ver [`ensure_visible`];
//! para scroll suave/inercia, ver [`approach`].

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, Position, Rect, Size, Style},
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{DragPhase, View};

/// Alto mínimo del thumb en px — para que no desaparezca con contenido
/// muy largo.
const MIN_THUMB: f32 = 28.0;
/// Px de desplazamiento por "línea" de rueda. Aproxima el step de scroll
/// de un editor (≈3 líneas de texto).
pub const DEFAULT_LINE_PX: f32 = 48.0;
/// Ancho de la barra de scroll en px.
pub const DEFAULT_BAR_WIDTH: f32 = 10.0;

/// Colores de la barra de scroll.
#[derive(Debug, Clone, Copy)]
pub struct ScrollPalette {
    /// Canal de fondo (track).
    pub track: Color,
    /// Pulgar (thumb) en reposo.
    pub thumb: Color,
    /// Pulgar al pasar el cursor.
    pub thumb_hover: Color,
    /// Ancho de la barra y px por línea de rueda.
    pub bar_width: f32,
    pub line_px: f32,
}

impl Default for ScrollPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl ScrollPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            track: t.bg_panel_alt,
            thumb: t.border,
            thumb_hover: t.accent,
            bar_width: DEFAULT_BAR_WIDTH,
            line_px: DEFAULT_LINE_PX,
        }
    }
}

/// Máximo offset posible: cuánto se puede desplazar antes de que el final
/// del contenido toque el borde inferior del viewport. `0` si el contenido
/// entra entero.
pub fn max_offset(content_len: f32, viewport_len: f32) -> f32 {
    (content_len - viewport_len).max(0.0)
}

/// Acota `offset` a `[0, max_offset]`. El caller lo usa en su `update`
/// tras sumar el delta de [`scroll_y`].
pub fn clamp_offset(offset: f32, content_len: f32, viewport_len: f32) -> f32 {
    offset.clamp(0.0, max_offset(content_len, viewport_len))
}

/// Devuelve el offset que deja **visible** el intervalo vertical
/// `[item_top, item_top + item_h]` dentro de un viewport de alto
/// `viewport_len`, partiendo de `offset`. Si ya está visible, lo devuelve
/// sin cambios. Pensado para "llevar la selección a la vista" al navegar
/// con teclado (flechas, Page Up/Down). El resultado se acota a `≥ 0`; el
/// caller puede clampear arriba con [`clamp_offset`] si lo necesita.
pub fn ensure_visible(offset: f32, viewport_len: f32, item_top: f32, item_h: f32) -> f32 {
    if item_top < offset {
        // El item arranca por encima del viewport: subí hasta su tope.
        item_top.max(0.0)
    } else if item_top + item_h > offset + viewport_len {
        // El item termina por debajo: bajá hasta que su fondo toque el borde.
        (item_top + item_h - viewport_len).max(0.0)
    } else {
        offset
    }
}

/// Un paso de aproximación exponencial de `current` hacia `target`
/// (scroll suave / inercia). `factor ∈ (0, 1]`: 1.0 salta de una, 0.2
/// desliza suave. Cuando la diferencia cae por debajo de 0.5 px aterriza
/// exacto en `target` (evita el "casi-llega" infinito). El caller lo
/// dispara por frame vía `Handle::spawn_periodic` guardando `target` en
/// su `Model`.
pub fn approach(current: f32, target: f32, factor: f32) -> f32 {
    let f = factor.clamp(0.0, 1.0);
    let next = current + (target - current) * f;
    if (target - next).abs() < 0.5 {
        target
    } else {
        next
    }
}

/// Geometría del thumb: `(altura, posición_y)` dentro del track de alto
/// `viewport_len`, y `offset_por_px` (cuánto offset de contenido equivale
/// a 1 px de arrastre del thumb). Público para tests y para callers que
/// quieran pintar su propia barra.
pub fn thumb_geometry(offset: f32, content_len: f32, viewport_len: f32) -> (f32, f32, f32) {
    let max_off = max_offset(content_len, viewport_len);
    if max_off <= 0.0 || content_len <= 0.0 {
        return (viewport_len, 0.0, 0.0);
    }
    let ratio = (viewport_len / content_len).clamp(0.0, 1.0);
    let thumb_h = (viewport_len * ratio).clamp(MIN_THUMB.min(viewport_len), viewport_len);
    let travel = (viewport_len - thumb_h).max(0.0);
    let thumb_y = if max_off > 0.0 {
        (offset / max_off).clamp(0.0, 1.0) * travel
    } else {
        0.0
    };
    let offset_per_px = if travel > 0.0 { max_off / travel } else { 0.0 };
    (thumb_h, thumb_y, offset_per_px)
}

/// Área de scroll vertical. `offset` es el desplazamiento actual (px, ya
/// clampeado por el caller). `content_len`/`viewport_len` el alto total y
/// visible. `content` se desplaza `-offset` y se recorta al viewport.
/// `on_scroll(delta_px)` se invoca con el delta a sumar al offset (rueda
/// y arrastre de barra); el caller acumula con [`clamp_offset`].
pub fn scroll_y<Msg, F>(
    offset: f32,
    content_len: f32,
    viewport_len: f32,
    content: View<Msg>,
    on_scroll: F,
    palette: &ScrollPalette,
) -> View<Msg>
where
    // `Msg` no necesita `Send + Sync`: los closures de rueda/arrastre
    // capturan el `Arc<dyn Fn + Send + Sync>`, no un `Msg`. Sólo se exige
    // `Clone` (para montar el `View`) y `'static`.
    Msg: Clone + 'static,
    F: Fn(f32) -> Msg + Send + Sync + 'static,
{
    let on_scroll = Arc::new(on_scroll);

    // Contenido desplazado: nodo absoluto anclado a left/right (toma el
    // ancho del viewport) con top = -offset y alto natural. El overflow se
    // recorta por el `clip` del viewport.
    let content_wrap = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(-offset),
            left: length(0.0),
            right: length(0.0),
            bottom: auto(),
        },
        ..Default::default()
    })
    .children(vec![content]);

    let mut children = vec![content_wrap];

    // Barra: sólo si hay overflow.
    if max_offset(content_len, viewport_len) > 0.0 {
        let (thumb_h, thumb_y, offset_per_px) =
            thumb_geometry(offset, content_len, viewport_len);

        let on_thumb = on_scroll.clone();
        let thumb = View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                top: length(thumb_y),
                right: length(0.0),
                left: auto(),
                bottom: auto(),
            },
            size: Size {
                width: length(palette.bar_width),
                height: length(thumb_h),
            },
            ..Default::default()
        })
        .fill(palette.thumb)
        .hover_fill(palette.thumb_hover)
        .radius((palette.bar_width * 0.5) as f64)
        .draggable(move |phase, _dx, dy| match phase {
            // Cada Move trae el delta de px de pantalla del thumb; lo
            // convertimos a delta de offset de contenido.
            DragPhase::Move => Some((on_thumb)(dy * offset_per_px)),
            DragPhase::End => None,
        });

        let track = View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                top: length(0.0),
                right: length(0.0),
                bottom: length(0.0),
                left: auto(),
            },
            size: Size {
                width: length(palette.bar_width),
                height: auto(),
            },
            ..Default::default()
        })
        .fill(palette.track)
        .children(vec![thumb]);

        children.push(track);
    }

    // Viewport: alto fijo, ancho del padre, contenido recortado, rueda
    // local. Position::Relative para ser el bloque contenedor de los
    // hijos absolutos.
    let line_px = palette.line_px;
    let on_wheel = on_scroll;
    View::new(Style {
        position: Position::Relative,
        size: Size {
            width: percent(1.0),
            height: length(viewport_len),
        },
        ..Default::default()
    })
    .clip(true)
    .on_scroll(move |_dx, dy| Some((on_wheel)(dy * line_px)))
    .children(children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_y_clamp() {
        assert_eq!(max_offset(1000.0, 300.0), 700.0);
        assert_eq!(max_offset(200.0, 300.0), 0.0); // entra entero
        assert_eq!(clamp_offset(-50.0, 1000.0, 300.0), 0.0);
        assert_eq!(clamp_offset(9999.0, 1000.0, 300.0), 700.0);
        assert_eq!(clamp_offset(400.0, 1000.0, 300.0), 400.0);
    }

    #[test]
    fn ensure_visible_arriba_abajo_y_sin_cambio() {
        let vp = 300.0;
        // Item por encima del offset → subir hasta su tope.
        assert_eq!(ensure_visible(500.0, vp, 100.0, 20.0), 100.0);
        // Item por debajo del fondo visible → bajar lo justo.
        assert_eq!(ensure_visible(0.0, vp, 400.0, 20.0), 120.0); // 400+20-300
        // Item ya visible → sin cambios.
        assert_eq!(ensure_visible(50.0, vp, 100.0, 20.0), 50.0);
        // Nunca negativo.
        assert_eq!(ensure_visible(50.0, vp, -10.0, 20.0), 0.0);
    }

    #[test]
    fn approach_aterriza_exacto() {
        // Se acerca pero no salta.
        let a = approach(0.0, 100.0, 0.25);
        assert!(a > 0.0 && a < 100.0);
        // Diferencia < 0.5 px → aterriza exacto.
        assert_eq!(approach(99.8, 100.0, 0.25), 100.0);
        // factor 1.0 salta de una.
        assert_eq!(approach(0.0, 100.0, 1.0), 100.0);
    }

    #[test]
    fn thumb_proporcional_y_topes() {
        // Contenido entra entero → thumb cubre todo, sin travel.
        let (h, y, opp) = thumb_geometry(0.0, 200.0, 300.0);
        assert_eq!((h, y, opp), (300.0, 0.0, 0.0));
        // Contenido 3× viewport → thumb ≈ 1/3 (clampeado a MIN_THUMB).
        let (h, y, _) = thumb_geometry(0.0, 900.0, 300.0);
        assert!((h - 100.0).abs() < 0.01);
        assert_eq!(y, 0.0);
        // En el máximo offset, el thumb toca el fondo del track.
        let max = max_offset(900.0, 300.0);
        let (h2, y2, _) = thumb_geometry(max, 900.0, 300.0);
        assert!((y2 + h2 - 300.0).abs() < 0.01);
    }
}
