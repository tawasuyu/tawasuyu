//! Render walk: traducción del `BoxTree` (CSS computado por puriy-engine) a la
//! jerarquía de `View<Msg>` de Llimphi. Incluye `viewport` (entrada desde la
//! UI), `render_box` y sus especializaciones (links, inputs, checkbox/radio,
//! submit, select, svg, canvas), `box_style` (BoxNode→taffy Style), las
//! decoraciones (bordes/sombras/fondos), los mappers CSS→taffy y el armado del
//! gradiente lineal. Extraído de `lib.rs` (regla #1). Comparte todos los tipos
//! del crate vía `use super::*`.
use super::*;

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
pub(crate) fn transform_affine(transforms: &[puriy_engine::style::Transform], zoom: f32) -> Option<Affine> {
    use puriy_engine::style::Transform as T;
    if transforms.is_empty() {
        return None;
    }
    let mut a = Affine::IDENTITY;
    for t in transforms {
        a *= match *t {
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
    Some(a)
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
        return render_canvas(frame, ctx.canvas_images, cw, ch, zoom);
    }
    let style = box_style(b, zoom);
    let mut view = View::new(style);
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
        let peniko = PenikoImage::new(blob, ImageFormat::Rgba8, img.width, img.height);
        return image_view(img.width, img.height, zoom).image(peniko);
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
        return view
            .text_aligned_full(
                text.clone(),
                size,
                text_fill,
                Alignment::Start,
                italic,
                b.font_family.clone(),
            )
            .line_height(b.line_height.unwrap_or(1.2));
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
    // setea al final para que cubra fill/text/decorations/children juntos.
    if let Some(xf) = transform_affine(&b.transforms, zoom) {
        view = view.transform(xf);
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

/// View dimensionada para una imagen — ancho hasta `width_px` pero
/// nunca más que el contenedor (`max_width: 100%`), altura proporcional
/// vía aspect ratio inverso (`width / height`).
pub(crate) fn image_view(width: u32, height: u32, zoom: f32) -> View<Msg> {
    let w = (width.max(1)) as f32 * zoom;
    let h = (height.max(1)) as f32 * zoom;
    let ratio = if height > 0 { Some(width as f32 / height as f32) } else { None };
    View::new(Style {
        size: Size { width: length(w), height: length(h) },
        // `max-width: 100%` clampa el ancho al contenedor (responsive
        // por default — sin esto, imágenes grandes rompen layouts narrow);
        // `aspect_ratio` deja que taffy preserve la proporción cuando el
        // ancho real (post-clamp) sea menor que `length(w)`.
        max_size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        aspect_ratio: ratio,
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(4.0_f32 * zoom),
            bottom: length(4.0_f32 * zoom),
        },
        ..Default::default()
    })
}

pub(crate) fn render_link_subtree(
    b: &BoxNode,
    target: &str,
    color: Color,
    new_tab: bool,
    ctx: &mut RenderCtx<'_>,
) -> View<Msg> {
    let zoom = ctx.zoom;
    // <details> dentro de un <a> es HTML inválido en la práctica; pero
    // si aparece, contamos el slot igualmente para no desalinear el
    // counter global. No reescribimos el comportamiento interactivo:
    // dentro de un link el subtree colapsado se ignora.
    if b.tag.as_deref() == Some("details") {
        skip_count_details(b, &mut ctx.details_counter);
    }
    let nav_msg = |t: &str| {
        if new_tab {
            Msg::NavigateNewTab(t.to_string())
        } else {
            Msg::Navigate(t.to_string())
        }
    };
    let mut view = View::new(box_style(b, zoom))
        .on_click(nav_msg(target))
        .on_middle_click(Msg::NavigateNewTab(target.to_string()));
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
    if let Some(c) = find_hit_color {
        view = view.fill(c);
    } else if let Some(bg) = b.background {
        view = view.fill(Color::from_rgb8(bg.r, bg.g, bg.b));
    }
    if let Some(img) = &b.image {
        let blob = Blob::from(img.rgba.clone());
        let peniko = PenikoImage::new(blob, ImageFormat::Rgba8, img.width, img.height);
        return image_view(img.width, img.height, zoom)
            .image(peniko)
            .on_click(nav_msg(target));
    }
    if let Some(text) = &b.text {
        let base = if b.font_weight >= 600 { b.font_size * 1.1 } else { b.font_size };
        let size = base * zoom;
        let italic = matches!(b.font_style, puriy_engine::FontStyle::Italic);
        return view
            .text_aligned_full(
                text.clone(),
                size,
                color,
                Alignment::Start,
                italic,
                b.font_family.clone(),
            )
            .line_height(b.line_height.unwrap_or(1.2));
    }
    if !b.children.is_empty() {
        let target_owned = target.to_string();
        view = view.children(
            b.children
                .iter()
                .map(|c| render_link_subtree(c, &target_owned, color, new_tab, ctx))
                .collect(),
        );
    }
    view
}

/// `<input type=text>` / `<input type=search>` / `<input type=password>` /
/// `<textarea>`: arma un widget `text_input_view` ligado al
/// `TextInputState` del slot DFS `idx`. Click→focus dispara
/// `Msg::FocusInput(idx)`. El estilo del input se mantiene básico
/// (border gris claro + padding); el font-size hereda del nodo. Sin
/// soporte de submit/Enter por ahora — Enter en un input single-line
/// no hace nada (en un textarea inserta newline via apply_key).
pub(crate) fn render_input(
    b: &BoxNode,
    kind: puriy_engine::InputKind,
    idx: usize,
    ctx: &mut RenderCtx<'_>,
) -> View<Msg> {
    let zoom = ctx.zoom;
    // Checkbox / radio / submit: widgets a parte (no text_input_view).
    match kind {
        puriy_engine::InputKind::Checkbox => {
            return render_checkbox_radio(b, idx, ctx, /* radio */ false);
        }
        puriy_engine::InputKind::Radio => {
            return render_checkbox_radio(b, idx, ctx, /* radio */ true);
        }
        puriy_engine::InputKind::Submit => {
            return render_submit_button(b, idx, ctx);
        }
        _ => {}
    }
    let focused = ctx.focused_input == Some(idx);
    // Estado por slot — usamos un blank si todavía no hay (no debería
    // pasar tras Loaded, pero defensivo).
    let blank = TextInputState::new();
    let state = ctx.inputs.get(idx).unwrap_or(&blank);

    let placeholder = b
        .input_placeholder
        .as_deref()
        .unwrap_or(match kind {
            puriy_engine::InputKind::Search => "buscar…",
            puriy_engine::InputKind::Password => "contraseña",
            puriy_engine::InputKind::TextArea => "",
            _ => "",
        });

    let palette = TextInputPalette::default();
    let input = text_input_view(state, placeholder, focused, &palette, Msg::FocusInput(idx));

    // Tamaño: ancho 100% del contenedor por default (los autores suelen
    // poner `width: 200px` o similar; el CSS engine ya lo materializa
    // como `b.width`). El alto: una línea para text/search/password, un
    // textarea recibe ~5 líneas.
    let line_h = (b.font_size * zoom).max(14.0_f32 * zoom) + 12.0;
    let height = match kind {
        puriy_engine::InputKind::TextArea => line_h * 5.0,
        _ => line_h,
    };
    let css_width = length_to_taffy(b.width, zoom);

    // Background base: CSS background-color del nodo si lo seteó; sino
    // blanco. Cuando está focado y el autor escribió `:focus { background:
    // X }`, aplicamos X.
    let base_bg = b
        .background
        .map(|c| Color::from_rgba8(c.r, c.g, c.b, c.a))
        .unwrap_or(Color::WHITE);
    let bg = if focused {
        b.focus_background
            .map(|c| Color::from_rgba8(c.r, c.g, c.b, c.a))
            .unwrap_or(base_bg)
    } else {
        base_bg
    };
    let outline = b
        .outline
        .color
        .filter(|_| focused && b.outline.style_active && b.outline.width > 0.0);

    let mut wrapper = View::new(Style {
        size: Size {
            width: css_width.unwrap_or_else(|| length(220.0_f32 * zoom)),
            height: length(height),
        },
        padding: Rect {
            left: length(6.0_f32 * zoom),
            right: length(6.0_f32 * zoom),
            top: length(4.0_f32 * zoom),
            bottom: length(4.0_f32 * zoom),
        },
        margin: Rect {
            left: margin_left_lpa(b, zoom),
            right: margin_right_lpa(b, zoom, 0.0),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .on_click(Msg::FocusInput(idx));
    // Ring de focus visible: si el autor no proveyó outline, lo damos
    // gratis para feedback. Stroke azul accent estándar.
    if focused && outline.is_none() {
        wrapper = wrapper.paint_with(|scene, _ts, rect| {
            let stroke = Stroke::new(2.0);
            let half = stroke.width * 0.5;
            let r = RoundedRect::new(
                rect.x as f64 - half,
                rect.y as f64 - half,
                (rect.x + rect.w) as f64 + half,
                (rect.y + rect.h) as f64 + half,
                3.0 + half,
            );
            scene.stroke(
                &stroke,
                Affine::IDENTITY,
                Color::from_rgba8(40, 110, 220, 255),
                None,
                &r,
            );
        });
    }
    wrapper.children(vec![input])
}

/// `<input type=checkbox|radio>`: caja chica con `☐`/`☑` (o circle
/// vacío/lleno para radio) clickeable. Sin label asociada — el `<label
/// for="...">` no se cablea todavía, pero el click sobre el widget
/// alcanza para toggle.
pub(crate) fn render_checkbox_radio(
    b: &BoxNode,
    idx: usize,
    ctx: &mut RenderCtx<'_>,
    radio: bool,
) -> View<Msg> {
    let zoom = ctx.zoom;
    let checked = ctx.input_checks.get(idx).copied().unwrap_or(false);
    let glyph = if radio {
        if checked { "●" } else { "○" }
    } else if checked {
        "☑"
    } else {
        "☐"
    };
    let msg = if radio { Msg::SelectRadio(idx) } else { Msg::ToggleCheckbox(idx) };
    let size_px = (b.font_size * zoom).max(14.0 * zoom);
    View::new(Style {
        size: Size {
            width: length(size_px + 4.0),
            height: length(size_px + 4.0),
        },
        margin: Rect {
            left: margin_left_lpa(b, zoom),
            right: margin_right_lpa(b, zoom, 4.0),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .on_click(msg)
    .text_aligned(
        glyph.to_string(),
        size_px,
        Color::from_rgb8(40, 40, 50),
        Alignment::Center,
    )
}

/// `<input type=submit|button>` — botón con label desde `value` o
/// default `Submit`. Click submitea el form.
pub(crate) fn render_submit_button(b: &BoxNode, idx: usize, ctx: &mut RenderCtx<'_>) -> View<Msg> {
    let zoom = ctx.zoom;
    let label = b
        .input_initial
        .clone()
        .unwrap_or_else(|| "Submit".to_string());
    let css_width = length_to_taffy(b.width, zoom);
    let h = (b.font_size * zoom).max(14.0 * zoom) + 12.0;
    View::new(Style {
        size: Size {
            width: css_width.unwrap_or_else(|| length(120.0 * zoom)),
            height: length(h),
        },
        padding: Rect {
            left: length(10.0 * zoom),
            right: length(10.0 * zoom),
            top: length(6.0 * zoom),
            bottom: length(6.0 * zoom),
        },
        margin: Rect {
            left: margin_left_lpa(b, zoom),
            right: margin_right_lpa(b, zoom, 0.0),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(230, 230, 240))
    .hover_fill(Color::from_rgb8(215, 220, 235))
    .radius(3.0)
    .on_click(Msg::SubmitForm(idx))
    .text_aligned(
        label,
        b.font_size * zoom,
        Color::from_rgb8(30, 30, 40),
        Alignment::Center,
    )
}

/// `<select>` con `<option>`s: renderea un header click-toggle con la
/// opción elegida + flecha; cuando está abierto, expande la lista
/// debajo. Click en una opción la selecciona y cierra el dropdown.
pub(crate) fn render_select(
    b: &BoxNode,
    info: &puriy_engine::SelectInfo,
    idx: usize,
    ctx: &mut RenderCtx<'_>,
) -> View<Msg> {
    let zoom = ctx.zoom;
    let state = ctx.selects.get(idx);
    let selected = state.map(|s| s.selected).unwrap_or(info.initial);
    let open = state.map(|s| s.open).unwrap_or(false);
    let current_label = info
        .options
        .get(selected)
        .map(|o| o.label.clone())
        .unwrap_or_default();

    let css_width = length_to_taffy(b.width, zoom);
    let header_h = (b.font_size * zoom).max(14.0_f32 * zoom) + 10.0;
    let header = View::new(Style {
        size: Size {
            width: css_width.clone().unwrap_or_else(|| length(220.0_f32 * zoom)),
            height: length(header_h),
        },
        padding: Rect {
            left: length(8.0_f32 * zoom),
            right: length(8.0_f32 * zoom),
            top: length(4.0_f32 * zoom),
            bottom: length(4.0_f32 * zoom),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::WHITE)
    .radius(3.0)
    .on_click(Msg::SelectToggle(idx))
    .children(vec![
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(header_h - 8.0) },
            ..Default::default()
        })
        .text_aligned(
            truncate(&current_label, 80),
            b.font_size * zoom,
            Color::from_rgb8(30, 30, 40),
            Alignment::Start,
        ),
        View::new(Style {
            size: Size {
                width: length(14.0_f32 * zoom),
                height: length(header_h - 8.0),
            },
            ..Default::default()
        })
        .text_aligned(
            if open { "▲".to_string() } else { "▼".to_string() },
            b.font_size * zoom * 0.8,
            Color::from_rgb8(80, 80, 95),
            Alignment::End,
        ),
    ]);

    // El header se rendera siempre; la lista expandida ahora vive en
    // `view_overlay` (popup flotante) cuando `open=true`. Esto evita
    // empujar el flow del documento al abrir un select.
    let _ = (selected, info, open); // ya consumidos en el overlay
    let all: Vec<View<Msg>> = vec![header];

    View::new(Style {
        size: Size {
            width: css_width.unwrap_or_else(|| length(220.0_f32 * zoom)),
            height: auto(),
        },
        margin: Rect {
            left: margin_left_lpa(b, zoom),
            right: margin_right_lpa(b, zoom, 0.0),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::from_rgb8(220, 220, 230))
    .radius(3.0)
    .children(all)
}

/// Overlay flotante con la lista de opciones del `<select>` abierto.
/// Centrado en la ventana; backdrop semitransparente que cierra el
/// dropdown al clickear fuera de la lista.
pub(crate) fn select_overlay_view(idx: usize, selected: usize, info: puriy_engine::SelectInfo) -> View<Msg> {
    let row_h = 28.0_f32;
    let total_h = (info.options.len() as f32 * row_h).min(360.0);
    let rows: Vec<View<Msg>> = info
        .options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            let is_sel = i == selected;
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(row_h) },
                padding: Rect {
                    left: length(12.0_f32),
                    right: length(12.0_f32),
                    top: length(4.0_f32),
                    bottom: length(4.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(if is_sel {
                Color::from_rgb8(220, 230, 250)
            } else {
                Color::WHITE
            })
            .hover_fill(Color::from_rgb8(238, 240, 248))
            .on_click(Msg::SelectPick(idx, i))
            .text_aligned(
                truncate(&opt.label, 80),
                13.0,
                Color::from_rgb8(30, 30, 40),
                Alignment::Start,
            )
        })
        .collect();

    let list = View::new(Style {
        size: Size { width: length(320.0_f32), height: length(total_h) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::from_rgb8(245, 245, 250))
    .radius(4.0)
    .clip(true)
    .children(rows);

    // Backdrop fullscreen con flex centering del list. Click en el
    // backdrop cierra el dropdown via SelectToggle.
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 60))
    .on_click(Msg::SelectToggle(idx))
    .children(vec![list])
}

/// Pinta las primitivas de un `<svg>` dentro de un rect del tamaño
/// `scene.width × scene.height` (escalado por zoom). Si `view_box` está
/// definido, las primitivas se mapean a [0..1] vía viewBox y luego se
/// escalan al rect del nodo (preservando aspect ratio, "meet").
pub(crate) fn render_svg(scene: &puriy_engine::SvgScene, zoom: f32) -> View<Msg> {
    use llimphi_raster::kurbo::{Circle as KurboCircle, Line as KurboLine};
    let w = scene.width * zoom;
    let h = scene.height * zoom;
    let prims = scene.prims.clone();
    let view_box = scene.view_box;
    let svg_w = scene.width;
    let svg_h = scene.height;
    View::new(Style {
        size: Size { width: length(w), height: length(h) },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        // Mapping local → pantalla. Si hay viewBox, normalizamos por él
        // y escalamos al rect; sino usamos directamente width/height del
        // svg como dominio.
        let (src_x, src_y, src_w, src_h) = view_box.unwrap_or((0.0, 0.0, svg_w, svg_h));
        let sx = if src_w > 0.0 { rect.w as f64 / src_w as f64 } else { 1.0 };
        let sy = if src_h > 0.0 { rect.h as f64 / src_h as f64 } else { 1.0 };
        let s = sx.min(sy).max(0.001);
        let to_x = |x: f32| rect.x as f64 + ((x - src_x) as f64) * s;
        let to_y = |y: f32| rect.y as f64 + ((y - src_y) as f64) * s;
        let to_color = |c: puriy_engine::Color| {
            Color::from_rgba8(c.r, c.g, c.b, c.a)
        };
        for p in &prims {
            match *p {
                puriy_engine::SvgPrim::Rect {
                    x, y, w, h, rx, fill, stroke, stroke_w,
                } => {
                    let r = RoundedRect::new(
                        to_x(x),
                        to_y(y),
                        to_x(x + w),
                        to_y(y + h),
                        (rx as f64) * s,
                    );
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &r);
                    }
                    if let Some(st) = stroke {
                        let stroke = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke, Affine::IDENTITY, to_color(st), None, &r);
                    }
                }
                puriy_engine::SvgPrim::Circle { cx, cy, r, fill, stroke, stroke_w } => {
                    let c = KurboCircle::new((to_x(cx), to_y(cy)), r as f64 * s);
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &c);
                    }
                    if let Some(st) = stroke {
                        let stroke = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke, Affine::IDENTITY, to_color(st), None, &c);
                    }
                }
                puriy_engine::SvgPrim::Line { x1, y1, x2, y2, stroke, stroke_w } => {
                    let l = KurboLine::new((to_x(x1), to_y(y1)), (to_x(x2), to_y(y2)));
                    let stroke_obj = Stroke::new(stroke_w as f64 * s);
                    scene.stroke(&stroke_obj, Affine::IDENTITY, to_color(stroke), None, &l);
                }
                puriy_engine::SvgPrim::Polyline {
                    ref points, closed, fill, stroke, stroke_w,
                } => {
                    use llimphi_raster::kurbo::{BezPath, PathEl, Point as KurboPoint};
                    let mut path = BezPath::new();
                    let mut iter = points.iter();
                    if let Some(&(x, y)) = iter.next() {
                        path.push(PathEl::MoveTo(KurboPoint::new(to_x(x), to_y(y))));
                        for &(x, y) in iter {
                            path.push(PathEl::LineTo(KurboPoint::new(to_x(x), to_y(y))));
                        }
                        if closed {
                            path.push(PathEl::ClosePath);
                        }
                    }
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &path);
                    }
                    if let Some(st) = stroke {
                        let stroke_obj = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke_obj, Affine::IDENTITY, to_color(st), None, &path);
                    }
                }
                puriy_engine::SvgPrim::Path { ref d, fill, stroke, stroke_w } => {
                    use llimphi_raster::kurbo::{BezPath, PathEl, Point as KurboPoint};
                    let mut path = BezPath::new();
                    for cmd in d {
                        match *cmd {
                            puriy_engine::PathCmd::MoveTo(x, y) => {
                                path.push(PathEl::MoveTo(KurboPoint::new(to_x(x), to_y(y))));
                            }
                            puriy_engine::PathCmd::LineTo(x, y) => {
                                path.push(PathEl::LineTo(KurboPoint::new(to_x(x), to_y(y))));
                            }
                            puriy_engine::PathCmd::CubicTo(x1, y1, x2, y2, x, y) => {
                                path.push(PathEl::CurveTo(
                                    KurboPoint::new(to_x(x1), to_y(y1)),
                                    KurboPoint::new(to_x(x2), to_y(y2)),
                                    KurboPoint::new(to_x(x), to_y(y)),
                                ));
                            }
                            puriy_engine::PathCmd::QuadTo(x1, y1, x, y) => {
                                path.push(PathEl::QuadTo(
                                    KurboPoint::new(to_x(x1), to_y(y1)),
                                    KurboPoint::new(to_x(x), to_y(y)),
                                ));
                            }
                            puriy_engine::PathCmd::ClosePath => {
                                path.push(PathEl::ClosePath);
                            }
                        }
                    }
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &path);
                    }
                    if let Some(st) = stroke {
                        let stroke_obj = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke_obj, Affine::IDENTITY, to_color(st), None, &path);
                    }
                }
            }
        }
    })
}

/// Una capa de background extra ya lista para pintar dentro del closure de
/// `paint_with` (la imagen ya envuelta en `peniko::Image`, o el gradiente
/// clonado). Preparada fuera del closure para no capturar `BoxNode`.
pub(crate) enum PreparedBgLayer {
    Image {
        img: PenikoImage,
        iw: f64,
        ih: f64,
        size: puriy_engine::style::BackgroundSize,
        position: puriy_engine::style::BackgroundPosition,
        repeat: puriy_engine::style::BackgroundRepeat,
    },
    Gradient(puriy_engine::style::LinearGradient),
}

/// Pinta las capas de background EXTRA (debajo de la capa 0) dentro de `rect`.
/// CSS pinta la primera capa de la lista arriba, así que estas van debajo y se
/// pintan en orden inverso (la última de la lista, la más al fondo, primero).
/// Cada capa es imagen (vía `paint_background_image`) o gradiente lineal.
pub(crate) fn paint_extra_bg_layers(
    scene: &mut llimphi_raster::vello::Scene,
    rect: llimphi_ui::PaintRect,
    radius: f64,
    layers: &[PreparedBgLayer],
    alpha_mul: f32,
) {
    for layer in layers.iter().rev() {
        match layer {
            PreparedBgLayer::Gradient(g) => {
                if let Some(brush) = build_linear_gradient_brush(g, rect, alpha_mul) {
                    let r = RoundedRect::new(
                        rect.x as f64,
                        rect.y as f64,
                        (rect.x + rect.w) as f64,
                        (rect.y + rect.h) as f64,
                        radius,
                    );
                    scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &r);
                }
            }
            PreparedBgLayer::Image { img, iw, ih, size, position, repeat } => {
                if *iw > 0.0 && *ih > 0.0 {
                    // Las capas extra usan border-box para origin y clip (los
                    // box-values del shorthand sólo viajan a la capa 0).
                    paint_background_image(
                        scene, rect, rect, radius, img, *iw, *ih, *size, *position, *repeat,
                    );
                }
            }
        }
    }
}

/// Pinta `background-image` dentro de `area` resolviendo `background-size`,
/// `background-position` y `background-repeat` (Fase 7.204). `area` es el área
/// de posicionamiento (`background-origin`, Fase 7.207): contra ella se calculan
/// `cover`/`contain`, los `%` y el origen del tiling. El pintado se recorta a
/// `clip_rect`/`clip_radius` (`background-clip`) con un clip layer. `iw`/`ih`
/// son las dimensiones naturales de la imagen (> 0, garantizado por el caller).
/// Asume transform identidad (los radios ya vienen escalados).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_background_image(
    scene: &mut llimphi_raster::vello::Scene,
    area: llimphi_ui::PaintRect,
    clip_rect: llimphi_ui::PaintRect,
    clip_radius: f64,
    img: &PenikoImage,
    iw: f64,
    ih: f64,
    size: BackgroundSize,
    position: BackgroundPosition,
    repeat: BackgroundRepeat,
) {
    let rect = area;
    let rw = rect.w as f64;
    let rh = rect.h as f64;
    if rw <= 0.0 || rh <= 0.0 {
        return;
    }
    // 1) Tamaño del tile (tw, th) en px.
    let resolve = |lv: LengthVal, basis: f64| -> Option<f64> {
        match lv {
            LengthVal::Px(n) => Some(n as f64),
            LengthVal::Pct(p) => Some(basis * p as f64 / 100.0),
            LengthVal::Auto => None,
        }
    };
    let (tw, th) = match size {
        BackgroundSize::Auto => (iw, ih),
        BackgroundSize::Cover => {
            let s = (rw / iw).max(rh / ih);
            (iw * s, ih * s)
        }
        BackgroundSize::Contain => {
            let s = (rw / iw).min(rh / ih);
            (iw * s, ih * s)
        }
        BackgroundSize::Explicit { x, y } => match (resolve(x, rw), resolve(y, rh)) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * ih / iw), // alto por aspecto
            (None, Some(h)) => (h * iw / ih, h), // ancho por aspecto
            (None, None) => (iw, ih),            // ambos auto = natural
        },
    };
    if tw <= 0.5 || th <= 0.5 {
        return;
    }
    // 2) Offset del primer tile (relativo al origen del rect). Para %, el
    //    punto p% de la imagen se alinea con el p% del box (semántica CSS).
    let pos_off = |lv: LengthVal, basis: f64, tile: f64| -> f64 {
        match lv {
            LengthVal::Px(n) => n as f64,
            LengthVal::Pct(p) => (basis - tile) * p as f64 / 100.0,
            LengthVal::Auto => 0.0,
        }
    };
    let ox = pos_off(position.x, rw, tw);
    let oy = pos_off(position.y, rh, th);
    // 3) Ejes a tilear + posiciones de inicio cubriendo [0, span].
    let (rep_x, rep_y) = match repeat {
        BackgroundRepeat::Repeat => (true, true),
        BackgroundRepeat::RepeatX => (true, false),
        BackgroundRepeat::RepeatY => (false, true),
        BackgroundRepeat::NoRepeat => (false, false),
    };
    let axis_positions = |off: f64, tile: f64, span: f64, rep: bool| -> Vec<f64> {
        if !rep {
            return vec![off];
        }
        let mut start = off;
        while start > 0.0 {
            start -= tile;
        }
        let mut v = Vec::new();
        let mut p = start;
        // Cap defensivo (tiles diminutos no deben colgar el render).
        while p < span && v.len() < 4096 {
            v.push(p);
            p += tile;
        }
        v
    };
    let xs = axis_positions(ox, tw, rw, rep_x);
    let ys = axis_positions(oy, th, rh, rep_y);
    // 4) Clip a la caja de `background-clip` (borde redondeado) y dibujo de
    //    cada tile. El tiling se ancla a `area` (origin); el recorte a clip_rect.
    let clip = RoundedRect::new(
        clip_rect.x as f64,
        clip_rect.y as f64,
        (clip_rect.x + clip_rect.w) as f64,
        (clip_rect.y + clip_rect.h) as f64,
        clip_radius,
    );
    scene.push_layer(llimphi_raster::peniko::Mix::Clip, 1.0, Affine::IDENTITY, &clip);
    let scale = Affine::scale_non_uniform(tw / iw, th / ih);
    for &x in &xs {
        for &y in &ys {
            let tf = Affine::translate((rect.x as f64 + x, rect.y as f64 + y)) * scale;
            scene.draw_image(img, tf);
        }
    }
    scene.pop_layer();
}

/// Aplica `border-radius` y dibuja, en una sola pasada de `paint_with`,
/// la sombra (si la hay) y el contorno del border (si lo hay). Vello
/// pinta el callback entre el `fill` y la `image`/`text` del view, así
/// que la sombra cae detrás del contenido pero encima del fondo del
/// parent. Aproximación: sin gaussian blur — el `blur_px` se mapea
/// como expansión adicional del rect con alpha proporcional, lo cual
/// da una sombra "dura" pero proporcionada.
pub(crate) fn apply_decorations(mut view: View<Msg>, b: &BoxNode, zoom: f32) -> View<Msg> {
    let z = zoom;
    // Radio del clip del view: usamos el máximo de las 4 esquinas (Llimphi
    // `View::radius` toma un escalar). Cuando las 4 esquinas son iguales
    // el resultado es exacto; cuando difieren, el clip queda con la
    // esquina más redonda — el border per-side dibujado abajo seguirá
    // marcando las corners individuales.
    let radii = b.border_radii;
    let radius_max =
        radii.top_left.max(radii.top_right).max(radii.bottom_right).max(radii.bottom_left);
    if radius_max > 0.0 {
        view = view.radius((radius_max * z) as f64);
    }
    let radius = (radius_max * z) as f64;
    let shadow = b.box_shadow.map(|s| BoxShadow {
        offset_x: s.offset_x * z,
        offset_y: s.offset_y * z,
        blur_px: s.blur_px * z,
        spread_px: s.spread_px * z,
        color: s.color,
    });
    let alpha_mul = b.opacity.clamp(0.0, 1.0);
    // Border uniforme = los 4 lados con mismo width y color. Lo
    // dibujamos como RoundedRect stroke para que las corners radius
    // queden suaves. Si los lados difieren, pintamos cada uno como
    // segmento independiente (Border::Sides) — las corners en ese caso
    // van en chaflán cuadrado, que matchea el look estándar de browsers
    // cuando se mezclan widths/colors por lado.
    let bw = b.border_widths;
    let bc = b.border_colors;
    let uniform_border = if bw.top == bw.right
        && bw.right == bw.bottom
        && bw.bottom == bw.left
        && bc.top == bc.right
        && bc.right == bc.bottom
        && bc.bottom == bc.left
        && bw.top > 0.0
    {
        bc.top.map(|c| (c, bw.top * z))
    } else {
        None
    };
    let per_side_border = if uniform_border.is_none() {
        let s_top = bc.top.filter(|_| bw.top > 0.0).map(|c| (c, bw.top * z));
        let s_right = bc.right.filter(|_| bw.right > 0.0).map(|c| (c, bw.right * z));
        let s_bottom = bc.bottom.filter(|_| bw.bottom > 0.0).map(|c| (c, bw.bottom * z));
        let s_left = bc.left.filter(|_| bw.left > 0.0).map(|c| (c, bw.left * z));
        if s_top.is_some() || s_right.is_some() || s_bottom.is_some() || s_left.is_some() {
            Some((s_top, s_right, s_bottom, s_left))
        } else {
            None
        }
    } else {
        None
    };
    // outline se pinta fuera del border + offset, sin afectar layout. Si
    // `style_active` es false (none/hidden) o falta color, no pinta.
    let outline = if b.outline.style_active
        && b.outline.width > 0.0
        && b.outline.color.is_some()
    {
        Some((
            b.outline.color.unwrap(),
            b.outline.width * z,
            b.outline.offset * z,
        ))
    } else {
        None
    };
    // text-decoration sólo tiene efecto visual sobre hojas de texto. En
    // un nodo container, la línea ya la pinta cada hoja descendiente.
    let deco = if b.text.is_some() && b.text_decoration != TextDecorationLine::None {
        Some((b.text_decoration, b.color, b.font_size * z))
    } else {
        None
    };
    let gradient = b.background_gradient.clone();
    // `background-image: url(...)`: si el engine pudo descargarla, la
    // envolvemos en peniko::Image para que el closure de paint_with la
    // escale/posicione/tile dentro del rect según `background-size`,
    // `background-position` y `background-repeat` (Fase 7.204).
    let bg_image = b.background_image.as_ref().map(|img| {
        let blob = Blob::from(img.rgba.clone());
        let peniko = PenikoImage::new(blob, ImageFormat::Rgba8, img.width, img.height);
        (peniko, img.width as f64, img.height as f64)
    });
    let bg_size = b.background_size;
    let bg_position = b.background_position;
    let bg_repeat = b.background_repeat;
    // `background-origin` / `background-clip` (Fase 7.207). Insets (escalados)
    // del border-box hacia adentro: padding-box descuenta el border; content-box
    // border + padding. Aplican a imagen y gradiente de la capa 0; el color
    // sólido sigue al border-box (lo pinta el `View::fill`, fuera del closure).
    let pad = b.padding;
    let pb_ins = (bw.left * z, bw.top * z, bw.right * z, bw.bottom * z);
    let cb_ins = (
        (bw.left + pad.left) * z,
        (bw.top + pad.top) * z,
        (bw.right + pad.right) * z,
        (bw.bottom + pad.bottom) * z,
    );
    let origin_ins = match b.background_origin {
        puriy_engine::style::BackgroundOrigin::BorderBox => (0.0, 0.0, 0.0, 0.0),
        puriy_engine::style::BackgroundOrigin::PaddingBox => pb_ins,
        puriy_engine::style::BackgroundOrigin::ContentBox => cb_ins,
    };
    let clip_ins = match b.background_clip {
        puriy_engine::style::BackgroundClip::PaddingBox => pb_ins,
        puriy_engine::style::BackgroundClip::ContentBox => cb_ins,
        // BorderBox y Text no insetean el rect (Text ni siquiera pinta el
        // fondo como rect — lo rellena en las glifos de la hoja de texto).
        _ => (0.0, 0.0, 0.0, 0.0),
    };
    // `background-clip: text` (Fase 7.208): el elemento estilado NO pinta su
    // gradiente/imagen como rect — el relleno va a las glifos de su texto hijo
    // (propagado en build a la hoja). Acá lo suprimimos en el rect.
    let clip_is_text =
        matches!(b.background_clip, puriy_engine::style::BackgroundClip::Text);
    // Capas de background EXTRA (debajo de la capa 0). Cada una es imagen
    // (raster ya decodificado) o gradiente. Se preparan acá para no capturar
    // el BoxNode dentro del closure.
    let extra_layers: Vec<PreparedBgLayer> = b
        .background_extra_layers
        .iter()
        .filter_map(|l| {
            if let Some(img) = &l.image {
                let blob = Blob::from(img.rgba.clone());
                let peniko = PenikoImage::new(blob, ImageFormat::Rgba8, img.width, img.height);
                Some(PreparedBgLayer::Image {
                    img: peniko,
                    iw: img.width as f64,
                    ih: img.height as f64,
                    size: l.size,
                    position: l.position,
                    repeat: l.repeat,
                })
            } else {
                l.gradient.clone().map(PreparedBgLayer::Gradient)
            }
        })
        .collect();
    if shadow.is_none()
        && uniform_border.is_none()
        && per_side_border.is_none()
        && deco.is_none()
        && outline.is_none()
        && gradient.is_none()
        && bg_image.is_none()
        && extra_layers.is_empty()
    {
        return view;
    }
    view.paint_with(move |scene, _typesetter, rect| {
        // Capas de background EXTRA, debajo de la capa 0.
        paint_extra_bg_layers(scene, rect, radius, &extra_layers, alpha_mul);
        // Cajas de `background-origin` (área de posicionamiento) y
        // `background-clip` (recorte) de la capa 0. Insetean el border-box.
        let inset = |ins: (f32, f32, f32, f32)| llimphi_ui::PaintRect {
            x: rect.x + ins.0,
            y: rect.y + ins.1,
            w: (rect.w - ins.0 - ins.2).max(0.0),
            h: (rect.h - ins.1 - ins.3).max(0.0),
        };
        let origin_rect = inset(origin_ins);
        let clip_rect = inset(clip_ins);
        // El radio interno se encoge con el inset (esquina top-left como repr.).
        let clip_radius = (radius - clip_ins.0.max(clip_ins.1) as f64).max(0.0);
        // linear-gradient: se dimensiona contra el origin box y se recorta al
        // clip box. peniko interpreta `Linear { start, end }` como las dos
        // puntas — `build_linear_gradient_brush` cruza el rect dado.
        if let Some(g) = &gradient {
            if !clip_is_text {
                if let Some(brush) = build_linear_gradient_brush(g, origin_rect, alpha_mul) {
                    let r = RoundedRect::new(
                        clip_rect.x as f64,
                        clip_rect.y as f64,
                        (clip_rect.x + clip_rect.w) as f64,
                        (clip_rect.y + clip_rect.h) as f64,
                        clip_radius,
                    );
                    scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &r);
                }
            }
        }
        // background-image: resuelve tamaño/posición/repeat (Fase 7.204) contra
        // el origin box y tilea, recortado al clip box (Fase 7.207).
        if let Some((img, iw, ih)) = &bg_image {
            if *iw > 0.0 && *ih > 0.0 && !clip_is_text {
                paint_background_image(
                    scene, origin_rect, clip_rect, clip_radius, img, *iw, *ih, bg_size,
                    bg_position, bg_repeat,
                );
            }
        }
        if let Some(BoxShadow { offset_x, offset_y, blur_px, spread_px, color }) = shadow {
            let extra = (blur_px + spread_px) as f64;
            let half_alpha = if blur_px > 0.0 { 0.55 } else { 0.85 };
            let sc = Color::from_rgba8(
                color.r,
                color.g,
                color.b,
                (color.a as f64 * half_alpha) as u8,
            );
            let r = RoundedRect::new(
                (rect.x + offset_x) as f64 - extra,
                (rect.y + offset_y) as f64 - extra,
                (rect.x + rect.w + offset_x) as f64 + extra,
                (rect.y + rect.h + offset_y) as f64 + extra,
                (radius + extra).max(0.0),
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, sc, None, &r);
        }
        if let Some((bc, w)) = uniform_border {
            let stroke = Stroke::new(w as f64);
            let half = stroke.width * 0.5;
            let r = RoundedRect::new(
                rect.x as f64 + half,
                rect.y as f64 + half,
                (rect.x + rect.w) as f64 - half,
                (rect.y + rect.h) as f64 - half,
                (radius - half).max(0.0),
            );
            let a = (bc.a as f32 * alpha_mul) as u8;
            let color = Color::from_rgba8(bc.r, bc.g, bc.b, a);
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &r);
        }
        if let Some((s_top, s_right, s_bottom, s_left)) = per_side_border {
            // Per-side: pintamos cada lado como una línea recta del color
            // y grosor correspondientes. Corners en chaflán cuadrado —
            // matchea el look de browsers cuando border-{top,right,...}
            // difieren entre sí.
            let x0 = rect.x as f64;
            let y0 = rect.y as f64;
            let x1 = x0 + rect.w as f64;
            let y1 = y0 + rect.h as f64;
            // Cada lado se inseta por w/2 para que el trazo caiga dentro
            // del rect del nodo (vello pinta centrado al path). Pintamos
            // inline (sin closure) para evitar capturas raras del scene.
            if let Some((c, w)) = s_top {
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x0, y0 + h), (x1, y0 + h));
                scene.stroke(&Stroke::new(w as f64), Affine::IDENTITY, color, None, &line);
            }
            if let Some((c, w)) = s_bottom {
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x0, y1 - h), (x1, y1 - h));
                scene.stroke(&Stroke::new(w as f64), Affine::IDENTITY, color, None, &line);
            }
            if let Some((c, w)) = s_left {
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x0 + h, y0), (x0 + h, y1));
                scene.stroke(&Stroke::new(w as f64), Affine::IDENTITY, color, None, &line);
            }
            if let Some((c, w)) = s_right {
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x1 - h, y0), (x1 - h, y1));
                scene.stroke(&Stroke::new(w as f64), Affine::IDENTITY, color, None, &line);
            }
        }
        if let Some((oc, ow, off)) = outline {
            let stroke = Stroke::new(ow as f64);
            let half = stroke.width * 0.5;
            // outline se dibuja FUERA del border, separado por `offset`.
            let outset = (off as f64) + half;
            let r = RoundedRect::new(
                rect.x as f64 - outset,
                rect.y as f64 - outset,
                (rect.x + rect.w) as f64 + outset,
                (rect.y + rect.h) as f64 + outset,
                radius + outset,
            );
            let a = (oc.a as f32 * alpha_mul) as u8;
            let color = Color::from_rgba8(oc.r, oc.g, oc.b, a);
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &r);
        }
        if let Some((line_kind, c, font_size)) = deco {
            // Posición vertical relativa al rect (sin baseline real). El
            // rect del leaf de texto tiene height = font_size * line_height
            // (≈1.4 default), así que el texto vive arriba-centro:
            //   overline    → top + line_height*0.10
            //   line-through → mid (≈ 0.55)
            //   underline   → ~ baseline (≈ 0.85)
            let y_frac = match line_kind {
                TextDecorationLine::Overline => 0.10,
                TextDecorationLine::LineThrough => 0.55,
                TextDecorationLine::Underline => 0.88,
                TextDecorationLine::None => return,
            };
            let y = rect.y as f64 + rect.h as f64 * y_frac;
            let thickness = ((font_size * 0.07) as f64).max(1.0);
            let stroke = Stroke::new(thickness);
            let dec_color = Color::from_rgba8(c.r, c.g, c.b, 255);
            scene.stroke(
                &stroke,
                Affine::IDENTITY,
                dec_color,
                None,
                &Line::new((rect.x as f64, y), ((rect.x + rect.w) as f64, y)),
            );
        }
    })
}

pub(crate) fn box_style(b: &BoxNode, zoom: f32) -> Style {
    // Las hojas de texto se miden con parley en el runtime
    // (`compute_with_measure`): taffy reserva el alto real del texto
    // envuelto (N líneas) en lugar de una sola. Por eso dejamos su height
    // en `auto` — si lo fijáramos a una línea, los párrafos que envuelven
    // se aplastarían unos sobre otros. Mantenemos `line_h` como piso
    // (min_height) para que un nodo de texto vacío no colapse a cero.
    let is_text_leaf = b.text.is_some();
    let lh_mult = b.line_height.unwrap_or(1.2);
    let line_h = b.font_size * lh_mult * zoom;

    let is_flex = matches!(b.display, Display::Flex | Display::InlineFlex);

    let is_grid = matches!(b.display, Display::Grid | Display::InlineGrid);

    // Defaults según display: Block fila completa columnar, Inline en row
    // con altura auto, Flex toma sus props del nodo. None: cero.
    let (default_direction, mut width, mut height) = match b.display {
        Display::Block => (FlexDirection::Column, percent(1.0_f32), auto()),
        Display::Flex => (map_flex_direction(b.flex_direction), percent(1.0_f32), auto()),
        Display::InlineFlex => (map_flex_direction(b.flex_direction), auto(), auto()),
        Display::Grid => (FlexDirection::Column, percent(1.0_f32), auto()),
        Display::InlineGrid => (FlexDirection::Column, auto(), auto()),
        Display::InlineBlock | Display::Inline => {
            // Texto: height auto → lo dimensiona la medición con parley.
            (FlexDirection::Row, auto(), auto())
        }
        Display::None => (FlexDirection::Column, length(0.0_f32), length(0.0_f32)),
    };

    // Para bloques con hijos inline conmutamos a Row + Wrap (igual que
    // antes — el hack original que hace que `<p>` flowee tokens). Para
    // Flex respetamos las props del autor sin tocar.
    let block_inline_wrap =
        matches!(b.display, Display::Block) && has_inline_children(b);

    let flex_wrap = if is_flex {
        map_flex_wrap(b.flex_wrap)
    } else if block_inline_wrap {
        FlexWrap::Wrap
    } else {
        FlexWrap::NoWrap
    };

    let (flex_direction, w_base) = if block_inline_wrap {
        (FlexDirection::Row, percent(1.0_f32))
    } else {
        (default_direction, width)
    };
    width = w_base;

    // CSS `width` explícito gana sobre el default de display.
    if let Some(explicit) = length_to_taffy(b.width, zoom) {
        width = explicit;
    }
    // CSS `height` explícito gana sobre el default (auto = lo dimensiona el
    // contenido). Los % de height sólo resuelven si el padre tiene altura
    // definida — taffy lo maneja igual que en un browser.
    if let Some(explicit) = length_to_taffy(b.height, zoom) {
        height = explicit;
    }
    let max_size = Size {
        width: length_to_taffy(b.max_width, zoom).unwrap_or_else(auto),
        height: length_to_taffy(b.max_height, zoom).unwrap_or_else(auto),
    };
    let min_size = Size {
        width: length_to_taffy(b.min_width, zoom).unwrap_or_else(|| length(0.0_f32)),
        height: length_to_taffy(b.min_height, zoom).unwrap_or_else(|| {
            // Piso de una línea para hojas de texto (el resto: 0).
            if is_text_leaf { length(line_h) } else { length(0.0_f32) }
        }),
    };

    // justify/align: si es flex, vienen del autor; sino, sólo derivamos
    // `justify_content` de `text-align` sobre bloques con inlines (el
    // viejo comportamiento heredado).
    let justify_content = if is_flex {
        Some(map_justify(b.justify_content))
    } else if block_inline_wrap {
        match b.text_align {
            TextAlign::Left | TextAlign::Justify => None,
            TextAlign::Center => Some(JustifyContent::Center),
            TextAlign::Right => Some(JustifyContent::End),
        }
    } else {
        None
    };

    let align_items = if is_flex {
        Some(map_align(b.align_items))
    } else {
        None
    };

    // align-content: distribución de líneas (flex multilínea) / pistas
    // (grid) en el eje cruzado. Aplica tanto a flex como a grid; `Normal`
    // deja el default de taffy (None ≈ stretch).
    let align_content = if is_flex || is_grid {
        map_align_content(b.align_content)
    } else {
        None
    };

    // gap: aplica a flex (y a futuros grid). Taffy lo expone como
    // `Size { width: column-gap, height: row-gap }`.
    let gap = if is_flex {
        Size {
            width: length(b.gap_column * zoom),
            height: length(b.gap_row * zoom),
        }
    } else {
        Size { width: length(0.0_f32), height: length(0.0_f32) }
    };

    // box-sizing default CSS = ContentBox; los resets modernos lo
    // fuerzan a BorderBox. Taffy 0.9 default es BorderBox así que
    // mapeamos explícito en ambos sentidos.
    let box_sizing = match b.box_sizing {
        CssBoxSizing::ContentBox => BoxSizing::ContentBox,
        CssBoxSizing::BorderBox => BoxSizing::BorderBox,
    };
    // vertical-align mapea a align_self (con prioridad sobre el de
    // align-self CSS) cuando es inline/inline-block — no es lo mismo en
    // CSS spec pero alcanza para el subset que nos importa.
    let align_self = match b.vertical_align {
        VerticalAlign::Baseline => map_align_self(b.align_self),
        VerticalAlign::Top => Some(AlignSelf::Start),
        VerticalAlign::Middle => Some(AlignSelf::Center),
        VerticalAlign::Bottom | VerticalAlign::Sub => Some(AlignSelf::End),
        VerticalAlign::Super => Some(AlignSelf::Start),
    };
    let flex_basis: Dimension = length_to_taffy(b.flex_basis, zoom).unwrap_or_else(auto);

    // Position + insets (top/right/bottom/left).
    let position_kind = match b.position {
        CssPosition::Static => TaffyPosition::Relative, // = layout normal
        CssPosition::Relative | CssPosition::Sticky => TaffyPosition::Relative,
        CssPosition::Absolute | CssPosition::Fixed => TaffyPosition::Absolute,
    };
    let inset = Rect {
        top: length_to_inset(b.inset_top, zoom),
        right: length_to_inset(b.inset_right, zoom),
        bottom: length_to_inset(b.inset_bottom, zoom),
        left: length_to_inset(b.inset_left, zoom),
    };

    // Taffy Display: Block/Flex/Grid/None. Inline/InlineBlock las
    // tratamos como Flex (row) por las hacks de inlines.
    let taffy_display = match b.display {
        Display::None => TaffyDisplay::None,
        Display::Grid | Display::InlineGrid => TaffyDisplay::Grid,
        _ => TaffyDisplay::Flex,
    };

    // Grid templates — sólo se aplican si display es grid. Las pistas Px
    // se escalan con zoom; fr/auto/pct quedan intactas.
    let grid_template_columns: Vec<GridTemplateComponent<String>> =
        if is_grid { b.grid_template_columns.iter().map(|t| map_grid_track(t, zoom)).collect() } else { Vec::new() };
    let grid_template_rows: Vec<GridTemplateComponent<String>> =
        if is_grid { b.grid_template_rows.iter().map(|t| map_grid_track(t, zoom)).collect() } else { Vec::new() };

    Style {
        display: taffy_display,
        flex_direction,
        flex_wrap,
        justify_content,
        align_items,
        align_content,
        // justify-items / justify-self: taffy sólo los usa en grid (los
        // ignora en flex). `None`/`Auto` → default de taffy.
        justify_items: b.justify_items.map(map_align),
        justify_self: map_align_self(b.justify_self),
        align_self,
        flex_grow: b.flex_grow,
        flex_shrink: b.flex_shrink,
        flex_basis,
        box_sizing,
        position: position_kind,
        inset,
        gap,
        size: Size { width, height },
        min_size,
        max_size,
        // CSS aspect-ratio: taffy dimensiona el eje `auto` a partir del otro
        // usando esta relación. `None` = sin relación.
        aspect_ratio: b.aspect_ratio,
        margin: Rect {
            left: margin_left_lpa(b, zoom),
            right: margin_right_lpa(b, zoom, 0.0),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        padding: Rect {
            left: length(b.padding.left * zoom),
            right: length(b.padding.right * zoom),
            top: length(b.padding.top * zoom),
            bottom: length(b.padding.bottom * zoom),
        },
        grid_template_columns: grid_template_columns.into(),
        grid_template_rows: grid_template_rows.into(),
        ..Default::default()
    }
}

pub(crate) fn map_grid_track(t: &GridTrackSize, zoom: f32) -> GridTemplateComponent<String> {
    let single: TrackSizingFunction = match t {
        GridTrackSize::Auto => auto(),
        GridTrackSize::Px(v) => length(*v * zoom),
        GridTrackSize::Pct(v) => percent(*v / 100.0),
        GridTrackSize::Fr(v) => fr(*v),
    };
    GridTemplateComponent::Single(single)
}

/// `length-percentage-auto`: para insets (top/right/bottom/left) que
/// aceptan `auto` además de px/%. `zoom` escala sólo el valor Px;
/// los porcentajes se resuelven contra el contenedor (que también escala).
/// Margen izquierdo del box como `LengthPercentageAuto`, respetando
/// `margin-left: auto` (centrado horizontal → taffy `auto()`).
pub(crate) fn margin_left_lpa(b: &BoxNode, zoom: f32) -> LengthPercentageAuto {
    if b.margin_left_auto {
        auto()
    } else {
        length(b.margin.left * zoom)
    }
}

/// Margen derecho del box; `extra` suma px fijos (algunos sitios lo
/// necesitan) sólo cuando el lado NO es `auto`.
pub(crate) fn margin_right_lpa(b: &BoxNode, zoom: f32, extra: f32) -> LengthPercentageAuto {
    if b.margin_right_auto {
        auto()
    } else {
        length(b.margin.right * zoom + extra)
    }
}

pub(crate) fn length_to_inset(v: LengthVal, zoom: f32) -> LengthPercentageAuto {
    match v {
        LengthVal::Auto => auto(),
        LengthVal::Px(px) => length(px * zoom),
        LengthVal::Pct(pct) => percent(pct / 100.0),
    }
}

pub(crate) fn map_align_self(a: CssAlignSelf) -> Option<AlignSelf> {
    match a {
        CssAlignSelf::Auto => None,
        CssAlignSelf::Start => Some(AlignSelf::Start),
        CssAlignSelf::Center => Some(AlignSelf::Center),
        CssAlignSelf::End => Some(AlignSelf::End),
        CssAlignSelf::Stretch => Some(AlignSelf::Stretch),
        CssAlignSelf::Baseline => Some(AlignSelf::Baseline),
    }
}

/// Arma un `peniko::Gradient` desde un gradiente CSS contra el rect, según
/// `g.geometry`: lineal (segmento en la dirección CSS), radial (círculo
/// centrado en `at <pos>` con radio por `<size>`) o cónico (sweep `from
/// <angle> at <pos>`). Aplica `alpha_mul` (opacity) a cada stop. Devuelve
/// None si los stops no se pueden representar.
pub(crate) fn build_linear_gradient_brush(
    g: &LinearGradient,
    rect: llimphi_ui::PaintRect,
    alpha_mul: f32,
) -> Option<Gradient> {
    use puriy_engine::style::{GradientGeometry, LengthVal, RadialSize};
    if g.stops.len() < 2 {
        return None;
    }
    let w = rect.w as f64;
    let h = rect.h as f64;
    // Resuelve una posición CSS (`Pct` contra el span, `Px` crudo) a px de
    // pantalla. Compartido por el centro de radial y conic.
    let resolve_pos = |v: LengthVal, span: f64, origin: f64| -> f64 {
        match v {
            LengthVal::Pct(p) => origin + span * (p as f64) / 100.0,
            LengthVal::Px(px) => origin + px as f64,
            LengthVal::Auto => origin + span * 0.5,
        }
    };

    // Stops: si pos es None, distribuir uniformemente.
    let n = g.stops.len();
    let mut peniko_stops: Vec<ColorStop> = Vec::with_capacity(n);
    for (i, s) in g.stops.iter().enumerate() {
        let pos = s.pos.unwrap_or_else(|| {
            if n == 1 { 0.0 } else { i as f32 / (n - 1) as f32 }
        });
        let a = ((s.color.a as f32) * alpha_mul) as u8;
        let c = Color::from_rgba8(s.color.r, s.color.g, s.color.b, a);
        peniko_stops.push(ColorStop::from((pos, c)));
    }

    let kind = match &g.geometry {
        GradientGeometry::Radial(spec) => {
            let cxp = resolve_pos(spec.cx, w, rect.x as f64);
            let cyp = resolve_pos(spec.cy, h, rect.y as f64);
            // Distancias a lados y esquinas.
            let (dl, dr) = ((cxp - rect.x as f64).abs(), (rect.x as f64 + w - cxp).abs());
            let (dt, db) = ((cyp - rect.y as f64).abs(), (rect.y as f64 + h - cyp).abs());
            let corner = |x: f64, y: f64| ((cxp - x).powi(2) + (cyp - y).powi(2)).sqrt();
            let corners = [
                corner(rect.x as f64, rect.y as f64),
                corner(rect.x as f64 + w, rect.y as f64),
                corner(rect.x as f64, rect.y as f64 + h),
                corner(rect.x as f64 + w, rect.y as f64 + h),
            ];
            let fmin = |a: f64, b: f64| a.min(b);
            let fmax = |a: f64, b: f64| a.max(b);
            let radius = match spec.size {
                RadialSize::ClosestSide => dl.min(dr).min(dt).min(db),
                RadialSize::FarthestSide => dl.max(dr).max(dt).max(db),
                RadialSize::ClosestCorner => corners.iter().copied().fold(f64::MAX, fmin),
                RadialSize::FarthestCorner => corners.iter().copied().fold(0.0, fmax),
            }
            .max(1.0);
            let center = Point::new(cxp, cyp);
            GradientKind::Radial {
                start_center: center,
                start_radius: 0.0,
                end_center: center,
                end_radius: radius as f32,
            }
        }
        GradientGeometry::Conic { from_deg, cx, cy } => {
            // peniko Sweep: ángulos en radianes, 0 = +x (derecha), crece CW
            // en pantalla. CSS `from`: 0 = up, crece CW. Diferencia = -90°.
            let center = Point::new(
                resolve_pos(*cx, w, rect.x as f64),
                resolve_pos(*cy, h, rect.y as f64),
            );
            let start = (*from_deg - 90.0).to_radians();
            GradientKind::Sweep {
                center,
                start_angle: start,
                end_angle: start + std::f32::consts::TAU,
            }
        }
        GradientGeometry::Linear { angle_deg } => {
            // CSS: 0deg = up (negative y), 90 = right (+x), 180 = down (+y),
            // 270 = left (-x). Convertimos a radianes y dirección en espacio
            // de pantalla (y crece hacia abajo).
            let theta = angle_deg.to_radians();
            let dx = theta.sin() as f64;
            let dy = -theta.cos() as f64;
            let cx = rect.x as f64 + w * 0.5;
            let cy = rect.y as f64 + h * 0.5;
            let half_len = (dx.abs() * w + dy.abs() * h) * 0.5;
            let start = Point::new(cx - dx * half_len, cy - dy * half_len);
            let end = Point::new(cx + dx * half_len, cy + dy * half_len);
            GradientKind::Linear { start, end }
        }
    };

    Some(Gradient {
        kind,
        stops: ColorStops(peniko_stops.into()),
        ..Default::default()
    })
}

pub(crate) fn map_flex_direction(d: CssFlexDirection) -> FlexDirection {
    match d {
        CssFlexDirection::Row => FlexDirection::Row,
        CssFlexDirection::RowReverse => FlexDirection::RowReverse,
        CssFlexDirection::Column => FlexDirection::Column,
        CssFlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
    }
}

pub(crate) fn map_flex_wrap(w: CssFlexWrap) -> FlexWrap {
    match w {
        CssFlexWrap::NoWrap => FlexWrap::NoWrap,
        CssFlexWrap::Wrap => FlexWrap::Wrap,
        CssFlexWrap::WrapReverse => FlexWrap::WrapReverse,
    }
}

pub(crate) fn map_justify(j: CssJustifyContent) -> JustifyContent {
    match j {
        CssJustifyContent::Start => JustifyContent::Start,
        CssJustifyContent::Center => JustifyContent::Center,
        CssJustifyContent::End => JustifyContent::End,
        CssJustifyContent::SpaceBetween => JustifyContent::SpaceBetween,
        CssJustifyContent::SpaceAround => JustifyContent::SpaceAround,
        CssJustifyContent::SpaceEvenly => JustifyContent::SpaceEvenly,
    }
}

pub(crate) fn map_align(a: CssAlignItems) -> AlignItems {
    match a {
        CssAlignItems::Start => AlignItems::Start,
        CssAlignItems::Center => AlignItems::Center,
        CssAlignItems::End => AlignItems::End,
        CssAlignItems::Stretch => AlignItems::Stretch,
        CssAlignItems::Baseline => AlignItems::Baseline,
    }
}

/// `align-content` CSS → taffy. `Normal` ⇒ `None` (taffy aplica su default,
/// ≈ stretch para flex). `Start`/`End` mapean a `FlexStart`/`FlexEnd` para
/// que respeten la dirección flex (row-reverse, etc.).
pub(crate) fn map_align_content(a: CssAlignContent) -> Option<AlignContent> {
    match a {
        CssAlignContent::Normal => None,
        CssAlignContent::Start => Some(AlignContent::Start),
        CssAlignContent::Center => Some(AlignContent::Center),
        CssAlignContent::End => Some(AlignContent::End),
        CssAlignContent::Stretch => Some(AlignContent::Stretch),
        CssAlignContent::SpaceBetween => Some(AlignContent::SpaceBetween),
        CssAlignContent::SpaceAround => Some(AlignContent::SpaceAround),
        CssAlignContent::SpaceEvenly => Some(AlignContent::SpaceEvenly),
    }
}

/// Traduce un `LengthVal` CSS al tipo de longitud que taffy entiende.
/// `Auto` queda como `None` (caller lo reemplaza con el default según
/// display o `auto()` para max-size).
pub(crate) fn length_to_taffy(v: LengthVal, zoom: f32) -> Option<llimphi_layout::taffy::style::Dimension> {
    match v {
        LengthVal::Auto => None,
        LengthVal::Px(px) => Some(length(px * zoom)),
        LengthVal::Pct(pct) => Some(percent(pct / 100.0)),
    }
}

/// `true` si todos los hijos directos son inline o inline-block. Si los
/// hijos son block, el bloque sigue siendo column.
pub(crate) fn has_inline_children(b: &BoxNode) -> bool {
    !b.children.is_empty()
        && b.children
            .iter()
            .all(|c| matches!(c.display, Display::Inline | Display::InlineBlock))
}
