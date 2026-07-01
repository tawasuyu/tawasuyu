//! Render de las cuatro vistas-panel meta-driven: `build_view_panel`
//! despacha por `ModuleView` y delega en el panel concreto —grafo de
//! morfismos, lista (con búsqueda/orden/paginación/drill), ficha
//! `Detail` (campos + KPIs 360 + listas relacionadas) y formulario
//! (`build_form_panel` + `build_field_control` por `FieldKind`)—. Los
//! tableros/reportes viven en `tablero`; acá quedan el resto.

use super::*;

pub(crate) fn build_view_panel(
    model: &Model,
    mod_idx: usize,
    view_key: &str,
    view: &ModuleView,
    theme: &Theme,
) -> View<Msg> {
    let module = &model.modules[mod_idx];
    match view {
        ModuleView::List(lv) => build_list_panel(model, mod_idx, lv, theme),
        ModuleView::Form(fv) => {
            // Form alcanzado sin sesión activa (p.ej. tras cancelar):
            // ofrecer reabrirlo.
            let title = text_line(
                format!("{} · {}", module.label, fv.title),
                16.0,
                theme.fg_text,
            );
            let open = button_styled(
                "+ Abrir formulario",
                btn_style(200.0),
                Alignment::Center,
                &accent_btn(theme),
                Msg::OpenForm {
                    module_idx: mod_idx,
                    view_key: form_view_key(module, fv),
                },
            );
            column(vec![title, open], 8.0)
        }
        ModuleView::Detail(dv) => {
            // Una Detail seleccionada desde el menú no tiene record
            // objetivo: se llega con el 👁 de una fila de lista.
            let lines = vec![format!(
                "elegí un record desde una lista (botón 👁) para ver su ficha de '{}'.",
                dv.entity
            )];
            placeholder_panel(module, &dv.title, lines, theme)
        }
        ModuleView::Dashboard(dv) => {
            build_dashboard_panel(model, mod_idx, view_key, dv, theme)
        }
        ModuleView::Report(rv) => {
            build_report_panel(model, mod_idx, view_key, rv, theme)
        }
        ModuleView::Graph(gv) => build_graph_panel(model, mod_idx, gv, theme),
    }
}

/// Origen y paso del auto-layout por rango topológico de la vista grafo.
pub(crate) const GRAPH_ORIGIN_X: f32 = 24.0;
pub(crate) const GRAPH_ORIGIN_Y: f32 = 16.0;
pub(crate) const GRAPH_COL_STEP: f32 = 220.0;
pub(crate) const GRAPH_ROW_STEP: f32 = 130.0;

/// Vista `Graph`: el DAG de morfismos del módulo nakui pintado sobre el
/// `llimphi-widget-nodegraph`. Cada morfismo es un nodo cuyos pins de
/// entrada son los tokens que lee y los de salida los que escribe; cada
/// par escritura→lectura del mismo token es un cable. El layout base es
/// por rango (profundidad de flujo de datos); el usuario puede arrastrar
/// nodos y sus posiciones se fijan en `model.graph_pos` (clave estable
/// `(module_id, morfismo)`) y se persisten al sidecar al soltar, así
/// sobreviven a reinicios.
pub(crate) fn build_graph_panel(model: &Model, mod_idx: usize, gv: &GraphView, theme: &Theme) -> View<Msg> {
    let module = &model.modules[mod_idx];
    let data = model
        .backend
        .lock()
        .ok()
        .and_then(|b| b.morphism_graph(&module.id));
    let data = match data {
        Some(d) if !d.nodes.is_empty() => d,
        Some(_) => {
            return placeholder_panel(
                module,
                &gv.title,
                vec!["el módulo no declara morfismos — no hay grafo que mostrar.".into()],
                theme,
            );
        }
        None => {
            return placeholder_panel(
                module,
                &gv.title,
                vec![format!(
                    "'{}' no tiene executor nakui (falta `nakui_module_dir`): sin grafo de morfismos.",
                    module.label
                )],
                theme,
            );
        }
    };

    let base = graph_layout(&data);
    let idx_of: BTreeMap<&str, usize> = data
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.name.as_str(), i))
        .collect();

    // Cámara: posición mundo → pantalla y métricas escaladas por el zoom.
    let zoom = model.graph_zoom;
    let pan = model.graph_pan;

    let nodes: Vec<NodeSpec> = data
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let id = i as NodeId;
            let (wx, wy) = model
                .graph_pos
                .get(&(module.id.clone(), n.name.clone()))
                .copied()
                .unwrap_or(base[i]);
            NodeSpec {
                id,
                label: n.name.clone(),
                x: wx * zoom + pan.0,
                y: wy * zoom + pan.1,
                inputs: n.reads.clone(),
                outputs: n.writes.clone(),
            }
        })
        .collect();

    let mut wires: Vec<Wire> = Vec::with_capacity(data.edges.len());
    for e in &data.edges {
        let (Some(&fi), Some(&ti)) =
            (idx_of.get(e.from.as_str()), idx_of.get(e.to.as_str()))
        else {
            continue;
        };
        let from_output = data.nodes[fi]
            .writes
            .iter()
            .position(|t| t == &e.token)
            .unwrap_or(0) as u16;
        let to_input = data.nodes[ti]
            .reads
            .iter()
            .position(|t| t == &e.token)
            .unwrap_or(0) as u16;
        wires.push(Wire {
            from_node: fi as NodeId,
            from_output,
            to_node: ti as NodeId,
            to_input,
        });
    }

    let palette = NodegraphPalette::from_theme(theme);
    // Geometría escalada por el zoom: nodos, pins, texto y trazo crecen
    // juntos para que el grafo se acerque/aleje como un todo.
    let base_metrics = NodegraphMetrics::default();
    let metrics = NodegraphMetrics {
        node_width: base_metrics.node_width * zoom,
        title_height: base_metrics.title_height * zoom,
        pin_row_height: base_metrics.pin_row_height * zoom,
        pin_radius: base_metrics.pin_radius * zoom,
        pin_label_size: base_metrics.pin_label_size * zoom,
        title_text_size: base_metrics.title_text_size * zoom,
        wire_stroke: base_metrics.wire_stroke * zoom,
        node_radius: base_metrics.node_radius * zoom as f64,
    };

    // Selección activa (si el morfismo seleccionado pertenece a este
    // módulo y sigue existiendo) y su cono: nodos aguas abajo (lo que
    // el morfismo afecta) y aguas arriba (de lo que depende).
    let selected: Option<NodeId> = match model.graph_selected {
        Some((mi, id)) if mi == mod_idx && (id as usize) < nodes.len() => Some(id),
        _ => None,
    };
    let cone = selected.map(|sel| graph_cone(sel, &wires, nodes.len()));

    // Tintes derivados del tema (el cono se pinta sólo si hay selección).
    let sel_tint = NodeTint {
        bg_node: Some(theme.bg_selected),
        bg_title: Some(theme.accent),
        fg_title: Some(theme.bg_app),
    };
    let down_tint = NodeTint {
        bg_node: Some(Color::from_rgba8(40, 33, 18, 255)),
        bg_title: Some(Color::from_rgba8(150, 110, 30, 255)),
        fg_title: Some(theme.fg_text),
    };
    let up_tint = NodeTint {
        bg_node: Some(Color::from_rgba8(16, 30, 36, 255)),
        bg_title: Some(Color::from_rgba8(30, 100, 120, 255)),
        fg_title: Some(theme.fg_text),
    };
    let dim_tint = NodeTint {
        bg_node: Some(theme.bg_panel_alt),
        bg_title: Some(theme.bg_panel_alt),
        fg_title: Some(theme.fg_placeholder),
    };
    let wire_hot = theme.accent;
    let wire_dim = theme.border;

    let node_tint_fn = |id: NodeId| -> Option<NodeTint> {
        let sel = selected?;
        let (down, up) = cone.as_ref()?;
        if id == sel {
            Some(sel_tint)
        } else if down.contains(&id) {
            Some(down_tint)
        } else if up.contains(&id) {
            Some(up_tint)
        } else {
            Some(dim_tint)
        }
    };
    // Un cable se resalta si ambos extremos están en el cono resaltado
    // (selección ∪ aguas arriba ∪ aguas abajo); el resto se atenúa.
    let wire_tint_fn = |w: &Wire| -> Option<Color> {
        let sel = selected?;
        let (down, up) = cone.as_ref()?;
        let lit = |n: NodeId| n == sel || down.contains(&n) || up.contains(&n);
        Some(if lit(w.from_node) && lit(w.to_node) {
            wire_hot
        } else {
            wire_dim
        })
    };

    let (node_tint, wire_tint): (
        Option<&dyn Fn(NodeId) -> Option<NodeTint>>,
        Option<&dyn Fn(&Wire) -> Option<Color>>,
    ) = if selected.is_some() {
        (Some(&node_tint_fn), Some(&wire_tint_fn))
    } else {
        (None, None)
    };

    // Capturas estables para la closure de arrastre (clave de persistencia).
    let drag_module_id = module.id.clone();
    let node_names: Vec<String> = data.nodes.iter().map(|n| n.name.clone()).collect();
    // El widget reporta el delta en píxeles de pantalla; lo convertimos a
    // mundo (÷zoom) porque `graph_pos` vive en coords de mundo.
    let drag_zoom = zoom.max(0.001);

    let canvas = nodegraph_view_styled(
        &nodes,
        &wires,
        &palette,
        &metrics,
        // Arrastre de nodo (botón izquierdo): el delta se integra en `update`;
        // al soltar (`End`) se persiste el layout.
        move |id, phase: DragPhase, dx, dy| {
            let morphism = node_names.get(id as usize)?.clone();
            Some(Msg::DragGraphNode {
                module_id: drag_module_id.clone(),
                morphism,
                dx: dx / drag_zoom,
                dy: dy / drag_zoom,
                end: matches!(phase, DragPhase::End),
            })
        },
        // El grafo de morfismos es read-only: no se crean cables a mano
        // (las aristas las dicta el manifest, no la UI).
        |_fn, _fp, _tn, _tp| None,
        // Click-derecho sobre la barra de título: selecciona el cono.
        Some(move |id: NodeId| Some(Msg::SelectGraphNode { mod_idx, id })),
        node_tint,
        wire_tint,
    );

    let n_nodes = data.nodes.len();
    let n_edges = data.edges.len();
    let mut header: Vec<View<Msg>> = vec![text_line(
        format!("{} · {}", module.label, gv.title),
        16.0,
        theme.fg_text,
    )];
    if let Some(sub) = &gv.subtitle {
        header.push(text_line(sub.clone(), 11.0, theme.fg_muted));
    }
    let hint = match selected {
        Some(id) => format!(
            "{n_nodes} morfismos · {n_edges} aristas — resaltando el cono de «{}» (ámbar = afecta · turquesa = depende); click-derecho de nuevo para limpiar.",
            nodes[id as usize].label
        ),
        None => format!(
            "{n_nodes} morfismos · {n_edges} aristas de flujo — arrastrá (botón izq.) para reorganizar; rueda para zoom; click-derecho resalta el cono."
        ),
    };
    header.push(text_line(hint, 11.0, theme.fg_muted));
    header.push(graph_zoom_controls(zoom, theme));

    // Lienzo dentro de una caja flex-grow para que ocupe el alto
    // restante bajo el encabezado. Su `paint_with` registra el rect del
    // lienzo en el side-channel de la cámara para que `on_wheel`/`FitGraph`
    // —que no ven el layout— sepan dónde y cuán grande es.
    let canvas_box = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        min_size: Size {
            width: auto(),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .paint_with(|_scene, _ts, rect| crate::camera::canvas_rect_set(rect))
    .children(vec![canvas]);
    header.push(canvas_box);

    column(header, 6.0)
}

/// Fila compacta de controles de cámara del grafo: zoom −/+, el porcentaje
/// actual y «ajustar» (fit-to-view). Co-locada en el encabezado del panel
/// para no sumar una toolbar aparte.
fn graph_zoom_controls(zoom: f32, theme: &Theme) -> View<Msg> {
    use crate::camera::ZOOM_STEP;
    let pal = ButtonPalette::from_theme(theme);
    let mini = || Style {
        size: Size {
            width: length(30.0_f32),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    };
    let pct = View::new(Style {
        size: Size {
            width: length(52.0_f32),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("{}%", (zoom * 100.0).round() as i32),
        12.0,
        theme.fg_muted,
        Alignment::Center,
    );
    chip_row(vec![
        button_styled(
            "−",
            mini(),
            Alignment::Center,
            &pal,
            Msg::ZoomGraph {
                mult: 1.0 / ZOOM_STEP,
                ancla: None,
            },
        ),
        pct,
        button_styled(
            "+",
            mini(),
            Alignment::Center,
            &pal,
            Msg::ZoomGraph {
                mult: ZOOM_STEP,
                ancla: None,
            },
        ),
        button_styled(
            "ajustar",
            btn_style(78.0),
            Alignment::Center,
            &pal,
            Msg::FitGraph,
        ),
    ])
}

/// Posiciones base `(x, y)` de los nodos del grafo de `data`, indexadas
/// por el índice de cada nodo (= su `NodeId`). El rango de un nodo es su
/// profundidad en el DAG de flujo de datos (longest-path desde una
/// fuente); los nodos de un mismo rango se apilan en filas.
pub(crate) fn graph_layout(data: &MorphismGraphData) -> Vec<(f32, f32)> {
    let n = data.nodes.len();
    let idx: BTreeMap<&str, usize> = data
        .nodes
        .iter()
        .enumerate()
        .map(|(i, m)| (m.name.as_str(), i))
        .collect();

    // Rango por relajación acotada (converge en ≤ n pasadas para un DAG;
    // el tope evita un bucle infinito si el flujo de datos tuviera ciclo).
    let mut rank = vec![0u32; n];
    for _ in 0..n {
        let mut changed = false;
        for e in &data.edges {
            if let (Some(&f), Some(&t)) =
                (idx.get(e.from.as_str()), idx.get(e.to.as_str()))
            {
                if rank[t] < rank[f] + 1 {
                    rank[t] = rank[f] + 1;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Fila dentro de cada rango (orden estable por índice de nodo).
    let mut row_in_rank = vec![0u32; n];
    let mut counts: BTreeMap<u32, u32> = BTreeMap::new();
    for (i, slot) in row_in_rank.iter_mut().enumerate() {
        let c = counts.entry(rank[i]).or_insert(0);
        *slot = *c;
        *c += 1;
    }

    (0..n)
        .map(|i| {
            (
                GRAPH_ORIGIN_X + rank[i] as f32 * GRAPH_COL_STEP,
                GRAPH_ORIGIN_Y + row_in_rank[i] as f32 * GRAPH_ROW_STEP,
            )
        })
        .collect()
}

/// Posición base de un nodo del grafo (sin override de drag), recomputada
/// desde el executor del módulo. La usa `update` para integrar el primer
/// delta de un arrastre sobre la posición correcta del layout.
pub(crate) fn graph_base_pos(model: &Model, module_id: &str, morphism: &str) -> (f32, f32) {
    let fallback = (GRAPH_ORIGIN_X, GRAPH_ORIGIN_Y);
    let Some(data) = model
        .backend
        .lock()
        .ok()
        .and_then(|b| b.morphism_graph(module_id))
    else {
        return fallback;
    };
    let Some(idx) = data.nodes.iter().position(|n| n.name == morphism) else {
        return fallback;
    };
    graph_layout(&data).get(idx).copied().unwrap_or(fallback)
}

/// `mod_idx` del módulo cuya vista activa es un grafo de morfismos, o
/// `None` si la vista actual no es un grafo (hay un form/ficha encima, o
/// el menú apunta a otra vista). La usa `on_wheel` para saber si la rueda
/// debe hacer zoom del grafo.
pub(crate) fn active_graph_module(model: &Model) -> Option<usize> {
    if model.form.is_some() || model.detail.is_some() {
        return None;
    }
    let mod_idx = model.selected_module?;
    let menu_idx = model.selected_menu?;
    let module = model.modules.get(mod_idx)?;
    let item = module.menu.get(menu_idx)?;
    matches!(module.views.get(&item.view)?, ModuleView::Graph(_)).then_some(mod_idx)
}

/// Bounding-box mundo `[min, max]` de todos los nodos del grafo del módulo
/// `mod_idx` (posición override o base del layout, más el tamaño del nodo).
/// La usa `FitGraph` para encuadrar. `None` si el módulo no tiene grafo.
pub(crate) fn graph_world_bounds(
    model: &Model,
    mod_idx: usize,
) -> Option<((f32, f32), (f32, f32))> {
    let module = model.modules.get(mod_idx)?;
    let data = model
        .backend
        .lock()
        .ok()
        .and_then(|b| b.morphism_graph(&module.id))?;
    if data.nodes.is_empty() {
        return None;
    }
    let base = graph_layout(&data);
    let metrics = NodegraphMetrics::default();
    let mut min = (f32::MAX, f32::MAX);
    let mut max = (f32::MIN, f32::MIN);
    for (i, n) in data.nodes.iter().enumerate() {
        let (x, y) = model
            .graph_pos
            .get(&(module.id.clone(), n.name.clone()))
            .copied()
            .unwrap_or(base[i]);
        let w = metrics.node_width;
        let h = metrics.node_height(n.reads.len(), n.writes.len());
        min.0 = min.0.min(x);
        min.1 = min.1.min(y);
        max.0 = max.0.max(x + w);
        max.1 = max.1.max(y + h);
    }
    Some((min, max))
}

/// Cono de dependencias de `sel` sobre el grafo dado por `wires` (con
/// `n` nodos cuyos `NodeId` son `0..n`). Devuelve `(aguas_abajo,
/// aguas_arriba)`: los nodos alcanzables siguiendo las aristas hacia
/// adelante (lo que `sel` afecta) y hacia atrás (de lo que depende). El
/// propio `sel` no se incluye en ninguno de los dos conjuntos.
pub(crate) fn graph_cone(
    sel: NodeId,
    wires: &[Wire],
    n: usize,
) -> (BTreeSet<NodeId>, BTreeSet<NodeId>) {
    let mut down_adj: Vec<Vec<NodeId>> = vec![Vec::new(); n];
    let mut up_adj: Vec<Vec<NodeId>> = vec![Vec::new(); n];
    for w in wires {
        let (f, t) = (w.from_node as usize, w.to_node as usize);
        if f < n && t < n {
            down_adj[f].push(w.to_node);
            up_adj[t].push(w.from_node);
        }
    }
    let reach = |adj: &Vec<Vec<NodeId>>| -> BTreeSet<NodeId> {
        let mut seen: BTreeSet<NodeId> = BTreeSet::new();
        let mut stack = vec![sel];
        while let Some(cur) = stack.pop() {
            for &nx in &adj[cur as usize] {
                if nx != sel && seen.insert(nx) {
                    stack.push(nx);
                }
            }
        }
        seen
    };
    (reach(&down_adj), reach(&up_adj))
}

/// Vista `List`: filas reales del store con columnas del manifest,
/// búsqueda (`search_in`), orden por columna, paginación, botones
/// editar/borrar/👁 por fila, `+ Nuevo` y export CSV.
pub(crate) fn build_list_panel(model: &Model, mod_idx: usize, lv: &ListView, theme: &Theme) -> View<Msg> {
    let module = &model.modules[mod_idx];
    // Sostenemos el guard durante el armado para resolver las columnas
    // `ref_entity` a su label legible sin re-lockear por celda.
    let guard = model.backend.lock().ok();
    let records = match guard.as_ref() {
        Some(b) => list_filtered_sorted(
            b,
            lv,
            &model.list_search.text(),
            &model.list_sort,
            model.drill.as_ref(),
        ),
        None => Vec::new(),
    };

    let total = records.len();
    let has_form = find_form_view(module, &lv.entity).is_some();
    let can_search = !lv.search_in.is_empty();

    // Paginación: clamp de la página contra el total filtrado.
    let pages = total.div_ceil(LIST_PAGE_SIZE).max(1);
    let page = model.list_page.min(pages - 1);

    // --- Fila 1: título + contador + Export + Nuevo. ---
    let title = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("{} · {} ({total})", module.label, lv.title),
        16.0,
        theme.fg_text,
        Alignment::Start,
    );
    let mut header_children = vec![title];
    if total > 0 {
        header_children.push(button_styled(
            "exportar CSV",
            btn_style(120.0),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            Msg::ExportCsv {
                entity: lv.entity.clone(),
            },
        ));
    }
    if has_form {
        header_children.push(button_styled(
            "+ Nuevo",
            btn_style(110.0),
            Alignment::Center,
            &accent_btn(theme),
            Msg::NewRecord {
                module_idx: mod_idx,
                entity: lv.entity.clone(),
            },
        ));
    }
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(header_children);

    let mut rows: Vec<View<Msg>> = vec![header];

    // --- Chip de drill-down activo (si filtra esta entity). ---
    if let Some(d) = model.drill.as_ref().filter(|d| d.entity == lv.entity) {
        let op = if d.prefix { "~" } else { "=" };
        rows.push(button_styled(
            format!("⤵ {} {op} {}   ✕ limpiar", d.field, d.label),
            btn_style_auto(),
            Alignment::Center,
            &accent_btn(theme),
            Msg::ClearDrill,
        ));
    }

    // --- Caja de búsqueda (sólo si la lista declara search_in). ---
    if can_search {
        rows.push(text_input_view(
            &model.list_search,
            &format!("buscar en {}…", lv.search_in.join(", ")),
            model.list_search_focused,
            &TextInputPalette::from_theme(theme),
            Msg::FocusListSearch,
        ));
    }

    // --- Fila de headers de columna (clickeables para ordenar). ---
    let mut head_cells: Vec<View<Msg>> = vec![cell_text("id".into(), 90.0, theme.fg_muted)];
    for col in &lv.columns {
        let arrow = match &model.list_sort {
            Some((f, true)) if *f == col.field => " ▲",
            Some((f, false)) if *f == col.field => " ▼",
            _ => "",
        };
        head_cells.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(22.0_f32),
                },
                flex_grow: 1.0,
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(
                format!("{}{arrow}", col.label),
                12.0,
                theme.fg_muted,
                Alignment::Start,
            )
            .on_click(Msg::SortBy(col.field.clone())),
        );
    }
    rows.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(8.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(head_cells),
    );

    if total == 0 {
        let msg = if model.list_search.text().trim().is_empty() {
            "(sin records — usá + Nuevo)"
        } else {
            "(ningún record coincide con la búsqueda)"
        };
        rows.push(text_line(msg.into(), 12.0, theme.fg_muted));
    }

    // --- Filas de la página actual. ---
    for (id, rec) in records
        .iter()
        .skip(page * LIST_PAGE_SIZE)
        .take(LIST_PAGE_SIZE)
    {
        let mut cells: Vec<View<Msg>> = vec![cell_text(short_uuid(id), 90.0, theme.fg_muted)];
        for col in &lv.columns {
            let disp = match guard.as_ref() {
                Some(b) => cell_display(b, col, lookup_field(rec, &col.field)),
                None => render_value(lookup_field(rec, &col.field)),
            };
            cells.push(cell_flex(disp, theme.fg_text));
        }
        if let Some(detail_vk) = &lv.row_detail {
            cells.push(button_styled(
                "👁",
                btn_style(44.0),
                Alignment::Center,
                &ButtonPalette::from_theme(theme),
                Msg::OpenDetail {
                    module_idx: mod_idx,
                    view_key: detail_vk.clone(),
                    entity: lv.entity.clone(),
                    id: *id,
                },
            ));
        }
        if has_form {
            cells.push(button_styled(
                "editar",
                btn_style(70.0),
                Alignment::Center,
                &ButtonPalette::from_theme(theme),
                Msg::EditRecord {
                    module_idx: mod_idx,
                    entity: lv.entity.clone(),
                    id: *id,
                },
            ));
        }
        cells.push(button_styled(
            "borrar",
            btn_style(70.0),
            Alignment::Center,
            &danger_btn(theme),
            Msg::DeleteRecord {
                entity: lv.entity.clone(),
                id: *id,
            },
        ));

        rows.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(30.0_f32),
                },
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(8.0_f32),
                    height: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(cells),
        );
    }

    // --- Controles de paginación (sólo si hay más de una página). ---
    if pages > 1 {
        let prev = button_styled(
            "‹ anterior",
            btn_style(100.0),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            Msg::ListPagePrev,
        );
        let indicator = View::new(Style {
            size: Size {
                width: length(140.0_f32),
                height: length(30.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned(
            format!("página {} de {pages}", page + 1),
            12.0,
            theme.fg_muted,
            Alignment::Center,
        );
        let next = button_styled(
            "siguiente ›",
            btn_style(100.0),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            Msg::ListPageNext,
        );
        rows.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(38.0_f32),
                },
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(8.0_f32),
                    height: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(vec![prev, indicator, next]),
        );
    }

    column(rows, 6.0)
}

/// Próximo estado de orden al clickear el header `field`: la misma
/// columna cicla ascendente → descendente → sin orden; otra arranca asc.
pub(crate) fn next_sort(current: Option<(String, bool)>, field: &str) -> Option<(String, bool)> {
    match current {
        Some((f, true)) if f == field => Some((f, false)),
        Some((f, false)) if f == field => None,
        _ => Some((field.to_string(), true)),
    }
}

/// Filas de una lista tras aplicar búsqueda (`search_in`) y orden.
/// Compartido por el render y el export CSV. La búsqueda compara el
/// valor crudo (`render_value`) de cada `search_in` field, sin distinguir
/// mayúsculas.
pub(crate) fn list_filtered_sorted(
    backend: &NakuiBackend,
    lv: &ListView,
    query: &str,
    sort: &Option<(String, bool)>,
    drill: Option<&DrillFilter>,
) -> Vec<(Uuid, Value)> {
    let mut rows = backend.list_records(&lv.entity);
    // Filtro de drill-down: si hay uno activo para esta entity, recorta
    // a los records cuyo campo coincide con el grupo elegido.
    if let Some(d) = drill {
        if d.entity == lv.entity {
            rows.retain(|(_, v)| match group_key_text(v, &d.field) {
                Some(cell) if d.prefix => cell.starts_with(&d.value),
                Some(cell) => cell == d.value,
                None => false,
            });
        }
    }
    let q = query.trim().to_lowercase();
    if !q.is_empty() && !lv.search_in.is_empty() {
        rows.retain(|(_, v)| {
            lv.search_in.iter().any(|field| {
                lookup_field(v, field)
                    .map(|c| render_value(Some(c)).to_lowercase().contains(&q))
                    .unwrap_or(false)
            })
        });
    }
    if let Some((field, asc)) = sort {
        rows.sort_by(|(_, a), (_, b)| {
            let ord = cmp_values(lookup_field(a, field), lookup_field(b, field));
            if *asc {
                ord
            } else {
                ord.reverse()
            }
        });
    }
    rows
}

/// El `ListView` de la vista seleccionada cuya entity coincide.
pub(crate) fn active_list_view<'a>(m: &'a Model, entity: &str) -> Option<&'a ListView> {
    let module = m.modules.get(m.selected_module?)?;
    let item = module.menu.get(m.selected_menu?)?;
    match module.views.get(&item.view) {
        Some(ModuleView::List(lv)) if lv.entity == entity => Some(lv),
        _ => None,
    }
}

/// Vista `Detail`: ficha de un record. Header con `← Volver` + `✎
/// Editar`, los campos declarados (label · valor, refs resueltas) y las
/// listas de records relacionados (back-references).
pub(crate) fn build_detail_panel(model: &Model, detail: &DetailState, theme: &Theme) -> View<Msg> {
    let Some(module) = model.modules.get(detail.module_idx) else {
        return empty_panel(theme, "módulo inválido");
    };
    let Some(ModuleView::Detail(dv)) = module.views.get(&detail.view_key) else {
        return empty_panel(theme, "la vista de detalle ya no existe en el manifest");
    };

    // Header: título + Volver + Editar.
    let title = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("{} · {}", module.label, dv.title),
        16.0,
        theme.fg_text,
        Alignment::Start,
    );
    let mut header_children = vec![
        title,
        button_styled(
            "← Volver",
            btn_style(100.0),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            Msg::CloseDetail,
        ),
    ];
    if find_form_view(module, &detail.entity).is_some() {
        header_children.push(button_styled(
            "✎ Editar",
            btn_style(100.0),
            Alignment::Center,
            &accent_btn(theme),
            Msg::EditRecord {
                module_idx: detail.module_idx,
                entity: detail.entity.clone(),
                id: detail.id,
            },
        ));
    }
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(10.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(header_children);

    let mut children: Vec<View<Msg>> = vec![header];

    // El cuerpo necesita el backend; lo sostenemos para el armado.
    let guard = model.backend.lock().ok();
    let record = guard
        .as_ref()
        .and_then(|b| b.load_record(&detail.entity, detail.id));

    let Some(record) = record else {
        children.push(text_line(
            format!("el record {} ya no existe.", short_uuid(&detail.id)),
            12.0,
            theme.fg_muted,
        ));
        return column(children, 8.0);
    };

    // Campos del record (label fijo a la izquierda · valor editable
    // in-situ). El Form view del módulo dice qué columnas son editables;
    // click en una de ellas abre el editor en el lugar (no un form aparte).
    let form_view = find_form_view(module, &detail.entity);
    let input_palette = TextInputPalette::from_theme(theme);
    for col in &dv.fields {
        let label = cell_text(col.label.clone(), 160.0, theme.fg_muted);
        let editing = model
            .inline_edit
            .as_ref()
            .filter(|fr| fr.spec.name == col.field);

        let row_children: Vec<View<Msg>> = if let Some(fr) = editing {
            // Campo en edición: editor + confirmar/cancelar en la fila.
            let control = build_inline_control(model, fr, &input_palette, theme);
            let editor_wrap = View::new(Style {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(8.0_f32),
                    height: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(vec![control]);
            vec![
                label,
                editor_wrap,
                button_styled(
                    "✓",
                    btn_style(34.0),
                    Alignment::Center,
                    &accent_btn(theme),
                    Msg::DetailInlineCommit,
                ),
                button_styled(
                    "✗",
                    btn_style(34.0),
                    Alignment::Center,
                    &ButtonPalette::from_theme(theme),
                    Msg::DetailInlineCancel,
                ),
            ]
        } else {
            // Sólo-lectura: clickeable si hay un FieldSpec editable detrás.
            let value = match guard.as_ref() {
                Some(b) => cell_display(b, col, lookup_field(&record, &col.field)),
                None => render_value(lookup_field(&record, &col.field)),
            };
            let editable = form_view.is_some_and(|fv| {
                fv.fields
                    .iter()
                    .any(|fs| fs.name == col.field && fs.kind != FieldKind::AutoId)
            });
            let mut val = cell_flex(value, theme.fg_text);
            if editable {
                val = val.on_click(Msg::DetailEditField {
                    field: col.field.clone(),
                });
            }
            vec![label, val]
        };

        let height: f32 = if editing.is_some() { 34.0 } else { 26.0 };
        children.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(height),
                },
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(12.0_f32),
                    height: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(row_children),
        );
    }

    // KPIs scopeados al record (el "360" de la ficha): stat cards con
    // agregados sobre los records relacionados.
    if !dv.metrics.is_empty() {
        if let Some(b) = guard.as_ref() {
            let cards: Vec<View<Msg>> = dv
                .metrics
                .iter()
                .map(|dm| {
                    let result = compute_detail_metric(b, dm, detail.id);
                    dashboard_card(&dm.label, &result, &dm.format, ChartKind::Bars, None, None, theme)
                })
                .collect();
            children.push(
                View::new(Style {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
                    size: Size {
                        width: percent(1.0_f32),
                        height: auto(),
                    },
                    gap: Size {
                        width: length(12.0_f32),
                        height: length(12.0_f32),
                    },
                    ..Default::default()
                })
                .children(cards),
            );
        }
    }

    // Listas de records relacionados.
    for rl in &dv.related {
        if let Some(b) = guard.as_ref() {
            children.push(build_related_list(b, rl, detail.id, theme));
        }
    }

    column(children, 8.0)
}

/// Computa un [`DetailMetric`]: agrega sobre los records de `dm.entity`
/// cuyo `dm.via_field` referencia al record `target_id` (mismo scope que
/// una [`RelatedList`]), con el `dm.filter` opcional como AND adicional.
pub(crate) fn compute_detail_metric(
    backend: &NakuiBackend,
    dm: &DetailMetric,
    target_id: Uuid,
) -> MetricResult {
    let id_str = target_id.to_string();
    let records: Vec<(Uuid, Value)> = backend
        .list_records(&dm.entity)
        .into_iter()
        .filter(|(_, v)| v.get(&dm.via_field).and_then(Value::as_str) == Some(id_str.as_str()))
        .collect();
    compute_metric(&dm.metric, dm.filter.as_ref(), &records)
}

/// Una lista de back-references dentro de una ficha: los records de
/// `rl.entity` cuyo `rl.via_field` apunta al record `target_id`.
pub(crate) fn build_related_list(
    backend: &NakuiBackend,
    rl: &RelatedList,
    target_id: Uuid,
    theme: &Theme,
) -> View<Msg> {
    let id_str = target_id.to_string();
    let rows: Vec<(Uuid, Value)> = backend
        .list_records(&rl.entity)
        .into_iter()
        .filter(|(_, v)| v.get(&rl.via_field).and_then(Value::as_str) == Some(id_str.as_str()))
        .collect();

    let mut children: Vec<View<Msg>> = vec![text_line(
        format!("{} ({})", rl.title, rows.len()),
        13.0,
        theme.fg_text,
    )];

    if rows.is_empty() {
        children.push(text_line("(ninguno)".into(), 11.0, theme.fg_muted));
    } else {
        // Header de columnas.
        let head_cells: Vec<View<Msg>> = rl
            .columns
            .iter()
            .map(|c| cell_flex(c.label.clone(), theme.fg_muted))
            .collect();
        children.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0_f32),
                },
                gap: Size {
                    width: length(8.0_f32),
                    height: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(head_cells),
        );

        for (_, v) in &rows {
            let cells: Vec<View<Msg>> = rl
                .columns
                .iter()
                .map(|c| {
                    cell_flex(cell_display(backend, c, lookup_field(v, &c.field)), theme.fg_text)
                })
                .collect();
            children.push(
                View::new(Style {
                    flex_direction: FlexDirection::Row,
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(22.0_f32),
                    },
                    gap: Size {
                        width: length(8.0_f32),
                        height: length(0.0_f32),
                    },
                    ..Default::default()
                })
                .children(cells),
            );
        }
    }

    // Bloque que se ajusta al contenido, con un poco de aire arriba.
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_shrink: 0.0,
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(10.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

/// Render del valor de una celda. Una columna con `ref_entity` resuelve
/// su UUID al label del record referido; el resto aplica el
/// `ValueFormat` de la columna. Espejo del `render_cell` GPUI.
pub(crate) fn cell_display(backend: &NakuiBackend, col: &Column, v: Option<&Value>) -> String {
    if let Some(ref_entity) = &col.ref_entity {
        return match v {
            Some(Value::String(s)) => match Uuid::parse_str(s) {
                Ok(uuid) => backend
                    .load_record(ref_entity, uuid)
                    .map(|rec| human_label_for_record(&rec, &uuid))
                    .unwrap_or_else(|| format!("(borrado · {})", short_uuid(&uuid))),
                Err(_) => render_value(v),
            },
            _ => render_value(v),
        };
    }
    format_value(v, &col.format)
}

/// Navega un path con puntos (`address.city`) dentro de un `Value`.
pub(crate) fn lookup_field<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// Panel del formulario activo: un `field_view` por field + fila de
/// acciones (Cancelar / Guardar) + banner de error.
pub(crate) fn build_form_panel(model: &Model, form: &FormState, theme: &Theme) -> View<Msg> {
    let module = model.modules.get(form.module_idx);
    let module_label = module.map(|m| m.label.as_str()).unwrap_or("");
    let mode = if form.editing.is_some() {
        "editar"
    } else {
        "nuevo"
    };
    let title = text_line(
        format!("{module_label} · {} ({mode})", form.title),
        16.0,
        theme.fg_text,
    );

    let field_palette = FieldPalette::from_theme(theme);
    let input_palette = TextInputPalette::from_theme(theme);

    let mut children: Vec<View<Msg>> = vec![title];

    for (i, fr) in form.fields.iter().enumerate() {
        let focused = form.focused == Some(i);
        let control = build_field_control(model, fr, i, focused, &input_palette, theme);
        children.push(field_view(FieldWidgetSpec {
            label: fr.spec.label.clone(),
            control,
            required: fr.spec.required,
            helper: fr.spec.help.clone(),
            error: None,
            palette: field_palette,
        }));
    }

    if let Some(err) = &form.error {
        children.push(banner_view::<Msg>(BannerKind::Error, err.clone()));
    }

    // Fila de acciones.
    let actions = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(38.0_f32),
        },
        gap: Size {
            width: length(10.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        button_styled(
            "Cancelar",
            btn_style(120.0),
            Alignment::Center,
            &ButtonPalette::from_theme(theme),
            Msg::CancelForm,
        ),
        button_styled(
            if form.editing.is_some() {
                "Guardar"
            } else {
                "Crear"
            },
            btn_style(120.0),
            Alignment::Center,
            &accent_btn(theme),
            Msg::SubmitForm,
        ),
    ]);
    children.push(actions);

    column(children, 10.0)
}

/// Control de edición in-situ de un campo en la ficha de detalle.
/// Espeja [`build_field_control`] pero el input siempre va con foco y los
/// mensajes son los `DetailInline*` (no los del form indexado).
pub(crate) fn build_inline_control(
    model: &Model,
    fr: &FieldRuntime,
    input_palette: &TextInputPalette,
    theme: &Theme,
) -> View<Msg> {
    match fr.spec.kind {
        FieldKind::Text
        | FieldKind::Multiline
        | FieldKind::Number
        | FieldKind::Date
        | FieldKind::Array => {
            let placeholder = fr.spec.help.clone().unwrap_or_default();
            text_input_view(
                &fr.input,
                &placeholder,
                true,
                input_palette,
                Msg::DetailInlineFocus,
            )
        }
        FieldKind::Boolean => {
            let on = fr.raw() == "true";
            let pal = if on {
                accent_btn(theme)
            } else {
                ButtonPalette::from_theme(theme)
            };
            button_styled(
                if on { "Sí" } else { "No" },
                btn_style(80.0),
                Alignment::Center,
                &pal,
                Msg::DetailInlineSet(if on { "false" } else { "true" }.to_string()),
            )
        }
        FieldKind::AutoId => cell_flex(fr.raw(), theme.fg_muted),
        FieldKind::Select => {
            let current = fr.raw();
            let chips: Vec<View<Msg>> = fr
                .spec
                .options
                .iter()
                .map(|opt| {
                    let selected = current == opt.value;
                    let pal = if selected {
                        accent_btn(theme)
                    } else {
                        ButtonPalette::from_theme(theme)
                    };
                    button_styled(
                        opt.display().to_string(),
                        btn_style_auto(),
                        Alignment::Center,
                        &pal,
                        Msg::DetailInlineSet(opt.value.clone()),
                    )
                })
                .collect();
            chip_row(chips)
        }
        FieldKind::EntityRef => {
            let target = fr.spec.ref_entity.clone().unwrap_or_default();
            let current = fr.raw();
            let records = model
                .backend
                .lock()
                .map(|b| b.list_records(&target))
                .unwrap_or_default();
            let total = records.len();
            let mut chips: Vec<View<Msg>> = records
                .iter()
                .take(ENTITY_REF_LIMIT)
                .map(|(id, rec)| {
                    let id_str = id.to_string();
                    let selected = current == id_str;
                    let label = entity_ref_label(id, rec);
                    let pal = if selected {
                        accent_btn(theme)
                    } else {
                        ButtonPalette::from_theme(theme)
                    };
                    button_styled(
                        label,
                        btn_style_auto(),
                        Alignment::Center,
                        &pal,
                        Msg::DetailInlineSet(id_str),
                    )
                })
                .collect();
            if total == 0 {
                chips.push(cell_text(
                    format!("(sin records en '{target}')"),
                    240.0,
                    theme.fg_muted,
                ));
            } else if total > ENTITY_REF_LIMIT {
                chips.push(cell_text(
                    format!("… +{} más", total - ENTITY_REF_LIMIT),
                    120.0,
                    theme.fg_muted,
                ));
            }
            chip_row(chips)
        }
    }
}

/// Renderea el control de un field según su `FieldKind`.
pub(crate) fn build_field_control(
    model: &Model,
    fr: &FieldRuntime,
    i: usize,
    focused: bool,
    input_palette: &TextInputPalette,
    theme: &Theme,
) -> View<Msg> {
    match fr.spec.kind {
        FieldKind::Text
        | FieldKind::Multiline
        | FieldKind::Number
        | FieldKind::Date
        | FieldKind::Array => {
            let placeholder = fr.spec.help.clone().unwrap_or_default();
            text_input_view(
                &fr.input,
                &placeholder,
                focused,
                input_palette,
                Msg::FocusField(i),
            )
        }
        FieldKind::Boolean => {
            let on = fr.raw() == "true";
            let pal = if on {
                accent_btn(theme)
            } else {
                ButtonPalette::from_theme(theme)
            };
            button_styled(
                if on { "Sí" } else { "No" },
                btn_style(80.0),
                Alignment::Center,
                &pal,
                Msg::ToggleBool(i),
            )
        }
        FieldKind::AutoId => {
            // Read-only: el UUID autogenerado, sin foco.
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(28.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(fr.raw(), 12.0, theme.fg_muted, Alignment::Start)
        }
        FieldKind::Select => {
            let current = fr.raw();
            let chips: Vec<View<Msg>> = fr
                .spec
                .options
                .iter()
                .map(|opt| {
                    let selected = current == opt.value;
                    let pal = if selected {
                        accent_btn(theme)
                    } else {
                        ButtonPalette::from_theme(theme)
                    };
                    button_styled(
                        opt.display().to_string(),
                        btn_style_auto(),
                        Alignment::Center,
                        &pal,
                        Msg::SetSelect(i, opt.value.clone()),
                    )
                })
                .collect();
            chip_row(chips)
        }
        FieldKind::EntityRef => {
            let target = fr.spec.ref_entity.clone().unwrap_or_default();
            let current = fr.raw();
            let records = model
                .backend
                .lock()
                .map(|b| b.list_records(&target))
                .unwrap_or_default();
            let total = records.len();
            let mut chips: Vec<View<Msg>> = records
                .iter()
                .take(ENTITY_REF_LIMIT)
                .map(|(id, rec)| {
                    let id_str = id.to_string();
                    let selected = current == id_str;
                    let label = entity_ref_label(id, rec);
                    let pal = if selected {
                        accent_btn(theme)
                    } else {
                        ButtonPalette::from_theme(theme)
                    };
                    button_styled(
                        label,
                        btn_style_auto(),
                        Alignment::Center,
                        &pal,
                        Msg::SetSelect(i, id_str),
                    )
                })
                .collect();
            if total == 0 {
                chips.push(cell_text(
                    format!("(sin records en '{target}')"),
                    240.0,
                    theme.fg_muted,
                ));
            } else if total > ENTITY_REF_LIMIT {
                chips.push(cell_text(
                    format!("… +{} más", total - ENTITY_REF_LIMIT),
                    120.0,
                    theme.fg_muted,
                ));
            }
            chip_row(chips)
        }
    }
}
