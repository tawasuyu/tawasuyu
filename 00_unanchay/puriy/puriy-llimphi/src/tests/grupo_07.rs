//! Fase 7.911 — puente de LAYOUT de grid (BoxNode → taffy Style).
//!
//! Antes de este lote `box_style` sólo pasaba `grid-template-{columns,rows}`
//! a taffy; `gap`, `grid-auto-flow`, `grid-auto-{rows,columns}` y la colocación
//! `grid-row`/`grid-column` de los ítems se TIRABAN (parseaban en el engine
//! pero nunca llegaban al layout). Estos tests montan la grilla en taffy y
//! computan posiciones reales — evidencia de layout, no decl-level.
#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use super::super::*;

use llimphi_ui::llimphi_compositor::mount;
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::View;

/// Busca recursivamente el primer `BoxNode` con el tag dado.
fn find_tag<'a>(b: &'a BoxNode, tag: &str) -> Option<&'a BoxNode> {
    if b.tag.as_deref() == Some(tag) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find_tag(c, tag))
}

/// Convierte un subárbol de `BoxNode` en `View<Msg>` usando el `box_style`
/// real, descartando las hojas de texto (sólo cajas-elemento con tamaño
/// explícito → no hace falta medir texto).
fn box_to_view(b: &BoxNode) -> View<Msg> {
    let children: Vec<View<Msg>> = b
        .children
        .iter()
        .filter(|c| c.text.is_none())
        .map(box_to_view)
        .collect();
    View::<Msg>::new(box_style(b, 1.0)).children(children)
}

/// Monta el `View`, computa layout contra `viewport`, y devuelve el `Layout`
/// (location + size) del nodo alcanzado por `path` (índices de hijo desde la
/// raíz montada).
fn layout_at(view: View<Msg>, viewport: (f32, f32), path: &[usize]) -> taffy::Layout {
    let mut tree = LayoutTree::new();
    let mounted = mount(&mut tree, view);
    tree.compute_with_measure(mounted.root, viewport, |_nid, _known, _avail| taffy::Size::ZERO)
        .expect("layout");
    let inner = tree.inner();
    let mut node = mounted.root;
    for &i in path {
        node = inner.child_at_index(node, i).expect("hijo en path");
    }
    *inner.layout(node).expect("layout del nodo")
}

/// Grilla `display:grid` con `extra` CSS en el contenedor y dos ítems de
/// 40×30 px. Devuelve el `View` del contenedor.
fn grid(extra: &str) -> View<Msg> {
    let html = format!(
        r#"<body><div style="display:grid; {extra}">
             <div style="width:40px;height:30px">a</div>
             <div style="width:40px;height:30px">b</div>
           </div></body>"#
    );
    let tree = parse(&html);
    let cont = find_tag(&tree.root, "div").expect("contenedor grid");
    box_to_view(cont)
}

#[test]
fn gap_de_grid_separa_columnas() {
    // 2 columnas de 40px con gap 20px → el 2º ítem arranca en x = 40 + 20 = 60.
    let view = grid("grid-template-columns: 40px 40px; gap: 20px");
    let item1 = layout_at(view, (400.0, 400.0), &[1]);
    assert!(
        (item1.location.x - 60.0).abs() < 0.5,
        "2º ítem debe estar en x≈60 (40px col + 20px gap), está en {}",
        item1.location.x
    );
}

#[test]
fn gap_cero_sin_gap() {
    // Control: sin gap el 2º ítem pega en x = 40 (ancho de la 1ª columna).
    let view = grid("grid-template-columns: 40px 40px");
    let item1 = layout_at(view, (400.0, 400.0), &[1]);
    assert!(
        (item1.location.x - 40.0).abs() < 0.5,
        "sin gap el 2º ítem debe estar en x≈40, está en {}",
        item1.location.x
    );
}

#[test]
fn auto_rows_dimensiona_pistas_implicitas() {
    // 1 sola columna explícita → el 2º ítem desborda a una fila IMPLÍCITA.
    // `grid-auto-rows: 80px` fija la altura de esa fila a 80, así que el 2º
    // ítem arranca en y = 80 (no en 30 = su contenido). Antes del puente las
    // auto-rows se tiraban y la fila medía por contenido (≈30).
    let view = grid("grid-template-columns: 40px; grid-auto-rows: 80px");
    let item1 = layout_at(view, (400.0, 400.0), &[1]);
    assert!(
        (item1.location.y - 80.0).abs() < 0.5,
        "2º ítem debe estar en y≈80 (auto-row de 80px), está en {}",
        item1.location.y
    );
}

#[test]
fn auto_flow_column_apila_en_columnas() {
    // `grid-auto-flow: column` con 1 fila explícita → los ítems fluyen por
    // columnas: el 2º va a la 2ª columna implícita (x>0), misma fila (y≈0).
    let view = grid("grid-template-rows: 30px; grid-auto-flow: column; grid-auto-columns: 40px");
    let item1 = layout_at(view, (400.0, 400.0), &[1]);
    assert!(
        item1.location.x > 1.0 && item1.location.y < 1.0,
        "auto-flow column: 2º ítem en otra columna (x>0,y≈0), está en ({},{})",
        item1.location.x,
        item1.location.y
    );
}

#[test]
fn grid_template_areas_coloca_por_nombre() {
    // Grilla 2×2 (50px cols, 30px rows) con áreas nombradas:
    //   "head head"
    //   "side main"
    // El ítem en `grid-area: main` aterriza en la celda inferior-derecha
    // (x≈50, y≈30). El ítem `head` ocupa la fila superior entera (ancho≈100).
    let html = r##"<html><head><style>
        .g { display:grid; grid-template-columns: 50px 50px; grid-template-rows: 30px 30px;
             grid-template-areas: "head head" "side main"; }
        .h { grid-area: head; } .m { grid-area: main; }
      </style></head><body><div class="g">
        <div class="h">h</div>
        <div class="m">m</div>
      </div></body></html>"##;
    let tree = parse(html);
    let cont = find_tag(&tree.root, "div").expect("contenedor grid");
    // Sanity: el engine parseó las áreas al BoxNode (con comillas crudas).
    assert!(
        cont.grid_template_areas.as_deref().unwrap_or("").contains("head"),
        "el engine debe dejar grid-template-areas en el BoxNode"
    );
    let view = box_to_view(cont);

    let head = layout_at(view, (400.0, 400.0), &[0]);
    assert!(head.location.x < 1.0 && head.location.y < 1.0, "head arriba-izq, está en ({},{})", head.location.x, head.location.y);
    assert!((head.size.width - 100.0).abs() < 0.5, "head ocupa ambas columnas (ancho≈100), mide {}", head.size.width);

    // Re-parsear para un View fresco (el View se consumió en el mount).
    let view2 = box_to_view(cont);
    let main = layout_at(view2, (400.0, 400.0), &[1]);
    assert!((main.location.x - 50.0).abs() < 0.5, "main en 2ª columna (x≈50), está en {}", main.location.x);
    assert!((main.location.y - 30.0).abs() < 0.5, "main en 2ª fila (y≈30), está en {}", main.location.y);
}

#[test]
fn parse_areas_calcula_coordenadas() {
    // Verificación directa del parser de áreas (sin layout).
    let areas = parse_grid_template_areas("\"head head\" \"side main\"");
    let get = |n: &str| areas.iter().find(|a| a.name == n).expect(n);
    let head = get("head");
    assert_eq!((head.row_start, head.row_end), (1, 2), "head: fila 1");
    assert_eq!((head.column_start, head.column_end), (1, 3), "head: cols 1..3");
    let main = get("main");
    assert_eq!((main.row_start, main.row_end), (2, 3), "main: fila 2");
    assert_eq!((main.column_start, main.column_end), (2, 3), "main: col 2");
}

#[test]
fn grid_minmax_respeta_piso_y_crece() {
    // 1 columna minmax(100px, 1fr) en un grid de 300px → el track crece a 1fr
    // = 300px (mayor que el piso). En un grid de 60px → el track respeta el
    // PISO de 100px (no se encoge bajo el mínimo). Antes (flatten a 1fr) el
    // track de 60px habría medido 60, perdiendo el piso.
    let mk = |w: u32| {
        let html = format!(
            r##"<html><head><style>
              .g {{ display:grid; width:{w}px; grid-template-columns: minmax(100px, 1fr); }}
              .c {{ height:20px; }}
            </style></head><body><div class="g"><div class="c">x</div></div></body></html>"##
        );
        let tree = parse(&html);
        let cont = find_tag(&tree.root, "div").expect("grid").clone();
        layout_at(box_to_view(&cont), (400.0, 400.0), &[0]).size.width
    };
    let ancho = mk(300);
    let angosto = mk(60);
    assert!((ancho - 300.0).abs() < 1.0, "minmax crece a 1fr=300, mide {ancho}");
    assert!((angosto - 100.0).abs() < 1.0, "minmax respeta el piso 100px, mide {angosto}");
}

#[test]
fn margin_top_auto_empuja_en_flex() {
    // Contenedor flex column de 200px de alto; el ítem (30px alto) con
    // `margin-top: auto` se empuja al fondo (y ≈ 200 − 30 = 170). En block
    // flow el mismo margin-top:auto computaría a 0 (ver el control abajo).
    let html = r##"<html><head><style>
        .f { display:flex; flex-direction:column; height:200px; }
        .i { height:30px; margin-top:auto; }
      </style></head><body><div class="f"><div class="i">x</div></div></body></html>"##;
    let tree = parse(html);
    let cont = find_tag(&tree.root, "div").expect("flex");
    // El engine resolvió el flag contra el padre flex.
    let item = cont.children.iter().find(|c| c.text.is_none()).expect("item");
    assert!(item.margin_top_auto, "el flag debe sobrevivir bajo padre flex");
    let view = box_to_view(cont);
    let l = layout_at(view, (400.0, 400.0), &[0]);
    assert!(
        (l.location.y - 170.0).abs() < 1.0,
        "margin-top:auto empuja al fondo (y≈170), está en {}",
        l.location.y
    );
}

#[test]
fn margin_top_auto_no_centra_en_block() {
    // Control: en block flow `margin-top:auto` NO centra (CSS lo computa a 0).
    // El build apaga el flag porque el padre no es flex/grid.
    let html = r##"<html><head><style>
        .b { height:200px; }
        .i { height:30px; margin-top:auto; }
      </style></head><body><div class="b"><div class="i">x</div></div></body></html>"##;
    let tree = parse(html);
    let cont = find_tag(&tree.root, "div").expect("block");
    let item = cont.children.iter().find(|c| c.text.is_none()).expect("item");
    assert!(!item.margin_top_auto, "en block flow el flag se apaga en el build");
    let view = box_to_view(cont);
    let l = layout_at(view, (400.0, 400.0), &[0]);
    assert!(l.location.y < 1.0, "block: el ítem queda arriba (y≈0), está en {}", l.location.y);
}

#[test]
fn border_reserva_espacio_en_layout() {
    // Caja con border:10px + padding:5px; su único hijo debe arrancar a
    // 15px del borde externo (no a 5px). Antes el borde no se reservaba en
    // taffy y el hijo quedaba a 5px (el borde se pintaba encima).
    let html = r##"<html><head><style>
        .outer { width:200px; border:10px solid black; padding:5px; }
        .inner { width:50px; height:20px; }
      </style></head><body><div class="outer"><div class="inner">x</div></div></body></html>"##;
    let tree = parse(html);
    let outer = find_tag(&tree.root, "div").expect("outer");
    let view = box_to_view(outer);
    let inner = layout_at(view, (400.0, 400.0), &[0]);
    assert!(
        (inner.location.x - 15.0).abs() < 0.5 && (inner.location.y - 15.0).abs() < 0.5,
        "hijo a border(10)+padding(5)=15 del borde, está en ({},{})",
        inner.location.x, inner.location.y
    );
}

#[test]
fn border_box_descuenta_borde_del_ancho() {
    // box-sizing:border-box + width:200px + border:20px → el área de contenido
    // mide 200 − 2·20 = 160. El hijo a 100% llena 160, no 200.
    let html = r##"<html><head><style>
        .outer { width:200px; box-sizing:border-box; border:20px solid black; }
        .inner { width:100%; height:10px; }
      </style></head><body><div class="outer"><div class="inner">x</div></div></body></html>"##;
    let tree = parse(html);
    let outer = find_tag(&tree.root, "div").expect("outer");
    let view = box_to_view(outer);
    let inner = layout_at(view, (400.0, 400.0), &[0]);
    assert!(
        (inner.size.width - 160.0).abs() < 0.5,
        "contenido = 200 − 2·20 = 160, mide {}",
        inner.size.width
    );
}

#[test]
fn grid_align_items_alinea_en_celda() {
    // Celda de 40×80; el ítem (auto, mide 40×30 por su contenido fijo) con
    // `align-items: end` en el contenedor se baja al fondo de la celda
    // (y ≈ 80 − 30 = 50). Antes del fix align-items se ignoraba en grid y el
    // ítem quedaba arriba (y ≈ 0, estirado o al tope).
    let html = r##"<html><head><style>
        .g { display:grid; grid-template-columns: 40px; grid-template-rows: 80px; align-items: end; }
        .c { width:40px; height:30px; }
      </style></head><body><div class="g"><div class="c">x</div></div></body></html>"##;
    let tree = parse(html);
    let cont = find_tag(&tree.root, "div").expect("grid");
    let view = box_to_view(cont);
    let item = layout_at(view, (400.0, 400.0), &[0]);
    assert!(
        (item.location.y - 50.0).abs() < 1.0,
        "align-items:end baja el ítem al fondo (y≈50), está en {}",
        item.location.y
    );
}

#[test]
fn grid_justify_content_distribuye_pistas() {
    // 1 columna de 40px en un grid de 200px de ancho con `justify-content: end`
    // → la pista se empuja a la derecha (x ≈ 200 − 40 = 160). Sin el fix la
    // pista quedaba a la izquierda (x ≈ 0).
    let html = r##"<html><head><style>
        .g { display:grid; width:200px; grid-template-columns: 40px; justify-content: end; }
        .c { width:40px; height:30px; }
      </style></head><body><div class="g"><div class="c">x</div></div></body></html>"##;
    let tree = parse(html);
    let cont = find_tag(&tree.root, "div").expect("grid");
    let view = box_to_view(cont);
    let item = layout_at(view, (400.0, 400.0), &[0]);
    assert!(
        (item.location.x - 160.0).abs() < 1.0,
        "justify-content:end empuja la pista a la derecha (x≈160), está en {}",
        item.location.x
    );
}

#[test]
fn grid_column_coloca_item_explicito() {
    // Grilla de 3 columnas de 40px. El 1er ítem pide `grid-column: 3` → salta
    // a la 3ª columna (x ≈ 80). Antes del puente la colocación se ignoraba y
    // caía en la 1ª (x≈0).
    let html = r#"<body><div style="display:grid; grid-template-columns: 40px 40px 40px">
        <div style="width:40px;height:30px; grid-column: 3">a</div>
        <div style="width:40px;height:30px">b</div>
      </div></body>"#;
    let tree = parse(html);
    let cont = find_tag(&tree.root, "div").expect("contenedor grid");
    // Sanity: el engine sí parseó la colocación al BoxNode.
    let item0 = cont.children.iter().find(|c| c.text.is_none()).expect("item0");
    assert_eq!(
        item0.grid_column_start.as_deref(),
        Some("3"),
        "el engine debe dejar grid-column-start=3 en el BoxNode"
    );
    let view = box_to_view(cont);
    let item0_l = layout_at(view, (400.0, 400.0), &[0]);
    assert!(
        (item0_l.location.x - 80.0).abs() < 0.5,
        "ítem con grid-column:3 debe estar en x≈80, está en {}",
        item0_l.location.x
    );
}
