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

/// Factor de alpha por defecto para el thumb en reposo — tenue moderno
/// estilo Chromium/Edge/Safari: visible pero discreto. Al hover sobre la
/// barra recupera alpha completo. `1.0` reproduce el comportamiento
/// histórico (thumb siempre opaco).
pub const DEFAULT_THUMB_IDLE_ALPHA: f32 = 0.55;

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
    /// Multiplicador de alpha aplicado al `thumb` en reposo. `1.0` deja
    /// el color sin tocar (comportamiento legacy); `≤ 0.0` esconde el
    /// thumb del todo en reposo. El `hover_fill` no se ve afectado: al
    /// pasar el cursor sobre la barra el thumb recupera el alpha completo
    /// del `thumb_hover`.
    pub thumb_idle_alpha: f32,
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
            thumb_idle_alpha: DEFAULT_THUMB_IDLE_ALPHA,
        }
    }

    /// Devuelve la `ScrollPalette` con el comportamiento histórico (thumb
    /// opaco en reposo, sin auto-hide visual). Para apps que dependen del
    /// look anterior al cambio del 2026-06-07.
    pub fn opaque(mut self) -> Self {
        self.thumb_idle_alpha = 1.0;
        self
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

/// Velocidad (px/s) por debajo de la cual una inercia se considera detenida.
pub const FLING_STOP: f32 = 8.0;
/// Fricción por defecto del fling: fracción de velocidad que **sobrevive por
/// segundo** (más chico = frena antes). 0.0015 ≈ deslizamiento tipo lista
/// táctil; subilo (p. ej. 0.1) para frenar rápido.
pub const FLING_FRICTION: f32 = 0.0015;

/// Un paso de **inercia** (fling): dado `velocity` en px/s y `dt` en segundos,
/// devuelve `(nueva_velocidad, delta_offset)` bajo decaimiento exponencial
/// `v(t) = v·friction^t`. `friction ∈ (0,1]` es la fracción de velocidad que
/// sobrevive por segundo. El `delta` es la integral exacta de la velocidad
/// sobre el paso (no el rectángulo `v·dt`), así el frenado no depende del
/// frame-rate. El caller suma `delta` al offset (clampeando con
/// [`clamp_offset`]) y reusa `nueva_velocidad` el próximo frame hasta que
/// [`fling_settled`] dé `true`. Es el análogo de [`approach`] pero para
/// "soltar con envión" en vez de "ir hacia un objetivo".
pub fn fling_step(velocity: f32, dt: f32, friction: f32) -> (f32, f32) {
    let f = friction.clamp(1e-6, 1.0);
    let decay = f.powf(dt.max(0.0));
    let new_v = velocity * decay;
    let delta = if (f - 1.0).abs() < 1e-6 {
        velocity * dt
    } else {
        // ∫₀^dt v·f^s ds = v·(f^dt − 1)/ln f.
        velocity * (decay - 1.0) / f.ln()
    };
    (new_v, delta)
}

/// ¿La inercia ya se detuvo? `true` cuando `|velocity| < FLING_STOP` — el
/// caller corta el ticker y deja el offset quieto.
pub fn fling_settled(velocity: f32) -> bool {
    velocity.abs() < FLING_STOP
}

/// Resistencia elástica (rubber-band) al **sobrepasar un borde**, estilo iOS:
/// dado cuánto se pasó del límite (`overscroll`, px; el signo se conserva) y la
/// dimensión del viewport (`dim`), devuelve el desplazamiento visual
/// **amortiguado** — siempre menor en magnitud que `overscroll`, con
/// rendimiento decreciente cuanto más se estira. El caller lo usa para pintar
/// el contenido un poco más allá del tope mientras arrastra, y lo libera
/// (anima a 0 con [`approach`]) al soltar. Constante 0.55 = la de Apple.
pub fn rubber_band(overscroll: f32, dim: f32) -> f32 {
    if dim <= 0.0 || overscroll == 0.0 {
        return overscroll;
    }
    const C: f32 = 0.55;
    let x = overscroll.abs();
    (1.0 - 1.0 / (x * C / dim + 1.0)) * dim * overscroll.signum()
}

// ── Slivers: app-bar colapsable + sticky headers (seam "extent-por-offset") ──

/// Altura de un **app-bar colapsable** dado el `offset` de scroll: arranca en
/// `header_max` (offset 0) y baja linealmente hasta `header_min`, donde queda
/// fijado (pinned). El "rango de colapso" es `header_max - header_min`.
pub fn collapsed_height(offset: f32, header_max: f32, header_min: f32) -> f32 {
    (header_max - offset.max(0.0)).clamp(header_min, header_max)
}

/// Fracción de colapso del app-bar en `[0, 1]`: `0` = expandido (offset 0),
/// `1` = colapsado al mínimo. El caller la usa para fundir el título, achicar
/// un subtítulo, bajar la opacidad de una imagen de fondo, etc.
pub fn collapse_fraction(offset: f32, header_max: f32, header_min: f32) -> f32 {
    let range = (header_max - header_min).max(0.0);
    if range <= 0.0 {
        return 1.0;
    }
    (offset.max(0.0) / range).clamp(0.0, 1.0)
}

/// Offset máximo de scroll con un app-bar colapsable: los `header_max -
/// header_min` px que consume el colapso **más** lo que scrollee el cuerpo
/// bajo el header ya fijado en `header_min`. El caller lo usa para clampear.
pub fn sliver_max_offset(
    content_len: f32,
    viewport_len: f32,
    header_max: f32,
    header_min: f32,
) -> f32 {
    let range = (header_max - header_min).max(0.0);
    let body_vp = (viewport_len - header_min).max(0.0);
    range + max_offset(content_len, body_vp)
}

/// Posición `y` (relativa al tope del viewport) de un encabezado **sticky** de
/// una sección que ocupa `[section_top, section_top + section_h]` en
/// coordenadas de contenido, con altura de encabezado `header_h`. Mientras la
/// sección está en pantalla, el encabezado se **pega al tope** (`y = 0`); al
/// llegar la próxima sección, ésta lo **empuja** hacia arriba (no pasa de
/// `section_bottom - header_h`). Antes de que la sección llegue al tope, sigue
/// su posición natural. El caller posiciona el encabezado absoluto en esta `y`.
pub fn sticky_y(offset: f32, section_top: f32, section_h: f32, header_h: f32) -> f32 {
    let natural = section_top - offset; // y del encabezado sin sticky
    let section_bottom = section_top + section_h - offset;
    natural.max(0.0).min(section_bottom - header_h)
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
        .fill(palette.thumb.multiply_alpha(palette.thumb_idle_alpha.clamp(0.0, 1.0)))
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
    //
    // **Scroll anidado**: si el delta del eje vertical es hacia un extremo
    // donde ya estamos topados (offset = 0 con dy<0, u offset = max con
    // dy>0), devolvemos `None` para que el runtime propague el evento al
    // ancestro scrollable más cercano (lista dentro de panel, etc.).
    let line_px = palette.line_px;
    let on_wheel = on_scroll;
    let max_off = max_offset(content_len, viewport_len);
    let at_top = offset <= 0.0;
    let at_bottom = offset >= max_off;
    View::new(Style {
        position: Position::Relative,
        size: Size {
            width: percent(1.0),
            height: length(viewport_len),
        },
        ..Default::default()
    })
    .clip(true)
    .on_scroll(move |_dx, dy| {
        let delta = dy * line_px;
        if (delta < 0.0 && at_top) || (delta > 0.0 && at_bottom) {
            return None;
        }
        Some((on_wheel)(delta))
    })
    .children(children)
}

/// Área de scroll **2D** (horizontal + vertical). Generaliza [`scroll_y`] a dos
/// ejes: el contenido toma su tamaño natural y se desplaza `(-x, -y)`, recortado
/// al viewport; aparece una barra por eje que tenga overflow (ninguna, una o
/// las dos). Para scroll puramente horizontal, pasá `content_size.1 ==
/// viewport_size.1` (no sale barra vertical).
///
/// `on_scroll(dx, dy)` recibe el **delta en px por eje** a sumar a cada offset
/// (rueda → ambos ejes; arrastre de la barra vertical → sólo `dy`; horizontal →
/// sólo `dx`). El caller acumula y clampea cada eje con [`clamp_offset`]. Las
/// dos barras se solapan en una esquinita inferior-derecha (v1; cosmético).
pub fn scroll_xy<Msg, F>(
    offset: (f32, f32),
    content_size: (f32, f32),
    viewport_size: (f32, f32),
    content: View<Msg>,
    on_scroll: F,
    palette: &ScrollPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(f32, f32) -> Msg + Send + Sync + 'static,
{
    let (ox, oy) = offset;
    let (cw, ch) = content_size;
    let (vw, vh) = viewport_size;
    let on_scroll = Arc::new(on_scroll);

    // Contenido a tamaño natural, desplazado (-x, -y). right/bottom = auto para
    // que no lo achique el viewport (a diferencia de scroll_y, que ancla
    // left/right para tomar el ancho del viewport).
    let content_wrap = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(-oy),
            left: length(-ox),
            right: auto(),
            bottom: auto(),
        },
        ..Default::default()
    })
    .children(vec![content]);

    let mut children = vec![content_wrap];

    // Barra vertical (borde derecho) — sólo si hay overflow vertical.
    if max_offset(ch, vh) > 0.0 {
        let (thumb_h, thumb_y, opp) = thumb_geometry(oy, ch, vh);
        let f = on_scroll.clone();
        let thumb = View::new(Style {
            position: Position::Absolute,
            inset: Rect { top: length(thumb_y), right: length(0.0), left: auto(), bottom: auto() },
            size: Size { width: length(palette.bar_width), height: length(thumb_h) },
            ..Default::default()
        })
        .fill(palette.thumb.multiply_alpha(palette.thumb_idle_alpha.clamp(0.0, 1.0)))
        .hover_fill(palette.thumb_hover)
        .radius((palette.bar_width * 0.5) as f64)
        .draggable(move |phase, _dx, dy| match phase {
            DragPhase::Move => Some((f)(0.0, dy * opp)),
            DragPhase::End => None,
        });
        let track = View::new(Style {
            position: Position::Absolute,
            inset: Rect { top: length(0.0), right: length(0.0), bottom: length(0.0), left: auto() },
            size: Size { width: length(palette.bar_width), height: auto() },
            ..Default::default()
        })
        .fill(palette.track)
        .children(vec![thumb]);
        children.push(track);
    }

    // Barra horizontal (borde inferior) — sólo si hay overflow horizontal.
    if max_offset(cw, vw) > 0.0 {
        let (thumb_w, thumb_x, opp) = thumb_geometry(ox, cw, vw);
        let f = on_scroll.clone();
        let thumb = View::new(Style {
            position: Position::Absolute,
            inset: Rect { left: length(thumb_x), bottom: length(0.0), top: auto(), right: auto() },
            size: Size { width: length(thumb_w), height: length(palette.bar_width) },
            ..Default::default()
        })
        .fill(palette.thumb.multiply_alpha(palette.thumb_idle_alpha.clamp(0.0, 1.0)))
        .hover_fill(palette.thumb_hover)
        .radius((palette.bar_width * 0.5) as f64)
        .draggable(move |phase, dx, _dy| match phase {
            DragPhase::Move => Some((f)(dx * opp, 0.0)),
            DragPhase::End => None,
        });
        let track = View::new(Style {
            position: Position::Absolute,
            inset: Rect { left: length(0.0), right: length(0.0), bottom: length(0.0), top: auto() },
            size: Size { width: auto(), height: length(palette.bar_width) },
            ..Default::default()
        })
        .fill(palette.track)
        .children(vec![thumb]);
        children.push(track);
    }

    // Scroll anidado 2D: si el delta NETO está bloqueado en ambos ejes
    // (cada componente cae en un extremo del eje correspondiente),
    // devolvemos `None` para propagar al ancestro scrollable. Si al menos
    // un eje aún tiene recorrido, el evento se consume entero (como antes).
    let line_px = palette.line_px;
    let on_wheel = on_scroll;
    let max_ox = max_offset(cw, vw);
    let max_oy = max_offset(ch, vh);
    let at_left = ox <= 0.0;
    let at_right = ox >= max_ox;
    let at_top = oy <= 0.0;
    let at_bottom = oy >= max_oy;
    View::new(Style {
        position: Position::Relative,
        size: Size { width: length(vw), height: length(vh) },
        ..Default::default()
    })
    .clip(true)
    // Rueda: dy = eje vertical; dx = eje horizontal (ratones/touchpads 2D, o
    // Shift+rueda en algunos backends). Ambos en px-línea.
    .on_scroll(move |dx, dy| {
        let ddx = dx * line_px;
        let ddy = dy * line_px;
        let x_blocked = (ddx < 0.0 && at_left)
            || (ddx > 0.0 && at_right)
            || ddx == 0.0;
        let y_blocked = (ddy < 0.0 && at_top)
            || (ddy > 0.0 && at_bottom)
            || ddy == 0.0;
        if x_blocked && y_blocked {
            return None;
        }
        Some((on_wheel)(ddx, ddy))
    })
    .children(children)
}

/// **App-bar colapsable + cuerpo scrolleable** en un solo viewport (el sliver
/// más pedido). Un único `offset` (en el Model) maneja las dos cosas: primero
/// **colapsa** el header de `header_max` a `header_min` (consume los primeros
/// `header_max - header_min` px de scroll), y luego **scrollea** el cuerpo bajo
/// el header ya fijado en `header_min`.
///
/// `header(frac)` construye el contenido del header dado `frac ∈ [0,1]` (ver
/// [`collapse_fraction`]) — el caller lo usa para fundir el título, mostrar una
/// versión compacta al colapsar, etc. El header se pinta a la altura
/// [`collapsed_height`] del momento.
///
/// `content_len` es el alto natural del cuerpo; el viewport del cuerpo cambia
/// con el colapso (crece a medida que el header se achica). La rueda funciona
/// tanto sobre el header como sobre el cuerpo (ambos emiten `on_scroll`). El
/// caller clampea el offset con [`sliver_max_offset`].
pub fn sliver_app_bar<Msg, H, F>(
    offset: f32,
    header_max: f32,
    header_min: f32,
    header: H,
    content: View<Msg>,
    content_len: f32,
    viewport_len: f32,
    on_scroll: F,
    palette: &ScrollPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
    H: FnOnce(f32) -> View<Msg>,
    F: Fn(f32) -> Msg + Send + Sync + 'static,
{
    let range = (header_max - header_min).max(0.0);
    let h = collapsed_height(offset, header_max, header_min);
    let frac = collapse_fraction(offset, header_max, header_min);
    // El cuerpo recién empieza a scrollear cuando el colapso terminó.
    let body_offset = (offset - range).max(0.0);
    let body_vp = (viewport_len - h).max(0.0);

    let on_scroll = Arc::new(on_scroll);
    let line_px = palette.line_px;

    // Header pinned (altura `h`), recortado, con rueda propia.
    let s_head = on_scroll.clone();
    let header_box = View::new(Style {
        size: Size { width: percent(1.0), height: length(h) },
        ..Default::default()
    })
    .clip(true)
    .on_scroll(move |_dx, dy| Some((s_head)(dy * line_px)))
    .children(vec![header(frac)]);

    // Cuerpo: reusa scroll_y con el viewport restante y el offset del cuerpo.
    let s_body = on_scroll;
    let body = scroll_y(
        body_offset,
        content_len,
        body_vp,
        content,
        move |d| (s_body)(d),
        palette,
    );

    View::new(Style {
        flex_direction:
            llimphi_ui::llimphi_layout::taffy::prelude::FlexDirection::Column,
        size: Size { width: percent(1.0), height: length(viewport_len) },
        ..Default::default()
    })
    .clip(true)
    .children(vec![header_box, body])
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
    fn fling_decae_y_se_detiene() {
        // Con fricción <1, la velocidad decae cada paso y el delta tiene el
        // signo de la velocidad.
        let (v1, d1) = fling_step(1000.0, 0.016, FLING_FRICTION);
        assert!(v1 < 1000.0 && v1 > 0.0);
        assert!(d1 > 0.0 && d1 < 1000.0 * 0.016 + 0.01); // < rectángulo v·dt
        // Tras muchos pasos de 16 ms, termina por debajo del umbral.
        let mut v = 1200.0_f32;
        let mut steps = 0;
        while !fling_settled(v) && steps < 100_000 {
            v = fling_step(v, 0.016, FLING_FRICTION).0;
            steps += 1;
        }
        assert!(fling_settled(v));
        // Velocidad negativa → delta negativo (scrollea al revés).
        let (_, dneg) = fling_step(-500.0, 0.016, FLING_FRICTION);
        assert!(dneg < 0.0);
        // friction = 1.0 (sin fricción) → delta = v·dt exacto.
        let (v2, d2) = fling_step(300.0, 0.02, 1.0);
        assert!((v2 - 300.0).abs() < 1e-3);
        assert!((d2 - 6.0).abs() < 1e-3);
    }

    #[test]
    fn rubber_band_amortigua() {
        let dim = 600.0;
        // Siempre menor en magnitud que el overscroll crudo.
        assert!(rubber_band(100.0, dim) < 100.0);
        assert!(rubber_band(100.0, dim) > 0.0);
        // Conserva el signo.
        assert!(rubber_band(-80.0, dim) < 0.0);
        // Rendimiento decreciente: estirar 2× no duplica el desplazamiento.
        let a = rubber_band(100.0, dim);
        let b = rubber_band(200.0, dim);
        assert!(b > a && b < 2.0 * a);
        // Cerca de 0 es casi lineal (poca amortiguación todavía).
        assert!(rubber_band(0.0, dim).abs() < 1e-6);
    }

    #[test]
    fn sliver_colapso_y_max() {
        // Header 200→64, viewport 500, contenido 1200.
        let (max_h, min_h) = (200.0, 64.0);
        // Offset 0 → expandido, frac 0.
        assert_eq!(collapsed_height(0.0, max_h, min_h), 200.0);
        assert_eq!(collapse_fraction(0.0, max_h, min_h), 0.0);
        // A mitad del rango (68px de 136) → ~0.5 y altura ~132.
        let mid = (max_h - min_h) / 2.0; // 68
        assert!((collapse_fraction(mid, max_h, min_h) - 0.5).abs() < 1e-3);
        assert!((collapsed_height(mid, max_h, min_h) - 132.0).abs() < 1e-3);
        // Pasado el rango → fijado al mínimo, frac 1.
        assert_eq!(collapsed_height(500.0, max_h, min_h), 64.0);
        assert_eq!(collapse_fraction(500.0, max_h, min_h), 1.0);
        // Max offset = rango (136) + scroll del cuerpo bajo el header mínimo.
        let body_vp = 500.0 - min_h; // 436
        let expected = 136.0 + max_offset(1200.0, body_vp);
        assert!((sliver_max_offset(1200.0, 500.0, max_h, min_h) - expected).abs() < 1e-3);
    }

    #[test]
    fn sticky_pegado_y_empujado() {
        // Sección [100, 100+300], encabezado 40px de alto.
        let (top, sh, hh) = (100.0, 300.0, 40.0);
        // Antes de llegar al tope (offset 50 < 100): posición natural 50.
        assert_eq!(sticky_y(50.0, top, sh, hh), 50.0);
        // Dentro de la sección (offset 200 > top): pegado al tope (0).
        assert_eq!(sticky_y(200.0, top, sh, hh), 0.0);
        // Cerca del fondo de la sección: la próxima lo empuja hacia arriba (<0).
        // section_bottom - hh = (100+300-380) - 40 = -20.
        assert_eq!(sticky_y(380.0, top, sh, hh), -20.0);
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
