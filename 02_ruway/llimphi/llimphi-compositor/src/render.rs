use super::*;

pub fn mount<Msg: Clone>(layout: &mut LayoutTree, v: View<Msg>) -> Mounted<Msg> {
    let mut nodes = Vec::new();
    let mut text_measures = std::collections::HashMap::new();
    let root = mount_recursive(layout, v, &mut nodes, &mut text_measures);
    Mounted { root, nodes, text_measures }
}

/// Mount en pre-orden directo sobre `out`: pusheamos el padre como
/// placeholder (id real desconocido hasta crear el taffy node), recursamos
/// hijos sobre el mismo `out`, y al volver completamos `id` + `subtree_end`.
pub fn mount_recursive<Msg: Clone>(
    layout: &mut LayoutTree,
    v: View<Msg>,
    out: &mut Vec<MountedNode<Msg>>,
    text_measures: &mut std::collections::HashMap<NodeId, TextMeasure>,
) -> NodeId {
    let View {
        style,
        fill,
        hover_fill,
        radius,
        corner_radii,
        shadow,
        fill_gradient,
        border,
        text,
        image,
        image_fit,
        painter,
        gpu_painter,
        on_click,
        on_click_at,
        on_right_click,
        on_right_click_at,
        on_middle_click,
        drag,
        drag_at,
        drag_velocity,
        drag_payload,
        on_drop,
        drop_hover_fill,
        clip,
        clip_inset,
        clip_ellipse,
        clip_polygon,
        clip_path_svg,
        clip_ref_inset,
        on_pointer_enter,
        on_pointer_leave,
        on_pointer_move_at,
        on_scroll,
        on_scale,
        on_rotate,
        on_double_tap,
        on_double_tap_at,
        on_long_press,
        on_long_press_at,
        focusable,
        text_select_key,
        alpha,
        anim,
        animated_size,
        semantics,
        hero,
        transform,
        transform_rel,
        tooltip,
        cursor,
        ripple,
        layout_builder,
        backdrop_blur,
        children,
    } = v;
    let parent_idx = out.len();
    out.push(MountedNode {
        id: NodeId::new(0), // placeholder, lo sobreescribimos abajo
        fill,
        hover_fill,
        radius,
        corner_radii,
        shadow,
        fill_gradient,
        border,
        text,
        image,
        image_fit,
        painter,
        gpu_painter,
        on_click,
        on_click_at,
        on_right_click,
        on_right_click_at,
        on_middle_click,
        drag,
        drag_at,
        drag_velocity,
        drag_payload,
        on_drop,
        drop_hover_fill,
        clip,
        clip_inset,
        clip_ellipse,
        clip_polygon,
        clip_path_svg,
        clip_ref_inset,
        on_pointer_enter,
        on_pointer_leave,
        on_pointer_move_at,
        on_scroll,
        on_scale,
        on_rotate,
        on_double_tap,
        on_double_tap_at,
        on_long_press,
        on_long_press_at,
        focusable,
        text_select_key,
        alpha,
        anim,
        animated_size,
        semantics,
        hero,
        transform,
        transform_rel,
        tooltip,
        cursor,
        ripple,
        // Un layout_builder ya expandido llega como nodo normal; si llega sin
        // expandir (caller no pasó por el runtime), se monta como hoja y este
        // flag permite que el runtime lo detecte y resuelva.
        is_layout_builder: layout_builder.is_some(),
        backdrop_blur,
        subtree_end: 0,
    });
    let mut child_ids = Vec::with_capacity(children.len());
    for child in children {
        child_ids.push(mount_recursive(layout, child, out, text_measures));
    }
    let id = if child_ids.is_empty() {
        layout.leaf(style).expect("layout leaf")
    } else {
        layout.node(style, &child_ids).expect("layout node")
    };
    out[parent_idx].id = id;
    out[parent_idx].subtree_end = out.len();
    // Hoja de texto uniforme: registrá su contenido para que el runtime lo
    // mida con parley. El texto multicolor (`runs`) lo dimensiona el caller
    // (editor: un nodo por línea), así que no lo medimos acá.
    if child_ids.is_empty() {
        if let Some(text) = out[parent_idx].text.as_ref() {
            if text.runs.is_none() {
                text_measures.insert(
                    id,
                    TextMeasure {
                        content: text.content.clone(),
                        size_px: text.size_px,
                        alignment: text.alignment,
                        italic: text.italic,
                        font_family: text.font_family.clone(),
                        line_height: text.line_height,
                        weight: text.weight,
                        max_lines: text.max_lines,
                        ellipsis: text.ellipsis,
                        underline: text.underline,
                        strikethrough: text.strikethrough,
                        spans: text.spans.clone(),
                    },
                );
            }
        }
    }
    id
}

/// Mide una hoja de texto para taffy: shaping + line-break con parley contra
/// el ancho disponible, devolviendo el bounding box. Si el ancho ya está
/// resuelto (`known.width`) se usa ese; si no, se deriva del `available`
/// (Definite → ese ancho; MaxContent → sin límite = una línea; MinContent →
/// 0 = envuelve a la palabra más ancha). El `line_height` sale del propio
/// `TextMeasure`, el mismo que usa `paint`, así medida y pintado coinciden.
pub fn measure_text_node(
    ts: &mut llimphi_text::Typesetter,
    tm: &TextMeasure,
    known: llimphi_layout::taffy::Size<Option<f32>>,
    available: llimphi_layout::taffy::Size<llimphi_layout::taffy::AvailableSpace>,
) -> llimphi_layout::taffy::Size<f32> {
    use llimphi_layout::taffy::AvailableSpace;
    let max_width: Option<f32> = known.width.or(match available.width {
        AvailableSpace::Definite(w) => Some(w),
        AvailableSpace::MaxContent => None,
        AvailableSpace::MinContent => Some(0.0),
    });
    // RichText: si hay spans, mediar con `layout_spans` para que taffy
    // reserve el alto considerando overrides de tamaño por rango (un span
    // con `size_px = 24` dentro de un párrafo de 14 px agranda esa línea).
    // El clamp `max_lines`/`ellipsis` no se aplica al camino spans en v1
    // (RichText típico no clampea — los headings y links viven el bloque
    // completo); el caller que necesite clamp con spans puede recortar el
    // texto antes de pasarlo.
    if let Some(spans) = tm.spans.as_ref() {
        if !spans.is_empty() {
            let layout = ts.layout_spans(
                &tm.content,
                tm.size_px,
                vello::peniko::Color::from_rgba8(0, 0, 0, 255),
                tm.weight,
                tm.line_height,
                tm.italic,
                tm.font_family.as_deref(),
                tm.underline,
                tm.strikethrough,
                spans,
                max_width,
                tm.alignment,
            );
            return llimphi_layout::taffy::Size {
                width: layout.width(),
                height: layout.height(),
            };
        }
    }
    // Camino directo a `layout_clamped` (no `TextBlock`) para transportar
    // `weight` (bold mide más ancho) y `max_lines` (taffy reserva el alto de
    // N líneas, no el del texto completo). Sin clamp, equivale a `layout`.
    let layout = ts.layout_clamped(
        &tm.content,
        tm.size_px,
        max_width,
        tm.alignment,
        tm.line_height,
        tm.italic,
        tm.font_family.as_deref(),
        tm.weight,
        tm.max_lines,
        tm.ellipsis,
        tm.underline,
        tm.strikethrough,
    );
    let m = llimphi_text::measurement(&layout);
    llimphi_layout::taffy::Size { width: m.width, height: m.height }
}

/// Construye el `RoundedRect` del nodo respetando radio por esquina si lo
/// hay (si no, el escalar uniforme), con un `inset` opcional restado al rect
/// y a cada radio (lo usa el borde, que pinta media línea hacia adentro).
pub(crate) fn node_rrect(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    radius: f64,
    corners: Option<RoundedRectRadii>,
    inset: f64,
) -> RoundedRect {
    let radii = match corners {
        Some(c) => RoundedRectRadii::new(
            (c.top_left - inset).max(0.0),
            (c.top_right - inset).max(0.0),
            (c.bottom_right - inset).max(0.0),
            (c.bottom_left - inset).max(0.0),
        ),
        None => {
            let r = (radius - inset).max(0.0);
            RoundedRectRadii::new(r, r, r, r)
        }
    };
    RoundedRect::new(x0 + inset, y0 + inset, x1 - inset, y1 - inset, radii)
}

/// Resuelve un radio de `clip-path: circle()/ellipse()` a px, dado su
/// quíntuple `[px, pct_w, pct_h, pct_diag, side]`, el centro local `(cxl,
/// cyl)` (relativo al origen del rect), el tamaño `(w, h)` y si el radio es
/// del eje X (`is_x`). Con `side == 0` suma px + porcentajes (diag =
/// √(w²+h²)/√2). Con `side != 0` ignora px/pct y mide la distancia del centro
/// a los bordes: `1`/`2` = closest/farthest sobre los 4 lados (circle);
/// `3`/`4` = ídem sobre el eje del radio (ellipse). Fase 7.1222.
fn resolve_clip_radius(q: &[f32], cxl: f64, cyl: f64, w: f64, h: f64, is_x: bool) -> f64 {
    let side = q[4] as i32;
    if side == 0 {
        let diag = (w * w + h * h).sqrt() / core::f64::consts::SQRT_2;
        return q[0] as f64 + q[1] as f64 / 100.0 * w + q[2] as f64 / 100.0 * h
            + q[3] as f64 / 100.0 * diag;
    }
    let (dx_near, dx_far) = (cxl.min(w - cxl), cxl.max(w - cxl));
    let (dy_near, dy_far) = (cyl.min(h - cyl), cyl.max(h - cyl));
    match side {
        1 => dx_near.min(dy_near), // closest-side, circle (4 lados)
        2 => dx_far.max(dy_far),   // farthest-side, circle
        3 => {
            if is_x {
                dx_near
            } else {
                dy_near
            }
        } // closest-side, ellipse (eje)
        _ => {
            if is_x {
                dx_far
            } else {
                dy_far
            }
        } // 4 = farthest-side, ellipse
    }
}

pub fn paint<Msg>(
    scene: &mut vello::Scene,
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    typesetter: &mut llimphi_text::Typesetter,
    hover_idx: Option<usize>,
    drop_hover_idx: Option<usize>,
) {
    paint_range(
        scene,
        mounted,
        computed,
        typesetter,
        hover_idx,
        drop_hover_idx,
        0,
        mounted.nodes.len(),
        Affine::IDENTITY,
    );
}

/// Recolecta los nodos con [`MountedNode::backdrop_blur`] activo del árbol
/// montado, junto con el sigma y el rect absoluto al cual restringir el
/// blur. El runtime (`llimphi-ui::eventloop`) los aplica como post-pasada
/// **después** de la rasterización vello, sobre la intermediate.
///
/// La búsqueda **salta el subárbol** al encontrar un blur — sin anidamiento
/// en v1: un blur dentro de otro blur sería redundante (el padre ya borrona
/// el rect que cubre al hijo).
///
/// **Limitación v1 (post-pasada)**: el blur ocurre tras vello, así que el
/// fill/text/imagen del nodo blur y sus descendientes — pintados antes en
/// la misma rasterización — quedan **borroneados** también. Útil para
/// paneles "vidrio sobre fondo" sin contenido propio (el contenido nítido
/// se compone como nodo hermano posterior con el mismo rect). La paridad
/// completa con CSS `backdrop-filter` requiere scene-split (Bloque 11.B
/// del roadmap).
pub fn collect_backdrop_blurs<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
) -> Vec<BackdropBlur> {
    let mut out = Vec::new();
    let mut idx = 0;
    while idx < mounted.nodes.len() {
        let node = &mounted.nodes[idx];
        if let Some(sigma) = node.backdrop_blur {
            if let Some(r) = computed.get(node.id) {
                out.push(BackdropBlur {
                    sigma,
                    rect: (r.x, r.y, r.w, r.h),
                });
                idx = node.subtree_end;
                continue;
            }
        }
        idx += 1;
    }
    out
}

/// Datos de un backdrop blur listos para que el runtime lo aplique sobre
/// la intermediate vía `llimphi_hal::BlurCompositor::blur`.
#[derive(Debug, Clone, Copy)]
pub struct BackdropBlur {
    /// Sigma del Gauss en pixels lógicos.
    pub sigma: f32,
    /// Rect absoluto `(x, y, w, h)` del nodo, en pixels lógicos del viewport.
    pub rect: (f32, f32, f32, f32),
}

/// Resuelve el afín efectivo de un nodo a partir de su `transform` (afín fijo)
/// y/o `transform_rel` (traslación en fracción de su tamaño), centrado por
/// `transform-origin: 50% 50%` contra su rect computado `r`. El `transform_rel`
/// entra como factor más externo (`T_rel · transform`), igual que un
/// `translate(<%>)` al frente de la lista CSS. `None` si el nodo no tiene
/// ninguno de los dos (caso mayoritario → no se toca el stack de transform).
/// Lo usan `paint_range` y los walks de hit-test para mantenerse en sincronía.
pub(crate) fn resolve_node_transform(
    transform: Option<Affine>,
    transform_rel: Option<(f64, f64)>,
    r: llimphi_layout::Rect,
) -> Option<Affine> {
    if transform.is_none() && transform_rel.is_none() {
        return None;
    }
    let mut local = transform.unwrap_or(Affine::IDENTITY);
    if let Some((fx, fy)) = transform_rel {
        local = Affine::translate((fx * r.w as f64, fy * r.h as f64)) * local;
    }
    let cx = (r.x + r.w * 0.5) as f64;
    let cy = (r.y + r.h * 0.5) as f64;
    Some(Affine::translate((cx, cy)) * local * Affine::translate((-cx, -cy)))
}

/// Pinta el rango de nodos `[start, end)` de `mounted` en `scene`, partiendo de
/// la transformación acumulada `base_xf`. [`paint`] lo llama con todo el árbol
/// (`0..len`, `IDENTITY`). El rango permite **capturar un subárbol** en una
/// escena aparte (p. ej. el snapshot de un nodo que va a animar su salida, ver
/// [`crate::AnimRegistry`]): se pasa `(start, subtree_end)` del nodo raíz. Las
/// coordenadas de los rects ya son absolutas, así que la subescena se puede
/// reproducir luego con `scene.append` aunque sus ancestros ya no existan.
///
/// Las capas (clip/alpha) que el subárbol abre se cierran dentro del rango (su
/// `subtree_end ≤ end`) o por el drenaje final — la LIFO se respeta. `base_xf`
/// debería ser la transformación de los ancestros del nodo raíz; al capturar
/// se pasa `IDENTITY` (v1 no contempla raíces bajo ancestros transformados).
#[allow(clippy::too_many_arguments)]
pub fn paint_range<Msg>(
    scene: &mut vello::Scene,
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    typesetter: &mut llimphi_text::Typesetter,
    hover_idx: Option<usize>,
    drop_hover_idx: Option<usize>,
    start: usize,
    end: usize,
    base_xf: Affine,
) {
    // Stack de subtree_end de los `push_layer` activos (clip y/o alpha).
    // Vello requiere pop_layer en orden LIFO estricto, así que mantenemos
    // un único stack común y popeamos en el orden en que se pushearon.
    // Dos entradas con el mismo `subtree_end` (alpha + clip sobre el
    // mismo nodo) se cierran en el orden inverso al push.
    let mut layer_stack: Vec<usize> = Vec::new();
    // Stack de transformaciones afines de subtree. Cada entrada guarda el
    // `subtree_end` y la `cur_xf` previa para restaurarla al salir del
    // subárbol. `cur_xf` es el producto acumulado de todos los `transform`
    // de los ancestros activos — se multiplica en cada draw call. Cuando
    // ningún nodo transforma, queda en `base_xf` y el paint es idéntico
    // al previo (cero regresión).
    let mut xf_stack: Vec<(usize, Affine)> = Vec::new();
    let mut cur_xf = base_xf;
    for idx in start..end {
        let node = &mounted.nodes[idx];
        // Cierre de capas que ya quedaron atrás (idx ≥ subtree_end).
        while let Some(&end) = layer_stack.last() {
            if idx >= end {
                scene.pop_layer();
                layer_stack.pop();
            } else {
                break;
            }
        }
        // Restaurá la transformación al salir de subárboles transformados.
        while let Some(&(end, prev)) = xf_stack.last() {
            if idx >= end {
                cur_xf = prev;
                xf_stack.pop();
            } else {
                break;
            }
        }
        let Some(r) = computed.get(node.id) else {
            continue;
        };
        // Transform CSS del nodo: se aplica alrededor del centro de su rect
        // (`transform-origin: 50% 50%`) y se compone sobre la del padre. Se
        // empuja ANTES del alpha/fill para que toda la pintura del subtree
        // (incl. la capa de alpha y el clip) caiga en el espacio transformado.
        if let Some(centered) = resolve_node_transform(node.transform, node.transform_rel, r) {
            xf_stack.push((node.subtree_end, cur_xf));
            cur_xf *= centered;
        }
        // Alpha de subtree: push ANTES de cualquier paint de este nodo
        // para que fill/text/image/painter/children entren en la misma
        // capa y se compongan juntos al alfa indicado. Si el nodo tiene
        // hijos, su `subtree_end > idx + 1` y la capa permanece abierta
        // hasta que el loop alcance el primer índice fuera del subárbol.
        // Para nodos hoja con alpha el push y el pop son consecutivos —
        // funcionalmente equivalente a multiplicar el alpha del fill,
        // pero permite usar el mismo API sin distinguir hoja vs rama.
        if let Some(a) = node.alpha {
            let rect = KurboRect::new(
                r.x as f64,
                r.y as f64,
                (r.x + r.w) as f64,
                (r.y + r.h) as f64,
            );
            scene.push_layer(Fill::NonZero, Mix::Normal, a, cur_xf, &rect);
            layer_stack.push(node.subtree_end);
        }
        // Sombra (drop shadow): se pinta ANTES del relleno para quedar
        // detrás. Usa el blur gaussiano nativo de vello sobre un rect
        // redondeado offseteado + inflado por `spread`.
        if let Some(sh) = node.shadow.as_ref() {
            if sh.color.components[3] > 0.0 && r.w > 0.0 && r.h > 0.0 {
                let rect = KurboRect::new(
                    (r.x as f64) + sh.dx - sh.spread,
                    (r.y as f64) + sh.dy - sh.spread,
                    (r.x + r.w) as f64 + sh.dx + sh.spread,
                    (r.y + r.h) as f64 + sh.dy + sh.spread,
                );
                let radius = (node.radius + sh.spread).max(0.0);
                scene.draw_blurred_rounded_rect(cur_xf, rect, sh.color, radius, sh.blur);
            }
        }
        // Prioridad de pintura: drop-hover (drag activo) > hover normal >
        // gradiente base > fill color base. Solo aplica el override si el
        // slot correspondiente está poblado; el siguiente cae como fallback.
        let hover_color = if Some(idx) == drop_hover_idx {
            node.drop_hover_fill.or(node.hover_fill).or(node.fill)
        } else if Some(idx) == hover_idx {
            node.hover_fill.or(node.fill)
        } else {
            None
        };
        let rr = node_rrect(
            r.x as f64,
            r.y as f64,
            (r.x + r.w) as f64,
            (r.y + r.h) as f64,
            node.radius,
            node.corner_radii,
            0.0,
        );
        if let Some(color) = hover_color {
            // Hover/drop gana sobre el gradiente y el fill base.
            scene.fill(Fill::NonZero, cur_xf, color, None, &rr);
        } else if let Some(grad) = node.fill_gradient.as_ref() {
            // Gradiente autoreado en `[0,1]²`, mapeado al rect vía
            // brush_transform (incluye la transformación acumulada).
            let brush_xf = cur_xf
                * Affine::translate((r.x as f64, r.y as f64))
                * Affine::scale_non_uniform(r.w as f64, r.h as f64);
            scene.fill(Fill::NonZero, cur_xf, grad, Some(brush_xf), &rr);
        } else if let Some(color) = node.fill {
            scene.fill(Fill::NonZero, cur_xf, color, None, &rr);
        }
        // Borde (stroke) sobre el relleno, inset media línea hacia adentro.
        if let Some(b) = node.border.as_ref() {
            if b.width > 0.0 && b.color.components[3] > 0.0 && r.w > 0.0 && r.h > 0.0 {
                let inset = b.width * 0.5;
                let brr = node_rrect(
                    r.x as f64,
                    r.y as f64,
                    (r.x + r.w) as f64,
                    (r.y + r.h) as f64,
                    node.radius,
                    node.corner_radii,
                    inset,
                );
                scene.stroke(&Stroke::new(b.width), cur_xf, b.color, None, &brr);
            }
        }
        if let Some(image) = node.image.as_ref() {
            // Encaje seleccionable (Bloque 12) — Contain/Cover/Fill/None.
            // Siempre clippeamos al `node_rrect` para respetar
            // `radius`/`corner_radii` (avatares + cards) y para que
            // `Cover`/`None` no derramen fuera del nodo.
            if image.image.width > 0 && image.image.height > 0 && r.w > 0.0 && r.h > 0.0 {
                let sx = r.w as f64 / image.image.width as f64;
                let sy = r.h as f64 / image.image.height as f64;
                let fit = node.image_fit.unwrap_or(ImageFit::Contain);
                let transform = match fit {
                    ImageFit::Contain => {
                        let s = sx.min(sy);
                        let disp_w = image.image.width as f64 * s;
                        let disp_h = image.image.height as f64 * s;
                        let tx = r.x as f64 + (r.w as f64 - disp_w) * 0.5;
                        let ty = r.y as f64 + (r.h as f64 - disp_h) * 0.5;
                        Affine::translate((tx, ty)) * Affine::scale(s)
                    }
                    ImageFit::Cover => {
                        let s = sx.max(sy);
                        let disp_w = image.image.width as f64 * s;
                        let disp_h = image.image.height as f64 * s;
                        let tx = r.x as f64 + (r.w as f64 - disp_w) * 0.5;
                        let ty = r.y as f64 + (r.h as f64 - disp_h) * 0.5;
                        Affine::translate((tx, ty)) * Affine::scale(s)
                    }
                    ImageFit::Fill => {
                        Affine::translate((r.x as f64, r.y as f64))
                            * Affine::scale_non_uniform(sx, sy)
                    }
                    ImageFit::None => {
                        let disp_w = image.image.width as f64;
                        let disp_h = image.image.height as f64;
                        let tx = r.x as f64 + (r.w as f64 - disp_w) * 0.5;
                        let ty = r.y as f64 + (r.h as f64 - disp_h) * 0.5;
                        Affine::translate((tx, ty))
                    }
                };
                let clip_rr = node_rrect(
                    r.x as f64,
                    r.y as f64,
                    (r.x + r.w) as f64,
                    (r.y + r.h) as f64,
                    node.radius,
                    node.corner_radii,
                    0.0,
                );
                scene.push_layer(Fill::NonZero, BlendMode::default(), 1.0, cur_xf, &clip_rr);
                scene.draw_image(image, cur_xf * transform);
                scene.pop_layer();
            }
        }
        if let Some(painter) = node.painter.as_ref() {
            (painter)(
                scene,
                typesetter,
                PaintRect {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                },
            );
        }
        if let Some(text) = node.text.as_ref() {
            let has_spans = text
                .spans
                .as_ref()
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if has_spans {
                // RichText (Bloque 13): defaults a nivel bloque + spans
                // sobreescriben size/weight/italic/family/color/underline/
                // strikethrough por rango de bytes. Respeta `max_width = r.w`
                // (wrap a párrafo) y la alignment del bloque; para Center
                // también centramos verticalmente como en el camino uniforme.
                let spans = text.spans.as_ref().unwrap();
                let layout = typesetter.layout_spans(
                    &text.content,
                    text.size_px,
                    text.color,
                    text.weight,
                    text.line_height,
                    text.italic,
                    text.font_family.as_deref(),
                    text.underline,
                    text.strikethrough,
                    spans,
                    Some(r.w),
                    text.alignment,
                );
                let origin =
                    if matches!(text.alignment, llimphi_text::Alignment::Center) {
                        let lh = layout.height() as f64;
                        (
                            r.x as f64,
                            r.y as f64 + ((r.h as f64 - lh) * 0.5).max(0.0),
                        )
                    } else {
                        (r.x as f64, r.y as f64)
                    };
                llimphi_text::draw_layout_runs_xf(
                    scene,
                    &layout,
                    cur_xf * Affine::translate(origin),
                );
            } else if let Some(runs) = text.runs.as_ref() {
                // Texto multicolor (syntax highlighting): una sola pasada de
                // shaping con color por rango, anclado arriba-izquierda. Cae
                // por el flujo normal (clip/alpha se cierran como siempre).
                let layout = typesetter.layout_runs(
                    &text.content,
                    text.size_px,
                    text.color,
                    runs,
                    text.alignment,
                    text.line_height,
                    text.weight,
                    text.underline,
                    text.strikethrough,
                );
                // `cur_xf *` para que el texto multicolor herede la
                // transformación del subárbol (scroll/rotación del padre), igual
                // que el texto de color único de abajo. Sin esto se pintaba en
                // coords de layout crudas y se desalineaba al scrollear.
                llimphi_text::draw_layout_runs_xf(
                    scene,
                    &layout,
                    cur_xf * Affine::translate((r.x as f64, r.y as f64)),
                );
            } else {
                // Parley resuelve la alineación horizontal vía max_width +
                // alignment. Para Center también centramos verticalmente; para
                // Start/End/Justify anclamos arriba (párrafo/editor). Camino
                // directo a `layout_clamped` para transportar `weight` y el
                // clamp de `max_lines`/`ellipsis` del TextSpec.
                let layout = typesetter.layout_clamped(
                    &text.content,
                    text.size_px,
                    Some(r.w),
                    text.alignment,
                    text.line_height,
                    text.italic,
                    text.font_family.as_deref(),
                    text.weight,
                    text.max_lines,
                    text.ellipsis,
                    text.underline,
                    text.strikethrough,
                );
                let origin =
                    if matches!(text.alignment, llimphi_text::Alignment::Center) {
                        let m = llimphi_text::measurement(&layout);
                        (
                            r.x as f64,
                            r.y as f64 + ((r.h - m.height) as f64 * 0.5).max(0.0),
                        )
                    } else {
                        (r.x as f64, r.y as f64)
                    };
                llimphi_text::draw_layout_xf(
                    scene,
                    &layout,
                    text.color,
                    cur_xf * Affine::translate(origin),
                );
            }
        }
        if node.clip {
            // El hit-test (más abajo) usa siempre el rect completo — el clip-path
            // sólo afecta el pintado, una aproximación menor en su banda.
            // Prioridad: path > polygon > elipse > inset/rect. `pushed` queda
            // false sólo si un path() no parsea (no se abre capa → no se cierra).
            let mut pushed = true;
            // Caja de referencia (clip-path geometry-box, Fase 7.1225): encoge
            // el rect del nodo por `clip_ref_inset` ANTES de resolver la forma,
            // así circle/ellipse/polygon/path y sus % se miden contra esa caja.
            let [rit, rir, rib, ril] = node.clip_ref_inset.unwrap_or([0.0; 4]);
            let (bx, by) = ((r.x + ril) as f64, (r.y + rit) as f64);
            let (bw, bh) = ((r.w - ril - rir).max(0.0) as f64, (r.h - rit - rib).max(0.0) as f64);
            if let Some((evenodd, d)) = &node.clip_path_svg {
                // `clip-path: path()` — parsea el SVG y lo traslada al origen
                // de la caja de referencia (user units px). from_svg falla → no
                // recorta.
                match vello::kurbo::BezPath::from_svg(d) {
                    Ok(mut path) => {
                        path.apply_affine(Affine::translate((bx, by)));
                        let fill = if *evenodd { Fill::EvenOdd } else { Fill::NonZero };
                        scene.push_layer(fill, BlendMode::default(), 1.0, cur_xf, &path);
                    }
                    Err(_) => pushed = false,
                }
            } else if let Some((evenodd, pts)) = &node.clip_polygon {
                // `clip-path: polygon()` — capa con un path cerrado. Cada punto
                // resuelve sus % contra la caja de referencia; move_to al 1º,
                // line_to al resto, close_path.
                let mut path = vello::kurbo::BezPath::new();
                for (i, p) in pts.iter().enumerate() {
                    let px = bx + p[0] as f64 + p[1] as f64 / 100.0 * bw;
                    let py = by + p[2] as f64 + p[3] as f64 / 100.0 * bh;
                    if i == 0 {
                        path.move_to((px, py));
                    } else {
                        path.line_to((px, py));
                    }
                }
                path.close_path();
                let fill = if *evenodd { Fill::EvenOdd } else { Fill::NonZero };
                scene.push_layer(fill, BlendMode::default(), 1.0, cur_xf, &path);
            } else if let Some(s) = node.clip_ellipse {
                // `clip-path: circle()/ellipse()` — capa elíptica. Centro y
                // radios resuelven contra la caja de referencia. El centro local
                // alimenta tanto la posición como el cómputo de los lados
                // (closest/farthest-side).
                let cxl = s[0] as f64 + s[1] as f64 / 100.0 * bw;
                let cyl = s[2] as f64 + s[3] as f64 / 100.0 * bh;
                let cx = bx + cxl;
                let cy = by + cyl;
                let rx = resolve_clip_radius(&s[4..9], cxl, cyl, bw, bh, true);
                let ry = resolve_clip_radius(&s[9..14], cxl, cyl, bw, bh, false);
                let ellipse = Ellipse::new((cx, cy), (rx, ry), 0.0);
                scene.push_layer(Fill::NonZero, BlendMode::default(), 1.0, cur_xf, &ellipse);
            } else {
                // `clip_inset` (clip-path: inset) encoge la caja de referencia
                // desde cada borde; `None` (overflow:hidden / geometry-box solo)
                // recorta a la caja de referencia completa.
                let [ct, cr, cb, cl] = node.clip_inset.unwrap_or([0.0; 4]);
                let clip_rect = KurboRect::new(
                    bx + cl as f64,
                    by + ct as f64,
                    bx + bw - cr as f64,
                    by + bh - cb as f64,
                );
                scene.push_layer(Fill::NonZero, BlendMode::default(), 1.0, cur_xf, &clip_rect);
            }
            if pushed {
                layer_stack.push(node.subtree_end);
            }
        }
    }
    // Cerrá capas (clip + alpha) que llegaron al final sin pop intermedio.
    while layer_stack.pop().is_some() {
        scene.pop_layer();
    }
}

/// Pasada GPU directo: recorre el `Mounted` en pre-orden DFS (mismo orden
/// que [`paint`]) e invoca cada `gpu_painter` con el encoder y la
/// `TextureView` del frame. Se ejecuta DESPUÉS de la pasada vello — la
/// intermediate ya tiene fill/image/painter/text encima cuando los
/// callbacks corren, así que su `LoadOp` debe ser `Load`. Devuelve si
/// se invocó al menos un painter (para que el caller decida si vale la
/// pena finalizar y submitir el encoder).
/// `true` si algún nodo del árbol registró un `gpu_painter` (p. ej. el video
/// de media vía `gpu_paint_with`). El eventloop lo usa para decidir si la
/// capa de overlay necesita componerse aparte (sobre el contenido gpu) en vez
/// de pintarse en la escena principal.
pub fn has_gpu_painter<Msg>(mounted: &Mounted<Msg>) -> bool {
    mounted.nodes.iter().any(|n| n.gpu_painter.is_some())
}

pub fn paint_gpu<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    viewport: (u32, u32),
) -> bool {
    let mut any = false;
    for node in &mounted.nodes {
        let Some(painter) = node.gpu_painter.as_ref() else {
            continue;
        };
        let Some(r) = computed.get(node.id) else {
            continue;
        };
        (painter)(
            device,
            queue,
            encoder,
            view,
            PaintRect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
            },
            viewport,
        );
        any = true;
    }
    any
}

/// Hit-test parametrizado por elegibilidad. Devuelve el índice del nodo
/// más al frente (último en pre-orden) cuyo rect contiene `(x, y)` y para
/// el cual `pred` devuelve `true`, respetando `clip`: si el punto cae
/// afuera de un nodo con clip, el subárbol entero es invisible.
///
/// **Respeta `transform`**: igual que [`paint`], compone el afín acumulado
/// de los ancestros (cada `transform` alrededor del centro del rect del
/// nodo, convención CSS `transform-origin: 50% 50%`). El punto de pantalla
/// `(x, y)` se lleva al espacio local del nodo invirtiendo ese afín, y se
/// testea contra el rect sin transformar. Así un nodo rotado/escalado/
/// trasladado recibe los clicks donde realmente se ve pintado (recorrido
/// tipo Prezi, lienzos de tullpu, `@keyframes` de puriy). Un subárbol con
/// afín singular (escala 0) es inalcanzable, igual que es invisible.
pub fn hit_test_pred<Msg, F>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
    pred: F,
) -> Option<usize>
where
    F: Fn(&MountedNode<Msg>) -> bool,
{
    let mut hit: Option<usize> = None;
    let mut clip_stack: Vec<usize> = Vec::new();
    // Espejo del stack de transformaciones de `paint`: `cur_xf` es el
    // producto acumulado de los `transform` de los ancestros activos
    // (local → pantalla). Vacío ⇒ identidad ⇒ camino directo sin invertir
    // (cero costo para la abrumadora mayoría de árboles sin transform).
    let mut xf_stack: Vec<(usize, Affine)> = Vec::new();
    let mut cur_xf = Affine::IDENTITY;
    let mut idx = 0;
    while idx < mounted.nodes.len() {
        while let Some(&end) = clip_stack.last() {
            if idx >= end {
                clip_stack.pop();
            } else {
                break;
            }
        }
        while let Some(&(end, prev)) = xf_stack.last() {
            if idx >= end {
                cur_xf = prev;
                xf_stack.pop();
            } else {
                break;
            }
        }
        let node = &mounted.nodes[idx];
        let Some(r) = computed.get(node.id) else {
            idx += 1;
            continue;
        };
        // Componé el transform de este nodo igual que `paint`, ANTES de
        // resolver el punto local (su propio rect ya cae en el espacio
        // transformado).
        if let Some(centered) = resolve_node_transform(node.transform, node.transform_rel, r) {
            xf_stack.push((node.subtree_end, cur_xf));
            cur_xf *= centered;
        }
        // Punto en el espacio local del nodo. Sin transform activo, es el
        // punto de pantalla tal cual. Con transform, se invierte el afín;
        // si es singular (no invertible) el subárbol es inalcanzable.
        let (lx, ly) = if xf_stack.is_empty() {
            (x as f64, y as f64)
        } else if cur_xf.determinant().abs() < 1e-9 {
            idx = node.subtree_end;
            continue;
        } else {
            let p = cur_xf.inverse() * Point::new(x as f64, y as f64);
            (p.x, p.y)
        };
        let inside = lx >= r.x as f64
            && lx < (r.x + r.w) as f64
            && ly >= r.y as f64
            && ly < (r.y + r.h) as f64;
        if node.clip {
            if !inside {
                idx = node.subtree_end;
                continue;
            }
            clip_stack.push(node.subtree_end);
        }
        if inside && pred(node) {
            hit = Some(idx);
        }
        idx += 1;
    }
    hit
}

/// Hit-test específico para clicks (incluye nodos draggables).
pub fn hit_test_click<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| {
        n.on_click.is_some()
            || n.on_click_at.is_some()
            || n.drag.is_some()
            || n.drag_at.is_some()
            || n.drag_velocity.is_some()
    })
}

/// Hit-test específico para right-click. Sólo considera nodos que
/// declararon `on_right_click` o `on_right_click_at` — un right-click
/// sobre un nodo sin handler no hace nada (no se "filtra" al click
/// izquierdo).
pub fn hit_test_right_click<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| {
        n.on_right_click.is_some() || n.on_right_click_at.is_some()
    })
}

/// Hit-test específico para middle-click. Mismo modelo que right-click:
/// sólo nodos que declararon `on_middle_click` reaccionan.
pub fn hit_test_middle_click<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.on_middle_click.is_some())
}

/// Hit-test específico para hover (nodos con `hover_fill`).
pub fn hit_test_hover<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.hover_fill.is_some())
}

/// Hit-test para movimiento posicional del cursor (nodos con
/// `on_pointer_move_at`). El runtime lo invoca en cada `CursorMoved` para
/// reportar la posición local al nodo más al frente que lo declare.
pub fn hit_test_pointer_move<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.on_pointer_move_at.is_some())
}

/// Hit-test específico para la **forma del cursor**: devuelve el [`Cursor`]
/// del nodo más al frente bajo el punto que declare uno. Como un hijo sin
/// cursor no matchea el predicado, el cursor "cae" al ancestro más cercano que
/// lo declare — herencia estilo CSS sin recorrer el árbol a mano. `None` =
/// ningún nodo bajo el punto declara cursor (el runtime usa el default de la
/// ventana). Lo invoca `llimphi-ui` en la transición de hover.
pub fn hit_test_cursor<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<Cursor> {
    hit_test_pred(mounted, computed, x, y, |n| n.cursor.is_some())
        .and_then(|i| mounted.nodes[i].cursor)
}

/// Hit-test específico para drop targets (nodos con `on_drop`). Usado
/// durante un drag activo para resaltar el destino y para invocar el
/// handler al soltar.
pub fn hit_test_drop<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.on_drop.is_some())
}

/// Hit-test específico para áreas de scroll (nodos con `on_scroll`). El
/// runtime lo usa al recibir la rueda: el nodo más al frente bajo el
/// cursor con handler de scroll consume el evento antes del `on_wheel`
/// global.
pub fn hit_test_scroll<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.on_scroll.is_some())
}

/// Cadena de **scroll anidado**: devuelve todos los nodos con `on_scroll`
/// que contienen el punto, ordenados **front→back** (el primero es el más
/// al frente, igual que [`hit_test_scroll`]; los siguientes son sus
/// ancestros scrollables). El runtime itera la cadena al recibir la rueda
/// y se queda con el primer handler que devuelva `Some`: si un scroll
/// interno está en el extremo del eje y devuelve `None`, el evento "pasa"
/// al ancestro scrollable más cercano (lista dentro de panel, etc.).
/// Recorrido idéntico al de [`hit_test_pred`] pero acumulando todos los
/// hits en vez de pisar.
pub fn hit_test_scroll_chain<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Vec<usize> {
    let mut chain: Vec<usize> = Vec::new();
    let mut clip_stack: Vec<usize> = Vec::new();
    let mut xf_stack: Vec<(usize, Affine)> = Vec::new();
    let mut cur_xf = Affine::IDENTITY;
    let mut idx = 0;
    while idx < mounted.nodes.len() {
        while let Some(&end) = clip_stack.last() {
            if idx >= end {
                clip_stack.pop();
            } else {
                break;
            }
        }
        while let Some(&(end, prev)) = xf_stack.last() {
            if idx >= end {
                cur_xf = prev;
                xf_stack.pop();
            } else {
                break;
            }
        }
        let node = &mounted.nodes[idx];
        let Some(r) = computed.get(node.id) else {
            idx += 1;
            continue;
        };
        if let Some(centered) = resolve_node_transform(node.transform, node.transform_rel, r) {
            xf_stack.push((node.subtree_end, cur_xf));
            cur_xf *= centered;
        }
        let (lx, ly) = if xf_stack.is_empty() {
            (x as f64, y as f64)
        } else if cur_xf.determinant().abs() < 1e-9 {
            idx = node.subtree_end;
            continue;
        } else {
            let p = cur_xf.inverse() * Point::new(x as f64, y as f64);
            (p.x, p.y)
        };
        let inside = lx >= r.x as f64
            && lx < (r.x + r.w) as f64
            && ly >= r.y as f64
            && ly < (r.y + r.h) as f64;
        if node.clip {
            if !inside {
                idx = node.subtree_end;
                continue;
            }
            clip_stack.push(node.subtree_end);
        }
        if inside && node.on_scroll.is_some() {
            chain.push(idx);
        }
        idx += 1;
    }
    // El recorrido es pre-orden, así que los ancestros aparecen primero y
    // los hijos después. Para front→back necesitamos el orden inverso.
    chain.reverse();
    chain
}

/// Hit-test específico para gestos de **escala** (pinch-to-zoom): el nodo más
/// al frente bajo el punto que declaró un `on_scale`. Como un hijo sin handler
/// no matchea el predicado, el gesto "cae" al ancestro más cercano que lo
/// declare (un canvas grande zoomeable con widgets encima que no zoomean). El
/// runtime lo invoca al recibir Ctrl+rueda o un pinch de trackpad. `None` =
/// ningún nodo zoomeable bajo el cursor (el evento cae al scroll/`on_wheel`).
pub fn hit_test_scale<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.on_scale.is_some())
}

/// Hit-test específico para gestos de **rotación** (trackpad): el nodo más al
/// frente bajo el punto que declaró un `on_rotate`. Análogo a
/// [`hit_test_scale`]; el runtime lo invoca al recibir un `RotationGesture`.
/// `None` = ningún nodo rotable bajo el cursor.
pub fn hit_test_rotate<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.on_rotate.is_some())
}

/// Hit-test para **doble-tap**: el nodo más al frente bajo el punto que
/// declaró `on_double_tap`/`on_double_tap_at`. El runtime lo usa al detectar
/// dos presses rápidos y cercanos.
pub fn hit_test_double_tap<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| {
        n.on_double_tap.is_some() || n.on_double_tap_at.is_some()
    })
}

/// Hit-test para **long-press**: el nodo más al frente bajo el punto que
/// declaró `on_long_press`/`on_long_press_at`. El runtime lo usa al armar el
/// gesto en el press (que vence por tiempo si no hay movimiento ni release).
pub fn hit_test_long_press<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| {
        n.on_long_press.is_some() || n.on_long_press_at.is_some()
    })
}

/// Hit-test para **ripple**: el nodo más al frente bajo el punto que declaró
/// un [`Ripple`] (vía [`View::ripple`]). El runtime lo usa en el press para
/// disparar la salpicadura. Aditivo — no compite con click/drag.
pub fn hit_test_ripple<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.ripple.is_some())
}

/// Hit-test para foco: el id `focusable` del nodo más al frente bajo el
/// cursor (click-to-focus). `None` si no se clickeó nada enfocable.
pub fn hit_test_focusable<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<u64> {
    hit_test_pred(mounted, computed, x, y, |n| n.focusable.is_some())
        .and_then(|i| mounted.nodes[i].focusable)
}

/// Hit-test para **selección de texto**: el índice del nodo de texto
/// seleccionable (`text_select_key`) más al frente bajo el cursor. El runtime
/// lo usa para arrancar/extender una selección; devuelve el índice (no la key)
/// para que el caller acceda al `text` + rect del nodo. `None` si no hay texto
/// seleccionable bajo el punto.
pub fn hit_test_selectable<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.text_select_key.is_some())
}

/// Ids enfocables en orden de Tab (pre-orden del árbol = orden de
/// inserción de `Mounted::nodes`). Sólo nodos con rect computado
/// (presentes en el layout). Es el orden DOM-like de tabulación.
pub fn focus_order<Msg>(mounted: &Mounted<Msg>, computed: &ComputedLayout) -> Vec<u64> {
    mounted
        .nodes
        .iter()
        .filter_map(|n| {
            n.focusable
                .filter(|_| computed.get(n.id).is_some())
        })
        .collect()
}

/// Próximo id de foco al pulsar Tab (o Shift+Tab si `reverse`), dado el
/// `order` (de [`focus_order`]) y el `current`. Envuelve en los extremos.
/// Si no hay enfocables devuelve `None`; si `current` ya no existe en el
/// orden, arranca por el primero (Tab) o el último (Shift+Tab).
pub fn next_focus(order: &[u64], current: Option<u64>, reverse: bool) -> Option<u64> {
    if order.is_empty() {
        return None;
    }
    let n = order.len();
    let pos = current.and_then(|c| order.iter().position(|&id| id == c));
    let next_idx = match pos {
        Some(i) => {
            if reverse {
                (i + n - 1) % n
            } else {
                (i + 1) % n
            }
        }
        None => {
            if reverse {
                n - 1
            } else {
                0
            }
        }
    };
    Some(order[next_idx])
}

#[cfg(test)]
mod tests {
    use crate::{hit_test_click, mount, View};
    use llimphi_layout::taffy::prelude::*;
    use llimphi_layout::{LayoutTree, Style};
    use vello::kurbo::Affine;

    #[test]
    fn resolve_clip_radius_lados_y_porcentajes() {
        use super::resolve_clip_radius;
        // Caja 200×100, centro al (50%,50%) = (100,50) local.
        let (w, h, cxl, cyl): (f64, f64, f64, f64) = (200.0, 100.0, 100.0, 50.0);
        // side 0: px + pct_w·w + pct_h·h + pct_diag·diag.
        let diag = (w * w + h * h).sqrt() / core::f64::consts::SQRT_2;
        let r = resolve_clip_radius(&[10.0, 0.0, 0.0, 50.0, 0.0], cxl, cyl, w, h, true);
        assert!((r - (10.0 + 0.5 * diag)).abs() < 1e-6);
        // closest-side circle (1): min(100,100,50,50) = 50.
        assert_eq!(
            resolve_clip_radius(&[0.0, 0.0, 0.0, 0.0, 1.0], cxl, cyl, w, h, true),
            50.0
        );
        // farthest-side circle (2): max(...) = 100.
        assert_eq!(
            resolve_clip_radius(&[0.0, 0.0, 0.0, 0.0, 2.0], cxl, cyl, w, h, true),
            100.0
        );
        // closest-side ellipse eje X (3, is_x): min(cxl, w-cxl) = 100.
        assert_eq!(
            resolve_clip_radius(&[0.0, 0.0, 0.0, 0.0, 3.0], cxl, cyl, w, h, true),
            100.0
        );
        // closest-side ellipse eje Y (3, !is_x): min(cyl, h-cyl) = 50.
        assert_eq!(
            resolve_clip_radius(&[0.0, 0.0, 0.0, 0.0, 3.0], cxl, cyl, w, h, false),
            50.0
        );
        // Centro descentrado (30, 20): closest circle = min(30,170,20,80)=20.
        assert_eq!(
            resolve_clip_radius(&[0.0, 0.0, 0.0, 0.0, 1.0], 30.0, 20.0, w, h, true),
            20.0
        );
    }

    /// Un hijo clickeable de 100×100 anclado arriba-izquierda. Devuelve
    /// `(mounted, computed)` ya layouteados sobre un viewport 400×400.
    fn fixture(
        transform: Option<Affine>,
    ) -> (crate::Mounted<()>, llimphi_layout::ComputedLayout) {
        let mut child = View::<()>::new(Style {
            size: Size {
                width: length(100.0),
                height: length(100.0),
            },
            ..Default::default()
        })
        .on_click(());
        if let Some(xf) = transform {
            child = child.transform(xf);
        }
        let root = View::<()>::new(Style {
            align_items: Some(AlignItems::FlexStart),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .children(vec![child]);
        let mut layout = LayoutTree::new();
        let mounted = mount(&mut layout, root);
        let computed = layout.compute(mounted.root, (400.0, 400.0)).expect("layout");
        (mounted, computed)
    }

    #[test]
    fn sin_transform_el_hit_cae_en_el_rect() {
        let (m, c) = fixture(None);
        assert_eq!(hit_test_click(&m, &c, 50.0, 50.0), Some(1)); // dentro
        assert_eq!(hit_test_click(&m, &c, 250.0, 50.0), None); // fuera
    }

    #[test]
    fn traslacion_mueve_el_area_clickeable() {
        // El nodo se ve corrido +200px en x; el click debe seguirlo.
        let (m, c) = fixture(Some(Affine::translate((200.0, 0.0))));
        assert_eq!(hit_test_click(&m, &c, 250.0, 50.0), Some(1)); // donde se ve
        assert_eq!(hit_test_click(&m, &c, 50.0, 50.0), None); // ya no donde estaba
    }

    #[test]
    fn rotacion_180_grados_alrededor_del_centro() {
        // Rotar 180° alrededor del centro (50,50) deja el rect en su sitio:
        // una esquina mapea a la opuesta, pero el cuadrado cubre lo mismo.
        let (m, c) = fixture(Some(Affine::rotate(std::f64::consts::PI)));
        assert_eq!(hit_test_click(&m, &c, 10.0, 10.0), Some(1));
        assert_eq!(hit_test_click(&m, &c, 90.0, 90.0), Some(1));
        assert_eq!(hit_test_click(&m, &c, 150.0, 150.0), None);
    }

    #[test]
    fn escala_cero_es_inalcanzable() {
        let (m, c) = fixture(Some(Affine::scale(0.0)));
        assert_eq!(hit_test_click(&m, &c, 50.0, 50.0), None);
    }

    /// Como `fixture` pero seteando `transform_rel` (traslación en fracción
    /// del tamaño del nodo) en vez del afín fijo.
    fn fixture_rel(
        rel: (f64, f64),
    ) -> (crate::Mounted<()>, llimphi_layout::ComputedLayout) {
        let child = View::<()>::new(Style {
            size: Size { width: length(100.0), height: length(100.0) },
            ..Default::default()
        })
        .on_click(())
        .transform_rel(rel);
        let root = View::<()>::new(Style {
            align_items: Some(AlignItems::FlexStart),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .children(vec![child]);
        let mut layout = LayoutTree::new();
        let mounted = mount(&mut layout, root);
        let computed = layout.compute(mounted.root, (400.0, 400.0)).expect("layout");
        (mounted, computed)
    }

    #[test]
    fn transform_rel_resuelve_contra_el_tamano_del_nodo() {
        // El nodo es 100×100 en (0,0). `transform_rel(-0.5,-0.5)` =
        // `translate(-50%,-50%)` = correr -50px,-50px (la mitad de 100). El
        // área pintada pasa a (-50,-50)..(50,50): el centro del rect original
        // (50,50) queda ahora en (0,0).
        let (m, c) = fixture_rel((-0.5, -0.5));
        // Donde se ve ahora (el viejo centro corrido a 0,0; y la esquina
        // inferior-derecha del original (100,100) ahora en (50,50)).
        assert_eq!(hit_test_click(&m, &c, 25.0, 25.0), Some(1)); // dentro del corrido
        assert_eq!(hit_test_click(&m, &c, 49.0, 49.0), Some(1)); // casi esquina nueva
        // Donde estaba antes pero ya NO (el rect se corrió fuera de ahí).
        assert_eq!(hit_test_click(&m, &c, 75.0, 75.0), None);
        // Sin transform_rel ese mismo punto SÍ caería dentro (control).
        let (m0, c0) = fixture_rel((0.0, 0.0)); // (0,0) = no-op
        assert_eq!(hit_test_click(&m0, &c0, 75.0, 75.0), Some(1));
    }

    #[test]
    fn hit_test_cursor_directo_y_por_herencia() {
        use crate::{hit_test_cursor, Cursor};
        // Padre 200×200 con cursor Text; dentro un hijo 100×100 (arriba-izq)
        // SIN cursor propio; y un segundo hijo 50×50 con cursor Pointer.
        let hijo_sin = View::<()>::new(Style {
            size: Size { width: length(100.0), height: length(100.0) },
            ..Default::default()
        });
        let hijo_con = View::<()>::new(Style {
            size: Size { width: length(50.0), height: length(50.0) },
            ..Default::default()
        })
        .cursor(Cursor::Pointer);
        let root = View::<()>::new(Style {
            size: Size { width: length(200.0), height: length(200.0) },
            flex_direction: FlexDirection::Column,
            align_items: Some(AlignItems::FlexStart),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .cursor(Cursor::Text)
        .children(vec![hijo_sin, hijo_con]);
        let mut layout = LayoutTree::new();
        let m = mount(&mut layout, root);
        let c = layout.compute(m.root, (400.0, 400.0)).expect("layout");
        // Sobre el hijo sin cursor (0..100, 0..100) → hereda Text del padre.
        assert_eq!(hit_test_cursor(&m, &c, 50.0, 50.0), Some(Cursor::Text));
        // Sobre el hijo con cursor propio (apilado debajo: y 100..150) → Pointer.
        assert_eq!(hit_test_cursor(&m, &c, 25.0, 120.0), Some(Cursor::Pointer));
        // Sobre el padre pero fuera de ambos hijos (x>100) → Text del padre.
        assert_eq!(hit_test_cursor(&m, &c, 150.0, 50.0), Some(Cursor::Text));
        // Fuera del padre → None (la ventana usa su default).
        assert_eq!(hit_test_cursor(&m, &c, 350.0, 350.0), None);
    }

    #[test]
    fn tab_traversal_envuelve_en_los_extremos() {
        use crate::next_focus;
        let order = [10u64, 20, 30];
        // Avanza.
        assert_eq!(next_focus(&order, Some(10), false), Some(20));
        assert_eq!(next_focus(&order, Some(30), false), Some(10)); // wrap
        // Retrocede (Shift+Tab).
        assert_eq!(next_focus(&order, Some(20), true), Some(10));
        assert_eq!(next_focus(&order, Some(10), true), Some(30)); // wrap
        // Sin foco previo: Tab → primero, Shift+Tab → último.
        assert_eq!(next_focus(&order, None, false), Some(10));
        assert_eq!(next_focus(&order, None, true), Some(30));
        // Foco obsoleto (id que ya no está) → arranca por el extremo.
        assert_eq!(next_focus(&order, Some(99), false), Some(10));
        // Lista vacía.
        assert_eq!(next_focus(&[], Some(10), false), None);
    }

    #[test]
    fn hit_test_scale_directo_y_por_herencia() {
        use crate::{hit_test_scale, GesturePhase};
        // Canvas zoomeable 200×200 (declara on_scale); dentro un widget 50×50
        // (arriba-izq) SIN on_scale (no zoomea). El gesto sobre el widget debe
        // "caer" al canvas ancestro (herencia, como el cursor), y fuera de
        // todo debe dar None (el evento cae al scroll/on_wheel).
        let widget = View::<()>::new(Style {
            size: Size { width: length(50.0), height: length(50.0) },
            ..Default::default()
        });
        let canvas = View::<()>::new(Style {
            size: Size { width: length(200.0), height: length(200.0) },
            align_items: Some(AlignItems::FlexStart),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .on_scale(|_phase: GesturePhase, _f, _fx, _fy| None)
        .children(vec![widget]);
        let mut layout = LayoutTree::new();
        let m = mount(&mut layout, canvas);
        let c = layout.compute(m.root, (400.0, 400.0)).expect("layout");
        // Sobre el widget sin on_scale (0..50,0..50) → cae al canvas (idx 0).
        assert_eq!(hit_test_scale(&m, &c, 25.0, 25.0), Some(0));
        // Sobre el canvas fuera del widget (x>50) → el canvas (idx 0).
        assert_eq!(hit_test_scale(&m, &c, 150.0, 25.0), Some(0));
        // Fuera del canvas → None.
        assert_eq!(hit_test_scale(&m, &c, 350.0, 350.0), None);
    }

    #[test]
    fn hit_test_rotate_directo_y_por_herencia() {
        use crate::{hit_test_rotate, GesturePhase};
        // Mismo patrón que escala: canvas rotable con un widget no-rotable
        // encima; el gesto cae al ancestro que declara on_rotate.
        let widget = View::<()>::new(Style {
            size: Size { width: length(50.0), height: length(50.0) },
            ..Default::default()
        });
        let canvas = View::<()>::new(Style {
            size: Size { width: length(200.0), height: length(200.0) },
            align_items: Some(AlignItems::FlexStart),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .on_rotate(|_phase: GesturePhase, _d, _fx, _fy| None)
        .children(vec![widget]);
        let mut layout = LayoutTree::new();
        let m = mount(&mut layout, canvas);
        let c = layout.compute(m.root, (400.0, 400.0)).expect("layout");
        assert_eq!(hit_test_rotate(&m, &c, 25.0, 25.0), Some(0));
        assert_eq!(hit_test_rotate(&m, &c, 150.0, 25.0), Some(0));
        assert_eq!(hit_test_rotate(&m, &c, 350.0, 350.0), None);
    }

    #[test]
    fn hit_test_selectable_solo_sobre_texto_seleccionable() {
        use crate::hit_test_selectable;
        // Un label seleccionable 100×30 arriba-izq dentro de un panel 200×200
        // SIN selectable. Sólo el label matchea; el resto del panel da None.
        let label = View::<()>::new(Style {
            size: Size { width: length(100.0), height: length(30.0) },
            ..Default::default()
        })
        .text("hola", 14.0, vello::peniko::Color::from_rgba8(255, 255, 255, 255))
        .selectable(7);
        let panel = View::<()>::new(Style {
            size: Size { width: length(200.0), height: length(200.0) },
            align_items: Some(AlignItems::FlexStart),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .children(vec![label]);
        let mut layout = LayoutTree::new();
        let m = mount(&mut layout, panel);
        let c = layout.compute(m.root, (400.0, 400.0)).expect("layout");
        // Sobre el label (0..100, 0..30) → el label (idx 1).
        assert_eq!(hit_test_selectable(&m, &c, 50.0, 15.0), Some(1));
        // Sobre el panel fuera del label → None (el panel no es selectable).
        assert_eq!(hit_test_selectable(&m, &c, 150.0, 150.0), None);
    }

    #[test]
    fn hit_test_scroll_chain_devuelve_front_to_back() {
        use crate::hit_test_scroll_chain;
        // Padre scrollable 200×200 con un hijo scrollable 100×100 (arriba-izq).
        // Bajo el hijo: chain = [hijo, padre]. Bajo el padre pero fuera del
        // hijo: chain = [padre]. Fuera de ambos: chain vacío.
        let hijo = View::<()>::new(Style {
            size: Size { width: length(100.0), height: length(100.0) },
            ..Default::default()
        })
        .on_scroll(|_dx, _dy| None::<()>);
        let padre = View::<()>::new(Style {
            size: Size { width: length(200.0), height: length(200.0) },
            align_items: Some(AlignItems::FlexStart),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .on_scroll(|_dx, _dy| None::<()>)
        .children(vec![hijo]);
        let mut layout = LayoutTree::new();
        let m = mount(&mut layout, padre);
        let c = layout.compute(m.root, (400.0, 400.0)).expect("layout");
        // Sobre el hijo (0..100,0..100) → chain = [hijo=1, padre=0].
        let ch = hit_test_scroll_chain(&m, &c, 50.0, 50.0);
        assert_eq!(ch, vec![1, 0]);
        // Sobre el padre fuera del hijo (x>100) → chain = [padre=0].
        let ch = hit_test_scroll_chain(&m, &c, 150.0, 50.0);
        assert_eq!(ch, vec![0]);
        // Fuera del padre → chain vacío.
        let ch = hit_test_scroll_chain(&m, &c, 350.0, 350.0);
        assert!(ch.is_empty());
    }

    #[test]
    fn hit_test_double_tap_y_long_press() {
        use crate::{hit_test_double_tap, hit_test_long_press};
        // Un nodo 100×100 con doble-tap; otro 100×100 apilado debajo con
        // long-press. Cada hit-test sólo ve su propio gesto.
        let arriba = View::<()>::new(Style {
            size: Size { width: length(100.0), height: length(100.0) },
            ..Default::default()
        })
        .on_double_tap(());
        let abajo = View::<()>::new(Style {
            size: Size { width: length(100.0), height: length(100.0) },
            ..Default::default()
        })
        .on_long_press(());
        let root = View::<()>::new(Style {
            flex_direction: FlexDirection::Column,
            align_items: Some(AlignItems::FlexStart),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .children(vec![arriba, abajo]);
        let mut layout = LayoutTree::new();
        let m = mount(&mut layout, root);
        let c = layout.compute(m.root, (400.0, 400.0)).expect("layout");
        // Nodo de arriba (y 0..100): doble-tap sí, long-press no.
        assert_eq!(hit_test_double_tap(&m, &c, 50.0, 50.0), Some(1));
        assert_eq!(hit_test_long_press(&m, &c, 50.0, 50.0), None);
        // Nodo de abajo (y 100..200): long-press sí, doble-tap no.
        assert_eq!(hit_test_long_press(&m, &c, 50.0, 150.0), Some(2));
        assert_eq!(hit_test_double_tap(&m, &c, 50.0, 150.0), None);
        // Fuera de ambos.
        assert_eq!(hit_test_double_tap(&m, &c, 300.0, 300.0), None);
        assert_eq!(hit_test_long_press(&m, &c, 300.0, 300.0), None);
    }
}
