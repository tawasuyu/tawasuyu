// helpers.rs — Funciones puras auxiliares del bucle Elm.
// Todas sin efecto observable sobre el runtime; se testean fácil de forma aislada.

use super::super::*;

/// Mapea el [`Cursor`](llimphi_compositor::Cursor) llimphi-native (resuelto por
/// el hit-test de hover) a `winit::window::CursorIcon`. `None` → flecha default.
/// Mantiene al compositor winit-free: la traducción vive sólo en el runtime.
pub(super) fn to_winit_cursor(c: Option<llimphi_compositor::Cursor>) -> llimphi_hal::winit::window::CursorIcon {
    use llimphi_compositor::Cursor as C;
    use llimphi_hal::winit::window::CursorIcon as I;
    match c {
        None | Some(C::Default) => I::Default,
        Some(C::Pointer) => I::Pointer,
        Some(C::Text) => I::Text,
        Some(C::Crosshair) => I::Crosshair,
        Some(C::Move) => I::Move,
        Some(C::Grab) => I::Grab,
        Some(C::Grabbing) => I::Grabbing,
        Some(C::NotAllowed) => I::NotAllowed,
        Some(C::Wait) => I::Wait,
        Some(C::Progress) => I::Progress,
        Some(C::Help) => I::Help,
        Some(C::ColResize) => I::ColResize,
        Some(C::RowResize) => I::RowResize,
        Some(C::EwResize) => I::EwResize,
        Some(C::NsResize) => I::NsResize,
        Some(C::NeswResize) => I::NeswResize,
        Some(C::NwseResize) => I::NwseResize,
        Some(C::ZoomIn) => I::ZoomIn,
        Some(C::ZoomOut) => I::ZoomOut,
    }
}

/// Resuelve el handler de **escala** (pinch-to-zoom) bajo el punto `(x, y)`
/// contra el cache del último frame (overlay con prioridad, igual que clicks).
/// Devuelve `(handler, focal_x, focal_y)` con el punto focal ya en coordenadas
/// **locales** al rect del nodo. `None` si no hay nodo `on_scale` bajo el
/// cursor. Compartido por el camino Ctrl+rueda y el de `PinchGesture`.
pub(super) fn scale_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<(ScaleFn<Msg>, f32, f32)> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    hit_test_scale(m, c, x, y).and_then(|i| {
        let node = &m.nodes[i];
        node.on_scale.clone().map(|h| {
            let (fx, fy) = c
                .get(node.id)
                .map(|r| (x - r.x, y - r.y))
                .unwrap_or((0.0, 0.0));
            (h, fx, fy)
        })
    })
}

/// Resuelve el handler de **rotación** (trackpad) bajo `(x, y)` contra el
/// cache del último frame (overlay con prioridad). Espejo de
/// [`scale_hit_from_cache`]. Devuelve `(handler, focal_x, focal_y)` con el
/// punto focal local al rect del nodo. `None` si no hay nodo `on_rotate`.
pub(super) fn rotate_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<(RotateFn<Msg>, f32, f32)> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    hit_test_rotate(m, c, x, y).and_then(|i| {
        let node = &m.nodes[i];
        node.on_rotate.clone().map(|h| {
            let (fx, fy) = c
                .get(node.id)
                .map(|r| (x - r.x, y - r.y))
                .unwrap_or((0.0, 0.0));
            (h, fx, fy)
        })
    })
}

/// Resuelve el handler de **doble-tap** bajo `(x, y)` contra el cache del
/// último frame (overlay con prioridad). Elige la variante `_at` (con focal
/// local) si está, o el `Msg` directo. `None` si no hay nodo con doble-tap.
pub(super) fn double_tap_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<GestureResolved<Msg>> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    hit_test_double_tap(m, c, x, y).and_then(|i| {
        let node = &m.nodes[i];
        let (rx, ry, rw, rh) = c.get(node.id).map(|r| (r.x, r.y, r.w, r.h)).unwrap_or_default();
        if let Some(h) = node.on_double_tap_at.clone() {
            Some(GestureResolved::At(h, x - rx, y - ry, rw, rh))
        } else {
            node.on_double_tap.clone().map(GestureResolved::Direct)
        }
    })
}

/// Como [`double_tap_hit_from_cache`] pero para **long-press**.
pub(super) fn long_press_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<GestureResolved<Msg>> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    hit_test_long_press(m, c, x, y).and_then(|i| {
        let node = &m.nodes[i];
        let (rx, ry, rw, rh) = c.get(node.id).map(|r| (r.x, r.y, r.w, r.h)).unwrap_or_default();
        if let Some(h) = node.on_long_press_at.clone() {
            Some(GestureResolved::At(h, x - rx, y - ry, rw, rh))
        } else {
            node.on_long_press.clone().map(GestureResolved::Direct)
        }
    })
}

/// Resuelve el **ripple** bajo `(x, y)` contra el cache del último frame
/// (overlay con prioridad). Devuelve `(Ripple, lx, ly)`: la config de la onda
/// + el punto del tap relativo al rect del nodo. `None` si no hay nodo ripple.
pub(super) fn ripple_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<(llimphi_compositor::Ripple, f32, f32)> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    hit_test_ripple(m, c, x, y).and_then(|i| {
        let node = &m.nodes[i];
        node.ripple.map(|rp| {
            let (rx, ry) = c.get(node.id).map(|r| (r.x, r.y)).unwrap_or_default();
            (rp, x - rx, y - ry)
        })
    })
}

// ── Selección de texto fuera del editor (ver `View::selectable`) ──

/// Rect absoluto de un nodo: `(x, y, w, h)`.
pub(super) type AbsRect = (f32, f32, f32, f32);

/// `true` si el `TextSpec` es de texto **uniforme** (sin `runs`/`spans`): los
/// únicos que la selección fuera-del-editor soporta. Los multicolor/RichText
/// son del editor y se ignoran.
pub(super) fn spec_is_uniform(spec: &llimphi_compositor::TextSpec) -> bool {
    spec.runs.is_none() && spec.spans.is_none()
}

/// Bajo `(x, y)`, el nodo de texto seleccionable más al frente: su key, su
/// `TextSpec` clonado y su rect absoluto. `None` si no hay texto seleccionable
/// uniforme ahí.
pub(super) fn selectable_hit_from_cache<Msg: Clone>(
    cache: &RenderCache<Msg>,
    x: f32,
    y: f32,
) -> Option<(u64, llimphi_compositor::TextSpec, AbsRect)> {
    let (m, c) = match cache.overlay.as_ref() {
        Some(ov) => (&ov.mounted, &ov.computed),
        None => (&cache.mounted, &cache.computed),
    };
    let i = hit_test_selectable(m, c, x, y)?;
    let node = &m.nodes[i];
    let key = node.text_select_key?;
    let spec = node.text.as_ref()?;
    if !spec_is_uniform(spec) {
        return None;
    }
    let r = c.get(node.id)?;
    Some((key, spec.clone(), (r.x, r.y, r.w, r.h)))
}

/// Busca el nodo seleccionable por su `key` estable (para extender el drag o
/// pintar el resaltado en frames posteriores, cuando el `NodeId` ya cambió).
/// Recorre el overlay y el árbol principal. `None` si la key ya no está.
pub(super) fn selectable_by_key<Msg>(
    cache: &RenderCache<Msg>,
    key: u64,
) -> Option<(llimphi_compositor::TextSpec, AbsRect)> {
    let trees = [
        cache.overlay.as_ref().map(|ov| (&ov.mounted, &ov.computed)),
        Some((&cache.mounted, &cache.computed)),
    ];
    trees
        .into_iter()
        .flatten()
        .find_map(|(m, c)| selectable_node_in(m, c, key))
}

/// Busca en un árbol montado concreto el nodo de texto seleccionable con esa
/// `key` y devuelve su `TextSpec` clonado + rect. Lo usa tanto la búsqueda por
/// cache como el pintado del resaltado en el redraw (que tiene el `Mounted`
/// del frame a mano, no un `RenderCache`).
pub(super) fn selectable_node_in<Msg>(
    m: &Mounted<Msg>,
    c: &ComputedLayout,
    key: u64,
) -> Option<(llimphi_compositor::TextSpec, AbsRect)> {
    for node in &m.nodes {
        if node.text_select_key == Some(key) {
            let spec = node.text.as_ref()?;
            if !spec_is_uniform(spec) {
                return None;
            }
            let r = c.get(node.id)?;
            return Some((spec.clone(), (r.x, r.y, r.w, r.h)));
        }
    }
    None
}

/// Reconstruye el `parley::Layout` de un nodo de texto, idéntico al que pinta
/// el render (misma ruta cacheada `Typesetter::layout`), para hit-testear y
/// medir la selección. El ancho de wrap es el del rect del nodo.
pub(super) fn build_selectable_layout(
    ts: &mut llimphi_text::Typesetter,
    spec: &llimphi_compositor::TextSpec,
    width: f32,
) -> llimphi_text::parley::Layout<()> {
    ts.layout(
        &spec.content,
        spec.size_px,
        Some(width),
        spec.alignment,
        spec.line_height,
        spec.italic,
        spec.font_family.as_deref(),
        spec.weight,
        spec.underline,
        spec.strikethrough,
        spec.letter_spacing,
        spec.word_spacing,
        spec.overflow_wrap,
    )
}

/// `true` si la tecla lógica es el carácter `c` (case-insensitive). Para
/// atajos como Ctrl+C sin acoplarse a mayúsculas/minúsculas ni layout.
pub(super) fn key_is_char(key: &Key, c: char) -> bool {
    matches!(
        key,
        Key::Character(s) if s.chars().next().map(|k| k.eq_ignore_ascii_case(&c)).unwrap_or(false)
    )
}

/// Copia texto al portapapeles del sistema (best-effort). Con la feature
/// `clipboard` usa `arboard`; sin backend (headless) o sin la feature es no-op
/// silencioso — nunca panica.
#[cfg(feature = "clipboard")]
pub(super) fn copy_to_clipboard(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text.to_string());
    }
}

#[cfg(not(feature = "clipboard"))]
pub(super) fn copy_to_clipboard(_text: &str) {}

/// Resuelve los [`View::layout_builder`] del árbol de la app en dos pasadas
/// (ver [`llimphi_compositor::expand_layout_builders`]). **Coste cero** cuando
/// ningún nodo usa el builder: devuelve el `view()` sin tocar tras un walk
/// barato. Cuando hay builders: monta el árbol (builders como hojas), computa
/// para conocer sus slots, y reconstruye un `view()` fresco expandiendo cada
/// builder con sus constraints reales. `viewport` en px físicos; `ts` para medir
/// texto igual que el compute principal. Lo llaman el redraw (vía cache) y el
/// fallback de press.
pub(super) fn resolve_layout_builders<A: App>(
    model: &A::Model,
    viewport: (f32, f32),
    ts: &mut llimphi_text::Typesetter,
) -> View<A::Msg> {
    let view = A::view(model);
    if !has_layout_builder(&view) {
        return view;
    }
    // Pasada 1: montar (builders = hojas con su Style) y computar el layout.
    let mut l1 = LayoutTree::new();
    let m1: Mounted<A::Msg> = mount(&mut l1, view);
    let c1 = {
        let tmap = &m1.text_measures;
        l1.compute_with_measure(m1.root, viewport, |nid, known, avail| {
            match tmap.get(&nid) {
                Some(tm) => measure_text_node(ts, tm, known, avail),
                None => llimphi_layout::taffy::Size::ZERO,
            }
        })
        .expect("layout layout_builder pasada 1")
    };
    let cons = collect_builder_constraints(&m1, &c1);
    // Pasada 2: árbol fresco (mismo Model → misma estructura, mismo pre-orden de
    // builders) + expand con las constraints resueltas.
    expand_layout_builders(A::view(model), &cons)
}
