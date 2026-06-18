//! Render walk: traducción del `BoxTree` (CSS computado por puriy-engine) a la
//! jerarquía de `View<Msg>` de Llimphi. Incluye `viewport` (entrada desde la
//! UI), `render_box` y sus especializaciones (links, inputs, checkbox/radio,
//! submit, select, svg, canvas), `box_style` (BoxNode→taffy Style), las
//! decoraciones (bordes/sombras/fondos), los mappers CSS→taffy y el armado del
//! gradiente lineal. Extraído de `lib.rs` (regla #1). Comparte todos los tipos
//! del crate vía `use super::*`.
use super::*;

pub(crate) mod widgets;
pub(crate) mod image;
pub(crate) mod decorations;
pub(crate) mod style;

pub(crate) use widgets::*;
pub(crate) use image::*;
pub(crate) use decorations::*;
pub(crate) use style::*;

/// Estado por-frame que el render walk hila por toda la jerarquía. Lo
/// agrupamos en un struct para que `render_box`/`render_link_subtree`
/// no tengan 10 params; los `*_counter` se mutan por referencia.
pub(crate) struct RenderCtx<'a> {
    zoom: f32,
    matcher: &'a Matcher,
    find_current: usize,
    find_counter: usize,
    details_open: &'a [bool],
    details_counter: usize,
    inputs: &'a [TextInputState],
    input_checks: &'a [bool],
    focused_input: Option<usize>,
    input_counter: usize,
    selects: &'a [SelectState],
    select_counter: usize,
    /// Tiempo transcurrido (ms) desde el `anim_start_ms` de la pestaña —
    /// `animation_overlay` lo usa para samplear el progreso de cada nodo
    /// animado al instante actual.
    anim_elapsed_ms: u64,
    /// Reloj absoluto (ms desde `Model.start`) del frame actual — el tween
    /// de `transition` en hover lo usa para samplear cada `HoverTween`.
    now_ms: u64,
    /// Tweens de transición en hover por `node_id` (estado de la pestaña).
    hover_tweens: &'a std::collections::HashMap<u32, HoverTween>,
    /// Frames de `<canvas>` 2D keyeados por `element_id` — `render_canvas`
    /// los busca por el id del box canvas. Fase 7.196.
    canvas_frames: &'a std::collections::HashMap<String, CanvasFrame>,
    /// Imágenes decodificadas para `drawImage`, keyeadas por `src`. Fase 7.197b.
    canvas_images: &'a std::collections::HashMap<String, Option<PenikoImage>>,
}

pub(crate) fn viewport(
    t: &TabState,
    zoom: f32,
    matcher: &Matcher,
    find_current: usize,
    anim_elapsed_ms: u64,
    now_ms: u64,
) -> View<Msg> {
    let Some(tree) = t.box_tree.as_ref() else {
        let msg = if t.url == NEW_TAB_URL {
            "(pestaña vacía · escribí una URL arriba)"
        } else {
            "(cargando…)"
        };
        return View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            padding: Rect {
                left: length(24.0_f32),
                right: length(24.0_f32),
                top: length(24.0_f32),
                bottom: length(24.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::WHITE)
        .text_aligned(msg.to_string(), 14.0 * zoom, Color::from_rgb8(120, 120, 120), Alignment::Start);
    };

    // Margen del viewport y scroll: el margen interior (24 px / 16 px) no
    // se escala para que el "marco" del documento sea estable; lo que
    // escala es el contenido (font_size + spacing del box tree).
    let mut ctx = RenderCtx {
        zoom,
        matcher,
        find_current,
        find_counter: 0,
        details_open: &t.details_open,
        details_counter: 0,
        inputs: &t.inputs,
        input_checks: &t.input_checks,
        focused_input: t.focused_input,
        input_counter: 0,
        selects: &t.selects,
        select_counter: 0,
        anim_elapsed_ms,
        now_ms,
        hover_tweens: &t.hover_tweens,
        canvas_frames: &t.canvas_frames,
        canvas_images: &t.canvas_images,
    };
    let content = View::new(Style {
        position: TaffyPosition::Absolute,
        inset: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(16.0_f32 - t.scroll_y),
            bottom: auto(),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![render_box(&tree.root, &mut ctx)]);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(Color::WHITE)
    .clip(true)
    .children(vec![content])
}

/// Samplea la animación CSS del nodo (`b.animation`) al instante actual
/// (`ctx.anim_elapsed_ms`) y devuelve un clon del `BoxNode` con el overlay
/// aplicado, o `None` si el nodo no anima o el overlay está vacío. `opacity`/
/// `color`/`background` los pinta el flujo normal de `render_box`; `transforms`
/// se setea para cuando el chrome los aplique (hoy no los renderiza todavía).
pub(crate) fn animation_overlay(b: &BoxNode, ctx: &RenderCtx<'_>) -> Option<BoxNode> {
    let inst = b.animation.as_ref()?;
    let elapsed_s = ctx.anim_elapsed_ms as f32 / 1000.0;
    let progress = puriy_engine::anim::animation_progress(&inst.binding, elapsed_s)?;
    let ov = puriy_engine::anim::sample_keyframes(&inst.keyframes, progress);
    if ov.is_empty() {
        return None;
    }
    let mut nb = b.clone();
    if let Some(o) = ov.opacity {
        nb.opacity = o.clamp(0.0, 1.0);
    }
    if let Some(c) = ov.color {
        nb.color = c;
    }
    if let Some(bg) = ov.background {
        nb.background = Some(bg);
    }
    if let Some(ts) = ov.transforms {
        nb.transforms = ts;
    }
    Some(nb)
}

/// Construye el afín 2D a partir de la lista de `transform` CSS del nodo
/// (`translate`/`scale`/`rotate`, ya sea de la regla estática o del overlay
/// de `@keyframes`). El compositor lo aplica alrededor del centro del rect
/// (CSS `transform-origin: 50% 50%`), así que acá sólo componemos el afín
/// "local" en orden de declaración: `transform: A B C` → matriz `A·B·C`.
/// `translate` se escala por el zoom de página (es px de layout); `scale`/
/// `rotate` son unitless. `None` si la lista está vacía → el nodo no
/// declara transform y el compositor no toca su pintura.
/// Convierte la lista de `Transform` de puriy en (afín fijo, traslación
/// relativa al tamaño). Las `TranslatePct` (`translate(<%>)`) NO caben en un
/// afín fijo —el % resuelve contra el tamaño usado del nodo, que sólo se
/// conoce en composición— así que se acumulan aparte como fracciones y se
/// devuelven para pasarlas a `View::transform_rel`. El resto (px translate,
/// scale, rotate, skew, matrix) va al afín fijo. **Limitación documentada:**
/// las `%` se aplican como factor más externo (al frente de la lista) en
/// Llimphi; si una `translate(<%>)` viene DESPUÉS de un rotate/scale en la
/// lista CSS, el orden se aproxima (caso raro — el patrón usual es
/// `translate(-50%,-50%)` al frente o solo).
pub(crate) fn transform_affine(
    transforms: &[puriy_engine::style::Transform],
    zoom: f32,
) -> (Option<Affine>, Option<(f64, f64)>) {
    use puriy_engine::style::Transform as T;
    if transforms.is_empty() {
        return (None, None);
    }
    let mut a = Affine::IDENTITY;
    let mut has_fixed = false;
    let mut rel = (0.0_f64, 0.0_f64);
    let mut has_rel = false;
    for t in transforms {
        // `TranslatePct` se desvía a las fracciones relativas (Llimphi las
        // resuelve contra el rect); el resto compone el afín fijo.
        if let T::TranslatePct(px, py) = *t {
            rel.0 += px as f64 / 100.0;
            rel.1 += py as f64 / 100.0;
            has_rel = true;
            continue;
        }
        has_fixed = true;
        a *= match *t {
            T::TranslatePct(..) => unreachable!(),
            T::Translate(x, y) => {
                Affine::translate(((x * zoom) as f64, (y * zoom) as f64))
            }
            T::Scale(sx, sy) => Affine::scale_non_uniform(sx as f64, sy as f64),
            T::Rotate(deg) => Affine::rotate((deg as f64).to_radians()),
            // skew: cizalla por la tangente del ángulo en cada eje.
            T::Skew(ax, ay) => Affine::new([
                1.0,
                (ay as f64).to_radians().tan(),
                (ax as f64).to_radians().tan(),
                1.0,
                0.0,
                0.0,
            ]),
            // matrix(a,b,c,d,e,f): afín directa; e/f (traslación) por zoom.
            T::Matrix(a, b, c, d, e, f) => Affine::new([
                a as f64,
                b as f64,
                c as f64,
                d as f64,
                (e * zoom) as f64,
                (f * zoom) as f64,
            ]),
        };
    }
    (
        if has_fixed { Some(a) } else { None },
        if has_rel { Some(rel) } else { None },
    )
}

/// Traduce `transform-origin` del box al pivote del compositor: cada eje como
/// `px + frac · tamaño`. `Px(n)` → offset absoluto (×zoom); `Pct(p)` → fracción
/// `p/100`; cualquier `LengthVal` no resoluble (auto/keywords intrínsecos, que
/// `transform-origin` no admite) → centro del eje (`0.5`). El default CSS
/// `50% 50%` cae en `frac = (0.5, 0.5)` = centro, idéntico a no setear pivote.
pub(crate) fn transform_pivot(
    origin: puriy_engine::style::TransformOrigin,
    zoom: f32,
) -> llimphi_ui::TransformPivot {
    use puriy_engine::style::LengthVal as L;
    let axis = |lv: L| -> (f64, f64) {
        match lv {
            L::Px(n) => (n as f64 * zoom as f64, 0.0),
            L::Pct(p) => (0.0, p as f64 / 100.0),
            _ => (0.0, 0.5),
        }
    };
    let (px_x, fx) = axis(origin.x);
    let (px_y, fy) = axis(origin.y);
    llimphi_ui::TransformPivot { px: (px_x, px_y), frac: (fx, fy) }
}

/// Traduce el `MaskSpec` del box (encaje + modo + cajas `mask-clip`/`-origin`,
/// modelados con los mismos tipos que background) al [`MaskPlacement`] neutral
/// del compositor. `LengthVal` no resoluble (auto/keywords) → `MaskLen::Auto`
/// (intrínseco en size, offset 0 en position). `mask-mode: match-source` →
/// alpha (las máscaras de puriy son raster `url()`). Fases 7.1227–7.1230.
/// Traduce `mask-composite` CSS → operador neutral del compositor. Fase 7.1231.
fn mask_compose_de(c: puriy_engine::style::MaskComposite) -> llimphi_ui::MaskCompose {
    use puriy_engine::style::MaskComposite as C;
    match c {
        C::Add => llimphi_ui::MaskCompose::Add,
        C::Subtract => llimphi_ui::MaskCompose::Subtract,
        C::Intersect => llimphi_ui::MaskCompose::Intersect,
        C::Exclude => llimphi_ui::MaskCompose::Exclude,
    }
}

// Coeficientes de luminancia Rec.709 (los que usa CSS Filter Effects para
// grayscale/saturate). Fase 7.1233.
const LUMA_R: f32 = 0.2126;
const LUMA_G: f32 = 0.7152;
const LUMA_B: f32 = 0.0722;

/// `brightness(k)`: escala lineal de RGB. `k=1` identidad. Fase 7.1233.
fn mat_brightness(k: f32) -> [f32; 20] {
    [
        k, 0., 0., 0., 0., //
        0., k, 0., 0., 0., //
        0., 0., k, 0., 0., //
        0., 0., 0., 1., 0.,
    ]
}

/// `contrast(c)`: `out = c·in + (1-c)/2` sobre RGB. `c=1` identidad. Fase 7.1233.
fn mat_contrast(c: f32) -> [f32; 20] {
    let t = (1.0 - c) * 0.5;
    [
        c, 0., 0., 0., t, //
        0., c, 0., 0., t, //
        0., 0., c, 0., t, //
        0., 0., 0., 1., 0.,
    ]
}

/// `invert(a)`: `out = (1-2a)·in + a`. `a=0` identidad; `a=1` negativo. Fase 7.1233.
fn mat_invert(a: f32) -> [f32; 20] {
    let d = 1.0 - 2.0 * a;
    [
        d, 0., 0., 0., a, //
        0., d, 0., 0., a, //
        0., 0., d, 0., a, //
        0., 0., 0., 1., 0.,
    ]
}

/// `opacity(a)`: escala el canal alpha. `a=1` identidad. Fase 7.1233.
fn mat_opacity(a: f32) -> [f32; 20] {
    [
        1., 0., 0., 0., 0., //
        0., 1., 0., 0., 0., //
        0., 0., 1., 0., 0., //
        0., 0., 0., a, 0.,
    ]
}

/// `saturate(s)`: interpola entre gris (s=0) e identidad (s=1) por luminancia.
/// `s>1` sobresatura. Fase 7.1233.
fn mat_saturate(s: f32) -> [f32; 20] {
    let (lr, lg, lb) = (LUMA_R, LUMA_G, LUMA_B);
    [
        lr + s * (1.0 - lr), lg - s * lg,         lb - s * lb,         0., 0., //
        lr - s * lr,         lg + s * (1.0 - lg), lb - s * lb,         0., 0., //
        lr - s * lr,         lg - s * lg,         lb + s * (1.0 - lb), 0., 0., //
        0., 0., 0., 1., 0.,
    ]
}

/// `grayscale(g)`: `g=0` identidad, `g=1` gris total. Es `saturate(1-g)`. Fase 7.1233.
fn mat_grayscale(g: f32) -> [f32; 20] {
    mat_saturate(1.0 - g)
}

/// `sepia(a)`: `a=0` identidad, `a=1` viraje sepia completo (matriz fija de la
/// spec). Fase 7.1233.
fn mat_sepia(a: f32) -> [f32; 20] {
    let s = 1.0 - a;
    [
        0.393 + 0.607 * s, 0.769 - 0.769 * s, 0.189 - 0.189 * s, 0., 0., //
        0.349 - 0.349 * s, 0.686 + 0.314 * s, 0.168 - 0.168 * s, 0., 0., //
        0.272 - 0.272 * s, 0.534 - 0.534 * s, 0.131 + 0.869 * s, 0., 0., //
        0., 0., 0., 1., 0.,
    ]
}

/// `hue-rotate(deg)`: rotación de matiz preservando luminancia (matriz estándar
/// de SVG `feColorMatrix type="hueRotate"`). `deg=0` identidad. Fase 7.1233.
fn mat_hue_rotate(deg: f32) -> [f32; 20] {
    let rad = deg.to_radians();
    let (c, s) = (rad.cos(), rad.sin());
    [
        0.213 + c * 0.787 - s * 0.213,
        0.715 - c * 0.715 - s * 0.715,
        0.072 - c * 0.072 + s * 0.928,
        0.,
        0., //
        0.213 - c * 0.213 + s * 0.143,
        0.715 + c * 0.285 + s * 0.140,
        0.072 - c * 0.072 - s * 0.283,
        0.,
        0., //
        0.213 - c * 0.213 - s * 0.787,
        0.715 - c * 0.715 + s * 0.715,
        0.072 + c * 0.928 + s * 0.072,
        0.,
        0., //
        0.,
        0.,
        0.,
        1.,
        0.,
    ]
}

/// Convierte la lista CSS `filter` (`Vec<FilterFn>`) a las [`llimphi_ui::FilterOp`]
/// del compositor (filtros sobre el **propio subárbol**). `blur(<px>)` → `Blur`
/// (px = sigma del Gauss, misma convención CSS); los filtros de color colapsan a
/// una matriz 4×5 (`ColorMatrix`); `drop-shadow()` → `DropShadow` (sombra
/// Gaussiana del border-box). El orden de la lista se preserva (la cadena se
/// aplica en secuencia). Las magnitudes en px (blur sigma, offsets/blur de la
/// sombra) se escalan por el zoom de página `z`; las matrices de color son
/// adimensionales. Lista vacía → sin filtro. Fases 7.1232–7.1234.
fn filtros_a_ops(fns: &[puriy_engine::style::FilterFn], z: f32) -> Vec<llimphi_ui::FilterOp> {
    use llimphi_ui::FilterOp as Op;
    use puriy_engine::style::FilterFn as F;
    fns.iter()
        .filter_map(|f| match f {
            F::Blur(px) => (*px > 0.0).then_some(Op::Blur(*px * z)),
            F::Brightness(k) => Some(Op::ColorMatrix(mat_brightness(*k))),
            F::Contrast(c) => Some(Op::ColorMatrix(mat_contrast(*c))),
            F::Grayscale(g) => Some(Op::ColorMatrix(mat_grayscale(g.clamp(0.0, 1.0)))),
            F::Sepia(a) => Some(Op::ColorMatrix(mat_sepia(a.clamp(0.0, 1.0)))),
            F::Saturate(s) => Some(Op::ColorMatrix(mat_saturate(*s))),
            F::Invert(a) => Some(Op::ColorMatrix(mat_invert(a.clamp(0.0, 1.0)))),
            F::HueRotate(d) => Some(Op::ColorMatrix(mat_hue_rotate(*d))),
            F::Opacity(a) => Some(Op::ColorMatrix(mat_opacity(a.clamp(0.0, 1.0)))),
            F::DropShadow(bs) => Some(Op::DropShadow(llimphi_ui::Shadow {
                color: Color::from_rgba8(bs.color.r, bs.color.g, bs.color.b, bs.color.a),
                blur: (bs.blur_px * z) as f64,
                dx: (bs.offset_x * z) as f64,
                dy: (bs.offset_y * z) as f64,
                spread: (bs.spread_px * z) as f64,
            })),
        })
        .collect()
}

/// Suma los `blur(<px>)` de una lista de filtros. Lo usa `backdrop-filter`, que
/// el compositor aplica con el camino nativo [`llimphi_ui::View::backdrop_blur`]
/// (borronea lo pintado *debajo* del nodo). `None` si no hay ningún blur. Fase
/// 7.1232.
fn blur_sigma_de(fns: &[puriy_engine::style::FilterFn]) -> Option<f32> {
    use puriy_engine::style::FilterFn as F;
    let total: f32 = fns
        .iter()
        .map(|f| match f {
            F::Blur(px) => *px,
            _ => 0.0,
        })
        .sum();
    (total > 0.0).then_some(total)
}

fn mask_placement_de(spec: &puriy_engine::MaskSpec) -> llimphi_ui::MaskPlacement {
    use llimphi_ui::{MaskLen, MaskSize};
    use puriy_engine::style::MaskMode as CssMaskMode;
    let len = |lv: LengthVal| -> MaskLen {
        match lv {
            LengthVal::Px(n) => MaskLen::Px(n),
            LengthVal::Pct(p) => MaskLen::Pct(p),
            _ => MaskLen::Auto,
        }
    };
    let size = match spec.size {
        BackgroundSize::Auto => MaskSize::Auto,
        BackgroundSize::Cover => MaskSize::Cover,
        BackgroundSize::Contain => MaskSize::Contain,
        BackgroundSize::Explicit { x, y } => MaskSize::Explicit { x: len(x), y: len(y) },
    };
    let (repeat_x, repeat_y) = match spec.repeat {
        BackgroundRepeat::Repeat => (true, true),
        BackgroundRepeat::RepeatX => (true, false),
        BackgroundRepeat::RepeatY => (false, true),
        BackgroundRepeat::NoRepeat => (false, false),
    };
    // match-source: raster url() → alpha (SVG <mask> daría luminance, pero
    // mask-image en puriy es siempre raster). Default CSS = match-source = alpha.
    let mode = match spec.mode {
        CssMaskMode::Luminance => llimphi_ui::MaskMode::Luminance,
        CssMaskMode::Alpha | CssMaskMode::MatchSource => llimphi_ui::MaskMode::Alpha,
    };
    llimphi_ui::MaskPlacement {
        size,
        pos_x: len(spec.position.x),
        pos_y: len(spec.position.y),
        repeat_x,
        repeat_y,
        mode,
        clip_inset: spec.clip_inset,
        origin_inset: spec.origin_inset,
    }
}

/// Mapea el `cursor` CSS resuelto (Fase 7.240, `BoxNode.cursor`) a la forma
/// de puntero de llimphi. `Auto` ⇒ `None` (sin override: el runtime decide su
/// default). Los resize de una sola dirección (`E/W`, `N/S`) colapsan al par
/// bidireccional que expone llimphi (`EwResize`/`NsResize`), que es la forma
/// real del cursor de redimensionado en esos ejes. Fase 7.1250.
fn map_cursor(c: puriy_engine::style::Cursor) -> Option<llimphi_ui::Cursor> {
    use llimphi_ui::Cursor as L;
    use puriy_engine::style::Cursor as C;
    Some(match c {
        C::Auto => return None,
        C::Default => L::Default,
        C::Pointer => L::Pointer,
        C::Text => L::Text,
        C::Wait => L::Wait,
        C::Help => L::Help,
        C::Crosshair => L::Crosshair,
        C::Move => L::Move,
        C::NotAllowed => L::NotAllowed,
        C::Grab => L::Grab,
        C::Grabbing => L::Grabbing,
        C::ZoomIn => L::ZoomIn,
        C::ZoomOut => L::ZoomOut,
        C::EResize | C::WResize | C::EwResize => L::EwResize,
        C::NResize | C::SResize | C::NsResize => L::NsResize,
        C::NeswResize => L::NeswResize,
        C::NwseResize => L::NwseResize,
        C::RowResize => L::RowResize,
        C::ColResize => L::ColResize,
    })
}

pub(crate) fn render_box(b: &BoxNode, ctx: &mut RenderCtx<'_>) -> View<Msg> {
    // Animación CSS: si el nodo tiene una `@keyframes` resuelta, sampleamos
    // el overlay al instante actual y renderizamos un clon con las props
    // animadas pisadas (el resto del flujo pinta `opacity`/`background`/
    // `color` desde el BoxNode, así que el overlay "se ve" gratis). El clon
    // se computa una sola vez por llamada → sin recursión.
    let overlaid = animation_overlay(b, ctx);
    let b = overlaid.as_ref().unwrap_or(b);
    let zoom = ctx.zoom;
    // <input>/<textarea>: reservar slot y devolver un text_input_view
    // independiente del flujo normal.
    if let Some(kind) = b.input_kind {
        let my_idx = ctx.input_counter;
        ctx.input_counter += 1;
        return render_input(b, kind, my_idx, ctx);
    }
    // <select>: reservar slot y devolver el dropdown (header + opciones).
    if let Some(info) = &b.select {
        let my_idx = ctx.select_counter;
        ctx.select_counter += 1;
        return render_select(b, info, my_idx, ctx);
    }
    // <svg>: bypass del flujo normal — pinta primitivas con vello.
    if let Some(scene) = &b.svg {
        return render_svg(scene, zoom);
    }
    // <canvas>: bypass — el frame del runtime JS se interpreta a vello.
    if let Some((cw, ch)) = b.canvas {
        let frame = b
            .element_id
            .as_deref()
            .and_then(|id| ctx.canvas_frames.get(id));
        return render_canvas(frame, ctx.canvas_images, cw, ch, zoom, b.image_rendering);
    }
    let style = box_style(b, zoom);
    let mut view = View::new(style);
    // `cursor` CSS (Fase 7.240, resuelto en `BoxNode.cursor`): hasta ahora
    // llegaba al box pero el wire nunca lo consumía, así que la forma del
    // puntero no cambiaba sobre links/botones/áreas con `cursor:` del autor.
    // Ahora lo mapeamos a la forma de llimphi. `Auto` ⇒ sin override (el
    // runtime decide). Los <input>/<select> retornan antes (su widget fija el
    // suyo), así que acá no hay conflicto.
    if let Some(c) = map_cursor(b.cursor) {
        view = view.cursor(c);
    }
    // Si este nodo es un <details>, reservamos su slot de estado y
    // renderizamos sólo `<summary>` (precedido de la flecha clickeable)
    // si está cerrado. La rama de `<details>` retorna acá para no caer
    // en el flujo normal de children.
    if b.tag.as_deref() == Some("details") {
        let my_idx = ctx.details_counter;
        ctx.details_counter += 1;
        let open = ctx.details_open.get(my_idx).copied().unwrap_or(false);
        let mut kids: Vec<View<Msg>> = Vec::new();
        for child in &b.children {
            let is_summary = child.tag.as_deref() == Some("summary");
            if is_summary {
                let arrow = if open { "▼ " } else { "▶ " };
                let arrow_view = View::new(Style {
                    size: Size {
                        width: length(16.0_f32 * zoom),
                        height: length(child.font_size * zoom * 1.2),
                    },
                    margin: Rect {
                        left: length(0.0_f32),
                        right: length(2.0_f32 * zoom),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    ..Default::default()
                })
                .text_aligned(
                    arrow.to_string(),
                    child.font_size * zoom,
                    Color::from_rgb8(80, 80, 95),
                    Alignment::Start,
                )
                .on_click(Msg::ToggleDetails(my_idx));
                let summary_view = render_box(child, ctx).on_click(Msg::ToggleDetails(my_idx));
                kids.push(
                    View::new(Style {
                        flex_direction: FlexDirection::Row,
                        align_items: Some(AlignItems::Center),
                        size: Size { width: percent(1.0_f32), height: auto() },
                        ..Default::default()
                    })
                    // Hover feedback sobre toda la fila (flecha + summary)
                    // para que sea evidente que es clickeable. El CSS no
                    // suele estilar `<summary>:hover`, así que es nuestra
                    // contribución de chrome — un gris muy suave.
                    .hover_fill(Color::from_rgba8(0, 0, 0, 18))
                    .on_click(Msg::ToggleDetails(my_idx))
                    .children(vec![arrow_view, summary_view]),
                );
            } else if open {
                kids.push(render_box(child, ctx));
            } else {
                // Cerrado y no-summary: no renderizamos, pero sí
                // avanzamos el counter por cada `<details>` anidado
                // adentro para no desalinear los índices con el vector
                // `details_open` que el Loaded prefilló en orden DFS
                // completo. Sin esto, abrir un parent cerrado le daría
                // a sus hijos índices que el state vector pensaba que
                // correspondían a `<details>` posteriores.
                skip_count_details(child, &mut ctx.details_counter);
            }
        }
        return view.children(kids);
    }
    // Find-in-page: si la query no es vacía y este nodo es una hoja de
    // texto que la contiene (case-insensitive), pintamos su background
    // con un highlight. El N-ésimo match en orden DFS es el "actual"
    // (find_current, 1-based) y pinta en naranja para destacarse —
    // el resto en amarillo. El paint del fill normal del nodo
    // (background CSS) se sobrescribe si hay match.
    let find_hit = b
        .text
        .as_ref()
        .map(|s| ctx.matcher.matches(s))
        .unwrap_or(false);
    let find_hit_color: Option<Color> = if find_hit {
        ctx.find_counter += 1;
        let is_current = ctx.find_current != 0 && ctx.find_counter == ctx.find_current;
        Some(if is_current {
            Color::from_rgba8(255, 140, 0, 240)
        } else {
            Color::from_rgba8(255, 230, 0, 200)
        })
    } else {
        None
    };

    // visibility:hidden ocupa espacio pero no pinta. Devolvemos la view
    // con su layout pero sin children/text/fill — sus descendientes
    // serían computados pero también deberían ser hidden por inheritance.
    let hidden = matches!(b.visibility, Visibility::Hidden);

    // opacity multiplica el alpha del background sólido. text/border
    // se manejan en apply_decorations/render del texto.
    let alpha_mul = b.opacity.clamp(0.0, 1.0);

    if !hidden {
        if let Some(c) = find_hit_color {
            view = view.fill(c);
        } else if let Some(bg) = b.background {
            let a = ((bg.a as f32) * alpha_mul) as u8;
            view = view.fill(Color::from_rgba8(bg.r, bg.g, bg.b, a));
        }
        if let Some(hbg) = b.hover_background {
            // ¿El nodo declara una `transition` que cubre el background? Si
            // sí, NO usamos el swap instantáneo del compositor (`hover_fill`):
            // tweeneamos el fill nosotros frame a frame y anclamos el reloj
            // con `on_pointer_enter/leave`. El find-in-page (find_hit_color)
            // gana sobre la transición — no querés tweenear un highlight.
            let bg_transition = puriy_engine::anim::transition_for(&b.transitions, "background-color")
                .or_else(|| puriy_engine::anim::transition_for(&b.transitions, "background"));
            match (find_hit_color, bg_transition) {
                (None, Some(tr)) => {
                    let duration_ms = (tr.duration_s * 1000.0).max(0.0) as u32;
                    // `from` = background actual; si no hay, el color de hover
                    // pero transparente (fade-in desde nada).
                    let base = b.background.unwrap_or(puriy_engine::Color {
                        r: hbg.r,
                        g: hbg.g,
                        b: hbg.b,
                        a: 0,
                    });
                    let lin = ctx
                        .hover_tweens
                        .get(&b.node_id)
                        .map(|tw| tw.sample_linear(ctx.now_ms))
                        .unwrap_or(0.0);
                    let eased = puriy_engine::anim::apply_easing(tr.timing, lin);
                    let cur = puriy_engine::anim::lerp_color(&base, &hbg, eased);
                    let a = ((cur.a as f32) * alpha_mul) as u8;
                    view = view
                        .fill(Color::from_rgba8(cur.r, cur.g, cur.b, a))
                        .on_pointer_enter(Msg::HoverTween {
                            node_id: b.node_id,
                            entering: true,
                            duration_ms,
                        })
                        .on_pointer_leave(Msg::HoverTween {
                            node_id: b.node_id,
                            entering: false,
                            duration_ms,
                        });
                }
                _ => {
                    let a = ((hbg.a as f32) * alpha_mul) as u8;
                    view = view.hover_fill(Color::from_rgba8(hbg.r, hbg.g, hbg.b, a));
                }
            }
        }
        view = apply_decorations(view, b, zoom);
    }
    if hidden {
        // Sin children/text — el subárbol queda invisible pero ocupando
        // su layout. Devolvemos acá para evitar pintar nada.
        return view;
    }
    // `overflow: hidden` aplica clip(true) — recorta el subárbol al
    // borde del rect del nodo.
    if matches!(b.overflow, Overflow::Hidden) {
        view = view.clip(true);
    }
    // `clip-path: inset(...)` (Fase 7.1219) — recorta a un rect encogido por
    // los insets px desde el border box. Implica clip aunque overflow no sea
    // hidden. Formas no rectangulares (circle/ellipse) no se modelan acá.
    if let Some(insets) = b.clip_inset {
        view = view.clip_inset(insets);
    }
    // `clip-path: circle(...)` / `ellipse(...)` (Fase 7.1220) — recorta a una
    // elipse. El spec `[cx_px, cx_pct, cy_px, cy_pct, rx, ry]` deja el centro
    // en forma (px, pct); el compositor lo resuelve contra el rect del nodo.
    if let Some(spec) = b.clip_ellipse {
        view = view.clip_ellipse(spec);
    }
    // `clip-path: polygon(...)` (Fase 7.1223) — recorta a un polígono. Cada
    // punto `[x_px, x_pct, y_px, y_pct]` lo resuelve el compositor contra el
    // rect del nodo.
    if let Some((evenodd, pts)) = &b.clip_polygon {
        view = view.clip_polygon(*evenodd, pts.clone());
    }
    // `clip-path: path(...)` (Fase 7.1224) — recorta a un path SVG; el
    // compositor lo parsea con BezPath::from_svg y lo ancla al rect.
    if let Some((evenodd, d)) = &b.clip_path_svg {
        view = view.clip_path_svg(*evenodd, d.clone());
    }
    // `clip-path` geometry-box (Fase 7.1225) — el compositor encoge el rect a
    // la caja de referencia antes de resolver la forma; sin forma, recorta a
    // ese rect.
    if let Some(insets) = b.clip_ref_inset {
        view = view.clip_ref_inset(insets);
    }
    // `mask-image: url(...)` (Fase 7.1226/7.1227) — máscara de luminancia del
    // subárbol. La imagen ya viene decodificada (RGBA8) desde el box build,
    // junto con su encaje `mask-size`/`-position`/`-repeat`; el compositor
    // multiplica el alpha del contenido por la luminancia de la máscara,
    // tileándola igual que background-image. Ortogonal a clip-path (un nodo
    // puede llevar ambos).
    if let Some(spec) = &b.mask_image {
        let to_peniko = |img: &puriy_engine::ImageData| {
            PenikoImage::new(ImageData {
                data: Blob::from(img.rgba.clone()),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width: img.width,
                height: img.height,
            })
        };
        view = view
            .mask_image(to_peniko(&spec.image))
            .mask_placement(mask_placement_de(spec));
        // Capas adicionales (mask-image: url(a), url(b), …) con su operador
        // mask-composite. Fase 7.1231.
        if !spec.extra.is_empty() {
            let extra: Vec<(PenikoImage, llimphi_ui::MaskCompose)> = spec
                .extra
                .iter()
                .map(|(img, comp)| (to_peniko(img), mask_compose_de(*comp)))
                .collect();
            view = view.mask_extra(extra);
        }
    }
    // `filter: blur(...)` (Fase 7.1232) — borronea el **propio subárbol** del
    // nodo como post-pasada GPU sobre la intermediate. Ortogonal a clip/mask.
    // Las demás funciones de filtro (brightness/grayscale/drop-shadow/…) llegan
    // en fases siguientes.
    let mut fops = filtros_a_ops(&b.filter, ctx.zoom);
    // `backdrop-filter` (Fases 7.1232/7.1235): el `blur` va al camino nativo
    // `backdrop_blur` (pre-renderiza el fondo y compone el contenido nítido
    // encima — más fiel). Los filtros de color van a la MISMA post-pasada que
    // `filter` (v1: colorean los píxeles finales del rect, no sólo el fondo —
    // misma limitación documentada). `drop-shadow` en backdrop no tiene sentido
    // y se descarta.
    if let Some(sigma) = blur_sigma_de(&b.backdrop_filter) {
        view = view.backdrop_blur(sigma * ctx.zoom);
    }
    for op in filtros_a_ops(&b.backdrop_filter, ctx.zoom) {
        if matches!(op, llimphi_ui::FilterOp::ColorMatrix(_)) {
            fops.push(op);
        }
    }
    if !fops.is_empty() {
        view = view.filter(fops);
    }

    // `mix-blend-mode` (Fase 7.1237): el nodo entero (su subárbol) se mezcla
    // contra su backdrop con el modo CSS resuelto. El compositor abre una capa
    // de blend alrededor del rect del nodo y la cierra al fin del subárbol; el
    // dato llega del box (`b.mix_blend_mode`, parseo de Fase 7.255). `normal`
    // → None (sin capa, source-over). Ortogonal a clip/mask/filter.
    if let Some(bm) = blend_mode_peniko(b.mix_blend_mode) {
        view = view.blend(bm);
    }

    let link_color = Color::from_rgb8(30, 90, 200);
    let display_color = if b.link.is_some() {
        link_color
    } else {
        Color::from_rgb8(b.color.r, b.color.g, b.color.b)
    };

    // pointer-events:none deshabilita on_click (también propaga por
    // inheritance, así que los descendientes ya lo tienen marcado).
    let pe_active = matches!(b.pointer_events, PointerEvents::Auto);

    if let Some(target) = &b.link {
        if pe_active {
            // `<a download>` descarga el target en lugar de navegar. El
            // filename hint queda en `b.link_download` (String vacío =
            // usar nombre del path).
            let native_msg = if let Some(filename_hint) = &b.link_download {
                Msg::DownloadLink {
                    url: target.clone(),
                    filename_hint: filename_hint.clone(),
                }
            } else if b.link_new_tab {
                Msg::NavigateNewTab(target.clone())
            } else {
                Msg::Navigate(target.clone())
            };
            // Fase 7.6 — cohabitación link+handler: si el `<a>` tiene
            // `id=`, despachamos el evento JS PRIMERO y la navegación
            // queda como fallback. El handler puede llamar
            // `event.preventDefault()` para cancelar la nav.
            let click_msg = if let Some(eid) = &b.element_id {
                if !eid.is_empty() {
                    Msg::JsDispatchEvent {
                        element_id: eid.clone(),
                        event_type: "click".into(),
                        fallback: Some(Box::new(native_msg.clone())),
                    }
                } else {
                    native_msg.clone()
                }
            } else {
                native_msg.clone()
            };
            view = view
                .on_click(click_msg)
                .on_middle_click(Msg::NavigateNewTab(target.clone()))
                .on_pointer_enter(Msg::HoverLink(Some(target.clone())))
                .on_pointer_leave(Msg::HoverLink(None));
        }
    } else if let Some(eid) = &b.element_id {
        // Elemento con `id=` y sin link/download/submit nativo: si JS
        // registró handlers para 'click', el chrome los dispara. Sin
        // handlers, `dispatch_event` devuelve count=0 y nada pasa.
        if pe_active && !eid.is_empty() && !matches!(b.display, Display::None) {
            view = view.on_click(Msg::JsDispatchEvent {
                element_id: eid.clone(),
                event_type: "click".into(),
                fallback: None,
            });
        }
    }

    // <img> con imagen decodificada: arma peniko::Image, ajusta el rect
    // del nodo al tamaño nativo (taffy luego lo clampa por el ancho del
    // contenedor). Llimphi escala preservando aspect ratio.
    if let Some(img) = &b.image {
        let blob = Blob::from(img.rgba.clone());
        // `image-rendering` (Fase 7.1239): fija la calidad de muestreo de la
        // imagen (`pixelated`/`crisp-edges` → nearest; `smooth` → bilineal).
        let peniko = with_image_rendering(
            PenikoImage::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: img.width, height: img.height }),
            b.image_rendering,
        );
        return match b.object_fit {
            Some(fit) => image_fit_view(b, peniko, fit, zoom),
            None => image_view(img.width, img.height, zoom).image(peniko),
        };
    }

    if let Some(text) = &b.text {
        let base = if b.font_weight >= 600 { b.font_size * 1.1 } else { b.font_size };
        let size = base * zoom;
        // text-shadows: paint_with previo al texto. Cada shadow se pinta
        // como una segunda capa de texto desplazada y semitransparente —
        // peniko no expone draw text directo desde el callback, así que
        // usamos un rect aproximado proporcional al tamaño de fuente.
        // Aproximación suficiente para hero text decorativo.
        if !b.text_shadows.is_empty() {
            let shadows = b.text_shadows.clone();
            let z = zoom as f64;
            view = view.paint_with(move |scene, _ts, rect| {
                for sh in &shadows {
                    // Banda horizontal centrada de altura ≈ font_size,
                    // desplazada por (offset_x, offset_y), expandida por
                    // blur. Alpha proporcional al blur (más blur = más
                    // difuso = menos opaco).
                    let extra = sh.blur_px as f64 * 0.5 * z;
                    let mid_y = rect.y as f64 + rect.h as f64 * 0.55;
                    let h = size as f64 * 0.55;
                    let r = KurboRect::new(
                        rect.x as f64 + sh.offset_x as f64 * z - extra,
                        mid_y - h * 0.5 + sh.offset_y as f64 * z - extra,
                        (rect.x + rect.w) as f64 + sh.offset_x as f64 * z + extra,
                        mid_y + h * 0.5 + sh.offset_y as f64 * z + extra,
                    );
                    let alpha = if sh.blur_px > 0.0 { 0.35 } else { 0.6 };
                    let c = Color::from_rgba8(
                        sh.color.r,
                        sh.color.g,
                        sh.color.b,
                        (sh.color.a as f64 * alpha) as u8,
                    );
                    scene.fill(Fill::NonZero, Affine::IDENTITY, c, None, &r);
                }
            });
        }
        let italic = matches!(b.font_style, puriy_engine::FontStyle::Italic);
        // `background-clip: text` (Fase 7.208): si la hoja trae el gradiente
        // propagado (build), rellenamos sus glifos con él vía `paint_with` +
        // `draw_layout_brush_xf`, y dejamos el color normal TRANSPARENTE para
        // que la pasada de texto de llimphi (que corre después) no lo tape.
        let grad_text =
            if matches!(b.background_clip, puriy_engine::style::BackgroundClip::Text) {
                b.background_gradient.clone()
            } else {
                None
            };
        let text_fill = if grad_text.is_some() {
            Color::from_rgba8(0, 0, 0, 0)
        } else {
            display_color
        };
        if let Some(g) = grad_text {
            let txt = text.clone();
            let size_c = size;
            let italic_c = italic;
            let ff = b.font_family.clone();
            let lh = b.line_height.unwrap_or(1.2);
            let alpha = b.opacity.clamp(0.0, 1.0);
            let ls_c = b.letter_spacing * zoom;
            let ws_c = b.word_spacing * zoom;
            // `overflow-wrap`/`word-break`: mismo criterio que el wire de abajo,
            // para que el re-shaping del gradiente parta la palabra igual que la
            // pasada normal.
            let ow_c = matches!(
                b.overflow_wrap,
                puriy_engine::style::OverflowWrap::BreakWord
                    | puriy_engine::style::OverflowWrap::Anywhere
            ) || matches!(b.word_break, puriy_engine::style::WordBreak::BreakAll);
            view = view.paint_with(move |scene, ts, rect| {
                // Re-shaping idéntico al de la pasada normal (mismo
                // size/wrap/alignment/line-height) para que las glifos del
                // gradiente caigan exactamente sobre las transparentes.
                let layout = ts.layout(
                    &txt,
                    size_c,
                    Some(rect.w),
                    Alignment::Start,
                    lh,
                    italic_c,
                    ff.as_deref(),
                    400.0,
                    false,
                    false,
                    ls_c,
                    ws_c,
                    ow_c,
                );
                // Gradiente en coords LOCALES (0,0)-(w,h): `draw_layout_brush_xf`
                // lo lleva al origen del texto con la afín, alineándolo.
                let w = layout.width().max(1.0);
                let h = llimphi_ui::llimphi_text::measurement(&layout).height.max(1.0);
                let local = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w, h };
                if let Some(grad) = build_linear_gradient_brush(&g, local, alpha) {
                    let brush = llimphi_raster::peniko::Brush::Gradient(grad);
                    llimphi_ui::llimphi_text::draw_layout_brush_xf(
                        scene,
                        &layout,
                        &brush,
                        Affine::translate((rect.x as f64, rect.y as f64)),
                    );
                }
            });
        }
        view = view
            .text_aligned_full(
                text.clone(),
                size,
                text_fill,
                Alignment::Start,
                italic,
                b.font_family.clone(),
            )
            .line_height(b.line_height.unwrap_or(1.2));
        // `letter-spacing`/`word-spacing` (Fase 7.1252): px extra entre
        // letras/palabras. Escalan por zoom como cualquier longitud. 0 = normal
        // (no-op en el shaper). Sólo el camino uniforme; el RichText (mixed
        // inline) los ignora en v1.
        if b.letter_spacing != 0.0 {
            view = view.letter_spacing(b.letter_spacing * zoom);
        }
        if b.word_spacing != 0.0 {
            view = view.word_spacing(b.word_spacing * zoom);
        }
        // `text-overflow: ellipsis` (Fase 7.1251): el engine propagó la
        // intención a esta hoja (el contenedor tiene `overflow != visible`).
        // `ellipsis(1)` clampa a una línea y termina en `…` cuando el texto
        // no entra en el ancho de la caja — el clásico single-line ellipsis.
        if matches!(b.text_overflow, puriy_engine::style::TextOverflow::Ellipsis) {
            view = view.ellipsis(1);
        }
        // `white-space: nowrap`/`pre` (Fase 7.1253): el engine heredó el
        // `white-space` del contenedor a esta hoja. Los valores que NO envuelven
        // (`NoWrap`/`Pre`) shapean en una sola línea; el texto desborda y lo
        // recorta el `overflow` del contenedor. `Normal`/`PreWrap`/`PreLine`
        // envuelven (default). Combina con `ellipsis(1)` para el clásico
        // single-line `overflow:hidden + white-space:nowrap + text-overflow:ellipsis`.
        if matches!(
            b.white_space,
            puriy_engine::WhiteSpace::NoWrap | puriy_engine::WhiteSpace::Pre
        ) {
            view = view.no_wrap();
        }
        // `overflow-wrap: break-word`/`anywhere` y `word-break: break-all`
        // (Fase 7.1254): el engine heredó la política de quiebre a esta hoja.
        // Cualquiera de ellas habilita partir DENTRO de una palabra cuando un
        // token es más ancho que la caja (en vez de desbordar). `Normal`/
        // `keep-all`/`auto-phrase` no parten (default). Inocuo bajo `no_wrap`:
        // ahí el texto va en una línea sin `max_width`, así que no hay quiebre
        // que partir.
        let ow = matches!(
            b.overflow_wrap,
            puriy_engine::style::OverflowWrap::BreakWord
                | puriy_engine::style::OverflowWrap::Anywhere
        );
        let wb = matches!(b.word_break, puriy_engine::style::WordBreak::BreakAll);
        if ow || wb {
            view = view.overflow_wrap();
        }
        return view;
    }

    if !b.children.is_empty() {
        let kids: Vec<View<Msg>> = if let Some(target) = &b.link {
            // Dentro de un <a>, los descendientes son no-interactive por
            // contagio (ya enlazan al target del <a>). No esperamos
            // <details> dentro de links — pero contamos por las dudas
            // para no romper el invariante del counter.
            let target = target.clone();
            let new_tab = b.link_new_tab;
            b.children
                .iter()
                .map(|c| render_link_subtree(c, &target, link_color, new_tab, ctx))
                .collect()
        } else if is_mixed_inline_context(b) {
            // Contexto inline con más de un hijo (texto + elementos inline
            // como <b>/<a>/<code>): partimos cada run de texto en palabras
            // para que TODO fluya palabra-a-palabra junto a los elementos.
            // Sin esto, el run de texto se mide como un bloque multi-línea y
            // el elemento inline queda colgado después, no en la misma línea.
            render_inline_flow(&b.children, ctx)
        } else {
            render_children_z_ordered(&b.children, ctx)
        };
        view = view.children(kids);
    }
    // Transform CSS (estático o animado por `@keyframes`): el compositor lo
    // aplica al nodo y todo su subtree alrededor del centro de su rect. Se
    // setea al final para que cubra fill/text/decorations/children juntos. El
    // afín fijo va por `transform`; las `translate(<%>)` (que dependen del
    // tamaño usado) por `transform_rel`, que Llimphi resuelve en composición.
    let (xf, rel) = transform_affine(&b.transforms, zoom);
    if let Some(xf) = xf {
        view = view.transform(xf);
    }
    if let Some(rel) = rel {
        view = view.transform_rel(rel);
    }
    // `transform-origin` (Fase 7.1248): pivote de la transformación. Sólo importa
    // si hay transform; lo seteamos en ese caso (el default `50% 50%` cae en el
    // centro, idéntico al comportamiento previo sin pivote).
    if xf.is_some() || rel.is_some() {
        view = view.transform_origin(transform_pivot(b.transform_origin, zoom));
    }
    view
}

/// Renderea los children aplicando z-index: in-flow primero (orden
/// DOM), luego out-of-flow (position absolute/fixed) ordenados por
/// z-index ascendente — mayor pinta encima de los demás. Reordenar
/// los out-of-flow es seguro porque su layout depende de insets, no
/// de su posición en el Vec.
pub(crate) fn render_children_z_ordered(children: &[BoxNode], ctx: &mut RenderCtx<'_>) -> Vec<View<Msg>> {
    let mut in_flow_idx: Vec<usize> = Vec::new();
    let mut out_of_flow_idx: Vec<usize> = Vec::new();
    for (i, c) in children.iter().enumerate() {
        match c.position {
            puriy_engine::Position::Absolute | puriy_engine::Position::Fixed => {
                out_of_flow_idx.push(i)
            }
            _ => in_flow_idx.push(i),
        }
    }
    // Sort estable por z-index ascending; ties mantienen orden DOM.
    out_of_flow_idx.sort_by_key(|&i| children[i].z_index);
    in_flow_idx
        .into_iter()
        .chain(out_of_flow_idx)
        .map(|i| render_box(&children[i], ctx))
        .collect()
}

/// ¿`b` es un contexto inline "mixto"? — todos sus hijos son inline y hay
/// **más de uno** (p. ej. texto + `<b>` + texto). Ese es el caso donde el
/// modelo "un run = un item flex" se rompe visualmente y conviene partir el
/// texto en palabras. Un párrafo de un solo run de texto (`children.len()==1`)
/// NO entra acá: se mide entero (envuelve a N líneas) y conserva el
/// find-in-page por hoja.
pub(crate) fn is_mixed_inline_context(b: &BoxNode) -> bool {
    b.children.len() > 1 && has_inline_children(b)
}

/// Renderiza un contexto inline mixto partiendo cada hoja de texto en
/// palabras: cada palabra es un item flex propio, así el `flex-wrap` del
/// bloque rompe líneas en los límites de palabra y los elementos inline
/// (`<b>`, `<code>`, `<a>`…) fluyen en la misma línea que el texto vecino.
/// Las hojas no-texto (elementos inline) se renderizan como una unidad.
pub(crate) fn render_inline_flow(children: &[BoxNode], ctx: &mut RenderCtx<'_>) -> Vec<View<Msg>> {
    let mut out: Vec<View<Msg>> = Vec::new();
    for c in children {
        match &c.text {
            // Hoja de texto: una vista por palabra (clon del nodo con el
            // texto reemplazado), reusando el render normal — hereda
            // color/peso/tamaño/familia/line-height sin duplicar lógica.
            Some(text) if c.children.is_empty() => {
                for word in split_words(text) {
                    let mut wn = c.clone();
                    wn.text = Some(word);
                    out.push(render_box(&wn, ctx));
                }
            }
            _ => out.push(render_box(c, ctx)),
        }
    }
    out
}

/// Parte un run de texto (ya whitespace-colapsado a espacios simples) en
/// tokens "palabra " con el espacio separador pegado, de modo que cada token
/// mida su propio ancho (incluido el espacio) y los words se separen al
/// fluir. Preserva un espacio inicial (separa del elemento inline anterior) y
/// recorta el espacio final si el run no terminaba en espacio.
pub(crate) fn split_words(s: &str) -> Vec<String> {
    let leading = s.starts_with(' ');
    let mut out: Vec<String> = Vec::new();
    for (i, w) in s.split(' ').filter(|w| !w.is_empty()).enumerate() {
        let mut tok = String::new();
        if i == 0 && leading {
            tok.push(' ');
        }
        tok.push_str(w);
        tok.push(' ');
        out.push(tok);
    }
    if !s.ends_with(' ') {
        if let Some(last) = out.last_mut() {
            if last.ends_with(' ') {
                last.pop();
            }
        }
    }
    out
}

/// Recorre `b` y avanza `*counter` por cada `<details>` descendiente.
/// Usado por el chrome cuando un `<details>` padre está cerrado: aunque
/// no rendereamos los hijos non-summary, sí tenemos que consumir sus
/// índices para que no se desalineen con el vector `details_open` que
/// el Loaded prefilló en DFS completo.
/// Si el input focado está dentro de un `<form>`, arma la URL `action?
/// n1=v1&n2=v2&…` con los inputs que tienen `name` no vacío,
/// urlencodeados de manera mínima. Devuelve `None` si no hay form
/// asociado o si el form no tiene action navegable.
pub(crate) fn build_form_submit_url(m: &Model) -> Option<Msg> {
    let t = m.active();
    let focused_idx = t.focused_input?;
    let tree = t.box_tree.as_ref()?;
    // Primer pase: identificá el form_idx del input focado.
    let mut focused_form: Option<usize> = None;
    let mut counter: usize = 0;
    tree.walk(|b| {
        if b.input_kind.is_some() {
            if counter == focused_idx {
                focused_form = b.form_idx;
            }
            counter += 1;
        }
    });
    let form_idx = focused_form?;
    // Segundo pase: junta los pares (name, value) de los inputs y
    // `<select>`s del mismo form que tengan `name`. Texto del input vive
    // en `t.inputs[idx]`; valor del select en `t.selects[idx].selected`
    // → SelectInfo.options[i].value.
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut input_idx: usize = 0;
    let mut select_idx: usize = 0;
    tree.walk(|b| {
        if let Some(kind) = b.input_kind {
            let my_idx = input_idx;
            input_idx += 1;
            if b.form_idx == Some(form_idx) {
                if let Some(name) = &b.input_name {
                    match kind {
                        puriy_engine::InputKind::Checkbox
                        | puriy_engine::InputKind::Radio => {
                            let checked = t.input_checks.get(my_idx).copied().unwrap_or(false);
                            if checked {
                                let val = b
                                    .input_initial
                                    .clone()
                                    .unwrap_or_else(|| "on".to_string());
                                pairs.push((name.clone(), val));
                            }
                            // No-checked checkbox/radio: NO se manda
                            // (HTML spec).
                        }
                        puriy_engine::InputKind::Submit => {
                            // Submit con name: contribuye su `value`/label.
                            let val = b
                                .input_initial
                                .clone()
                                .unwrap_or_else(|| "Submit".to_string());
                            pairs.push((name.clone(), val));
                        }
                        _ => {
                            let value = t
                                .inputs
                                .get(my_idx)
                                .map(|s| s.text())
                                .unwrap_or_default();
                            pairs.push((name.clone(), value));
                        }
                    }
                }
            }
        }
        if let Some(info) = &b.select {
            let my_idx = select_idx;
            select_idx += 1;
            if b.form_idx == Some(form_idx) {
                if let Some(name) = &b.input_name {
                    let sel = t
                        .selects
                        .get(my_idx)
                        .map(|s| s.selected)
                        .unwrap_or(info.initial);
                    let value = info
                        .options
                        .get(sel)
                        .map(|o| o.value.clone())
                        .unwrap_or_default();
                    pairs.push((name.clone(), value));
                }
            }
        }
    });
    let form = tree.forms.get(form_idx)?;
    let action = form.action.clone()?;
    // URL-encoder mínimo (espacios → '+', resto de chars unsafe → %HH).
    fn encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for &b in s.as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char);
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
        out
    }
    let qs: Vec<String> = pairs.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect();
    let body = qs.join("&");
    match form.method {
        puriy_engine::FormMethod::Get => {
            // Concatena action con `?…`. Si action ya tiene `?`, usamos `&`.
            let sep = if action.contains('?') { '&' } else { '?' };
            Some(Msg::Navigate(format!("{}{}{}", action, sep, body)))
        }
        puriy_engine::FormMethod::Post => Some(Msg::NavigatePost { url: action, body }),
    }
}

pub(crate) fn skip_count_details(b: &BoxNode, counter: &mut usize) {
    if b.tag.as_deref() == Some("details") {
        *counter += 1;
    }
    for c in &b.children {
        skip_count_details(c, counter);
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;
    use puriy_engine::style::FilterFn as F;

    const IDENTITY: [f32; 20] = [
        1., 0., 0., 0., 0., //
        0., 1., 0., 0., 0., //
        0., 0., 1., 0., 0., //
        0., 0., 0., 1., 0.,
    ];

    fn aprox(a: [f32; 20], b: [f32; 20]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn los_valores_neutros_dan_identidad() {
        // Cada filtro en su valor "sin efecto" debe colapsar a la matriz
        // identidad (clave: encadenar un neutro no cambia nada). Fase 7.1233.
        assert!(aprox(mat_brightness(1.0), IDENTITY), "brightness(1)");
        assert!(aprox(mat_contrast(1.0), IDENTITY), "contrast(1)");
        assert!(aprox(mat_invert(0.0), IDENTITY), "invert(0)");
        assert!(aprox(mat_opacity(1.0), IDENTITY), "opacity(1)");
        assert!(aprox(mat_saturate(1.0), IDENTITY), "saturate(1)");
        assert!(aprox(mat_grayscale(0.0), IDENTITY), "grayscale(0)");
        assert!(aprox(mat_sepia(0.0), IDENTITY), "sepia(0)");
        assert!(aprox(mat_hue_rotate(0.0), IDENTITY), "hue-rotate(0)");
    }

    #[test]
    fn grayscale_total_es_luminancia() {
        // grayscale(1): cada fila RGB = coeficientes de luminancia Rec.709, así
        // los tres canales de salida quedan iguales (gris). Fase 7.1233.
        let m = mat_grayscale(1.0);
        for fila in 0..3 {
            assert!((m[fila * 5] - LUMA_R).abs() < 1e-4);
            assert!((m[fila * 5 + 1] - LUMA_G).abs() < 1e-4);
            assert!((m[fila * 5 + 2] - LUMA_B).abs() < 1e-4);
        }
        // grayscale(g) == saturate(1-g).
        assert!(aprox(mat_grayscale(0.4), mat_saturate(0.6)));
    }

    #[test]
    fn invert_total_es_negativo() {
        // invert(1): diagonal -1 + bias 1 → out = 1 - in. Fase 7.1233.
        let m = mat_invert(1.0);
        assert!((m[0] + 1.0).abs() < 1e-4, "diag R = -1");
        assert!((m[4] - 1.0).abs() < 1e-4, "bias R = 1");
        assert!((m[9] - 1.0).abs() < 1e-4, "bias G = 1");
        assert!((m[14] - 1.0).abs() < 1e-4, "bias B = 1");
    }

    #[test]
    fn brightness_y_opacity_escalan_lo_suyo() {
        // brightness(2): diagonal RGB = 2, alpha intacta. Fase 7.1233.
        let b = mat_brightness(2.0);
        assert!((b[0] - 2.0).abs() < 1e-4 && (b[6] - 2.0).abs() < 1e-4 && (b[12] - 2.0).abs() < 1e-4);
        assert!((b[18] - 1.0).abs() < 1e-4, "alpha intacta");
        // opacity(0.5): RGB identidad, alpha *= 0.5.
        let o = mat_opacity(0.5);
        assert!((o[18] - 0.5).abs() < 1e-4, "alpha 0.5");
        assert!((o[0] - 1.0).abs() < 1e-4, "RGB intacto");
    }

    #[test]
    fn filtros_a_ops_mapea_y_preserva_orden() {
        // Mezcla blur + color: cada uno → su FilterOp, en el mismo orden CSS.
        // Fase 7.1233.
        let ops = filtros_a_ops(
            &[F::Grayscale(1.0), F::Blur(3.0), F::Brightness(1.5)],
            1.0,
        );
        assert_eq!(ops.len(), 3);
        assert!(matches!(ops[0], llimphi_ui::FilterOp::ColorMatrix(_)));
        assert!(matches!(ops[1], llimphi_ui::FilterOp::Blur(v) if (v - 3.0).abs() < 1e-4));
        assert!(matches!(ops[2], llimphi_ui::FilterOp::ColorMatrix(_)));
        // blur(0) no aporta op.
        let vacios = filtros_a_ops(&[F::Blur(0.0)], 1.0);
        assert!(vacios.is_empty());
    }

    #[test]
    fn blur_y_drop_shadow_escalan_por_zoom() {
        // Las magnitudes en px (sigma del blur, offsets/blur de la sombra)
        // escalan por el zoom de página; las matrices de color no. Fase 7.1234.
        use puriy_engine::Color as ECol;
        let bs = puriy_engine::style::BoxShadow {
            offset_x: 2.0,
            offset_y: 4.0,
            blur_px: 6.0,
            spread_px: 0.0,
            color: ECol { r: 0, g: 0, b: 0, a: 128 },
            inset: false,
        };
        let ops = filtros_a_ops(&[F::Blur(3.0), F::DropShadow(bs)], 2.0);
        assert!(matches!(ops[0], llimphi_ui::FilterOp::Blur(v) if (v - 6.0).abs() < 1e-4));
        match &ops[1] {
            llimphi_ui::FilterOp::DropShadow(sh) => {
                assert!((sh.dx - 4.0).abs() < 1e-4, "offset_x * zoom");
                assert!((sh.dy - 8.0).abs() < 1e-4, "offset_y * zoom");
                assert!((sh.blur - 12.0).abs() < 1e-4, "blur * zoom");
            }
            _ => panic!("esperaba DropShadow"),
        }
    }

    #[test]
    fn map_cursor_traduce_y_auto_no_overridea() {
        // Fase 7.1250: el `cursor` CSS resuelto se mapea a la forma de llimphi.
        use llimphi_ui::Cursor as L;
        use puriy_engine::style::Cursor as C;
        // `auto` no overridea (None ⇒ el runtime decide su default).
        assert_eq!(map_cursor(C::Auto), None);
        // Mapeos 1:1 representativos.
        assert_eq!(map_cursor(C::Pointer), Some(L::Pointer));
        assert_eq!(map_cursor(C::Text), Some(L::Text));
        assert_eq!(map_cursor(C::NotAllowed), Some(L::NotAllowed));
        assert_eq!(map_cursor(C::Grabbing), Some(L::Grabbing));
        assert_eq!(map_cursor(C::ZoomIn), Some(L::ZoomIn));
        // Los resize de una sola dirección colapsan al par bidireccional.
        assert_eq!(map_cursor(C::EResize), Some(L::EwResize));
        assert_eq!(map_cursor(C::WResize), Some(L::EwResize));
        assert_eq!(map_cursor(C::NResize), Some(L::NsResize));
        assert_eq!(map_cursor(C::SResize), Some(L::NsResize));
        assert_eq!(map_cursor(C::NeswResize), Some(L::NeswResize));
        assert_eq!(map_cursor(C::ColResize), Some(L::ColResize));
    }
}
