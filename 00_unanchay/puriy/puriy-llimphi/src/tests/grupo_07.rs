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
