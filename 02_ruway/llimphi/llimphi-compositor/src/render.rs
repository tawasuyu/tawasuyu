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
        on_scroll,
        focusable,
        alpha,
        transform,
        tooltip,
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
        on_scroll,
        focusable,
        alpha,
        transform,
        tooltip,
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
    let block = llimphi_text::TextBlock {
        text: &tm.content,
        size_px: tm.size_px,
        color: Color::BLACK,
        origin: (0.0, 0.0),
        max_width,
        alignment: tm.alignment,
        line_height: tm.line_height,
        italic: tm.italic,
        font_family: tm.font_family.clone(),
    };
    let m = llimphi_text::measure(ts, &block);
    llimphi_layout::taffy::Size { width: m.width, height: m.height }
}

pub fn paint<Msg>(
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
    // Stack de transformaciones afines de subtree. Cada entrada guarda el
    // `subtree_end` y la `cur_xf` previa para restaurarla al salir del
    // subárbol. `cur_xf` es el producto acumulado de todos los `transform`
    // de los ancestros activos — se multiplica en cada draw call. Cuando
    // ningún nodo transforma, queda en `IDENTITY` y el paint es idéntico
    // al previo (cero regresión).
    let mut xf_stack: Vec<(usize, Affine)> = Vec::new();
    let mut cur_xf = Affine::IDENTITY;
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
        if let Some(local) = node.transform {
            let cx = (r.x + r.w * 0.5) as f64;
            let cy = (r.y + r.h * 0.5) as f64;
            let centered =
                Affine::translate((cx, cy)) * local * Affine::translate((-cx, -cy));
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
            scene.push_layer(Mix::Normal, a, cur_xf, &rect);
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
            scene.fill(Fill::NonZero, cur_xf, color, None, &rr);
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
                scene.push_layer(Mix::Clip, 1.0, cur_xf, &node_rect);
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
                    text.line_height,
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
                    line_height: text.line_height,
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
                llimphi_text::draw_layout_xf(
                    scene,
                    &layout,
                    text.color,
                    cur_xf * Affine::translate(origin),
                );
            }
        }
        if node.clip {
            let clip_rect = KurboRect::new(
                r.x as f64,
                r.y as f64,
                (r.x + r.w) as f64,
                (r.y + r.h) as f64,
            );
            scene.push_layer(Mix::Clip, 1.0, cur_xf, &clip_rect);
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
        if let Some(local) = node.transform {
            let cx = (r.x + r.w * 0.5) as f64;
            let cy = (r.y + r.h * 0.5) as f64;
            let centered =
                Affine::translate((cx, cy)) * local * Affine::translate((-cx, -cy));
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
}
