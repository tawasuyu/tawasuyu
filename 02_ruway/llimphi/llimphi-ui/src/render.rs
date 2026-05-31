use super::*;

pub(crate) fn mount<Msg: Clone>(layout: &mut LayoutTree, v: View<Msg>) -> Mounted<Msg> {
    let mut nodes = Vec::new();
    let root = mount_recursive(layout, v, &mut nodes);
    Mounted { root, nodes }
}

/// Mount en pre-orden directo sobre `out`: pusheamos el padre como
/// placeholder (id real desconocido hasta crear el taffy node), recursamos
/// hijos sobre el mismo `out`, y al volver completamos `id` + `subtree_end`.
pub(crate) fn mount_recursive<Msg: Clone>(
    layout: &mut LayoutTree,
    v: View<Msg>,
    out: &mut Vec<MountedNode<Msg>>,
) -> NodeId {
    let View {
        style,
        fill,
        hover_fill,
        radius,
        text,
        image,
        painter,
        gpu_painter,
        on_click,
        on_click_at,
        on_right_click,
        on_right_click_at,
        on_middle_click,
        drag,
        drag_at,
        drag_payload,
        on_drop,
        drop_hover_fill,
        clip,
        on_pointer_enter,
        on_pointer_leave,
        alpha,
        children,
    } = v;
    let parent_idx = out.len();
    out.push(MountedNode {
        id: NodeId::new(0), // placeholder, lo sobreescribimos abajo
        fill,
        hover_fill,
        radius,
        text,
        image,
        painter,
        gpu_painter,
        on_click,
        on_click_at,
        on_right_click,
        on_right_click_at,
        on_middle_click,
        drag,
        drag_at,
        drag_payload,
        on_drop,
        drop_hover_fill,
        clip,
        on_pointer_enter,
        on_pointer_leave,
        alpha,
        subtree_end: 0,
    });
    let mut child_ids = Vec::with_capacity(children.len());
    for child in children {
        child_ids.push(mount_recursive(layout, child, out));
    }
    let id = if child_ids.is_empty() {
        layout.leaf(style).expect("layout leaf")
    } else {
        layout.node(style, &child_ids).expect("layout node")
    };
    out[parent_idx].id = id;
    out[parent_idx].subtree_end = out.len();
    id
}

pub(crate) fn paint<Msg>(
    scene: &mut vello::Scene,
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    typesetter: &mut llimphi_text::Typesetter,
    hover_idx: Option<usize>,
    drop_hover_idx: Option<usize>,
) {
    // Stack de subtree_end de los `push_layer` activos (clip y/o alpha).
    // Vello requiere pop_layer en orden LIFO estricto, así que mantenemos
    // un único stack común y popeamos en el orden en que se pushearon.
    // Dos entradas con el mismo `subtree_end` (alpha + clip sobre el
    // mismo nodo) se cierran en el orden inverso al push.
    let mut layer_stack: Vec<usize> = Vec::new();
    for (idx, node) in mounted.nodes.iter().enumerate() {
        // Cierre de capas que ya quedaron atrás (idx ≥ subtree_end).
        while let Some(&end) = layer_stack.last() {
            if idx >= end {
                scene.pop_layer();
                layer_stack.pop();
            } else {
                break;
            }
        }
        let Some(r) = computed.get(node.id) else {
            continue;
        };
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
            scene.push_layer(Mix::Normal, a, Affine::IDENTITY, &rect);
            layer_stack.push(node.subtree_end);
        }
        // Prioridad de pintura: drop-hover (drag activo) > hover normal >
        // fill base. Solo aplica el override si el slot correspondiente
        // está poblado; el siguiente cae como fallback.
        let effective_fill = if Some(idx) == drop_hover_idx {
            node.drop_hover_fill.or(node.hover_fill).or(node.fill)
        } else if Some(idx) == hover_idx {
            node.hover_fill.or(node.fill)
        } else {
            node.fill
        };
        if let Some(color) = effective_fill {
            let rr = RoundedRect::new(
                r.x as f64,
                r.y as f64,
                (r.x + r.w) as f64,
                (r.y + r.h) as f64,
                node.radius,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rr);
        }
        if let Some(image) = node.image.as_ref() {
            // Aspect-fit centrado: el min de las dos escalas ocupa
            // todo el rect en el eje más restrictivo y deja banda en
            // el otro. Defensivo: envolvemos en push_layer/pop_layer
            // con el rect del nodo para que, aunque el caller pida
            // un layout mal-dimensionado, la imagen nunca pinte fuera
            // del nodo (visualmente preferible a un overflow opaco).
            if image.width > 0 && image.height > 0 && r.w > 0.0 && r.h > 0.0 {
                let sx = r.w as f64 / image.width as f64;
                let sy = r.h as f64 / image.height as f64;
                let s = sx.min(sy);
                let disp_w = image.width as f64 * s;
                let disp_h = image.height as f64 * s;
                let tx = r.x as f64 + (r.w as f64 - disp_w) * 0.5;
                let ty = r.y as f64 + (r.h as f64 - disp_h) * 0.5;
                let transform = Affine::translate((tx, ty)) * Affine::scale(s);
                let node_rect = KurboRect::new(
                    r.x as f64,
                    r.y as f64,
                    (r.x + r.w) as f64,
                    (r.y + r.h) as f64,
                );
                scene.push_layer(Mix::Clip, 1.0, Affine::IDENTITY, &node_rect);
                scene.draw_image(image, transform);
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
            if let Some(runs) = text.runs.as_ref() {
                // Texto multicolor (syntax highlighting): una sola pasada de
                // shaping con color por rango, anclado arriba-izquierda. Cae
                // por el flujo normal (clip/alpha se cierran como siempre).
                let layout = typesetter.layout_runs(
                    &text.content,
                    text.size_px,
                    text.color,
                    runs,
                    text.alignment,
                    1.2,
                );
                llimphi_text::draw_layout_runs(scene, &layout, (r.x as f64, r.y as f64));
            } else {
                // Parley resuelve la alineación horizontal vía max_width +
                // alignment. Para Center también centramos verticalmente; para
                // Start/End/Justify anclamos arriba (párrafo/editor).
                let block = llimphi_text::TextBlock {
                    text: &text.content,
                    size_px: text.size_px,
                    color: text.color,
                    origin: (r.x as f64, r.y as f64),
                    max_width: Some(r.w),
                    alignment: text.alignment,
                    line_height: 1.2,
                    italic: text.italic,
                    font_family: text.font_family.clone(),
                };
                // Shaping una sola vez: el `Layout` retornado se reusa para
                // medir (cuando hay centrado vertical) y para pintar.
                let layout = llimphi_text::layout_block(typesetter, &block);
                let origin =
                    if matches!(text.alignment, llimphi_text::Alignment::Center) {
                        let m = llimphi_text::measurement(&layout);
                        (
                            r.x as f64,
                            r.y as f64 + ((r.h - m.height) as f64 * 0.5).max(0.0),
                        )
                    } else {
                        block.origin
                    };
                llimphi_text::draw_layout(scene, &layout, text.color, origin);
            }
        }
        if node.clip {
            let clip_rect = KurboRect::new(
                r.x as f64,
                r.y as f64,
                (r.x + r.w) as f64,
                (r.y + r.h) as f64,
            );
            scene.push_layer(Mix::Clip, 1.0, Affine::IDENTITY, &clip_rect);
            layer_stack.push(node.subtree_end);
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
pub(crate) fn paint_gpu<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    device: &llimphi_hal::wgpu::Device,
    queue: &llimphi_hal::wgpu::Queue,
    encoder: &mut llimphi_hal::wgpu::CommandEncoder,
    view: &llimphi_hal::wgpu::TextureView,
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
pub(crate) fn hit_test_pred<Msg, F>(
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
    let mut idx = 0;
    while idx < mounted.nodes.len() {
        while let Some(&end) = clip_stack.last() {
            if idx >= end {
                clip_stack.pop();
            } else {
                break;
            }
        }
        let node = &mounted.nodes[idx];
        let Some(r) = computed.get(node.id) else {
            idx += 1;
            continue;
        };
        let inside = x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;
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
pub(crate) fn hit_test_click<Msg>(
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
    })
}

/// Hit-test específico para right-click. Sólo considera nodos que
/// declararon `on_right_click` o `on_right_click_at` — un right-click
/// sobre un nodo sin handler no hace nada (no se "filtra" al click
/// izquierdo).
pub(crate) fn hit_test_right_click<Msg>(
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
pub(crate) fn hit_test_middle_click<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.on_middle_click.is_some())
}

/// Hit-test específico para hover (nodos con `hover_fill`).
pub(crate) fn hit_test_hover<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.hover_fill.is_some())
}

/// Hit-test específico para drop targets (nodos con `on_drop`). Usado
/// durante un drag activo para resaltar el destino y para invocar el
/// handler al soltar.
pub(crate) fn hit_test_drop<Msg>(
    mounted: &Mounted<Msg>,
    computed: &ComputedLayout,
    x: f32,
    y: f32,
) -> Option<usize> {
    hit_test_pred(mounted, computed, x, y, |n| n.on_drop.is_some())
}
