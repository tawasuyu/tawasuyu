//! Fase 7.849 — tamaño intrínseco (`width: min-content|max-content|fit-content`).
//!
//! Verifica las tres capas: (1) el engine parsea el keyword a
//! `LengthVal::*Content` y lo deja en `BoxNode.width`; (2) el puente
//! `box_style` lo traduce a `width: auto` + `align_self: Start` (supresión del
//! stretch del padre); (3) END-TO-END: montado en taffy y computado, una caja
//! `width: max-content` se ENCOGE a su contenido en vez de llenar el ancho del
//! contenedor (el bug previo). Esto último es la evidencia de render real, no
//! sólo decl-level.
#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use super::super::*;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_text::{Alignment, Typesetter};
use llimphi_ui::llimphi_layout::taffy::prelude::{auto, length, percent, FlexDirection, Size, Style};
use puriy_engine::style::LengthVal;

/// Busca recursivamente el primer `BoxNode` con el tag dado.
fn find_tag<'a>(b: &'a BoxNode, tag: &str) -> Option<&'a BoxNode> {
    if b.tag.as_deref() == Some(tag) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find_tag(c, tag))
}

#[test]
fn intrinsic_width_parsea_y_traduce_en_box_style() {
    // (1) El engine deja el keyword en BoxNode.width.
    let tree = parse(r#"<body><div style="width: max-content">hola</div></body>"#);
    let div = find_tag(&tree.root, "div").expect("div");
    assert_eq!(div.width, LengthVal::MaxContent, "width debe ser MaxContent");

    // (2) box_style: width → auto (no percent(1.0) del default de bloque) y
    // align_self → Start (no estira en el padre flex column).
    let st = box_style(div, 1.0);
    assert_eq!(st.size.width, auto(), "width intrínseca → auto en taffy");
    assert_eq!(
        st.align_self,
        Some(taffy::style::AlignSelf::Start),
        "align_self → Start para evitar el stretch del padre"
    );

    // Control: un bloque normal mantiene el default percent(1.0) (ancho lleno).
    let tree2 = parse(r#"<body><div>hola</div></body>"#);
    let div2 = find_tag(&tree2.root, "div").expect("div2");
    assert_eq!(div2.width, LengthVal::Auto);
    let st2 = box_style(div2, 1.0);
    assert_eq!(st2.size.width, percent(1.0), "bloque normal llena el ancho");
}

#[test]
fn min_max_fit_content_todos_parsean() {
    for (css, expected) in [
        ("min-content", LengthVal::MinContent),
        ("max-content", LengthVal::MaxContent),
        ("fit-content", LengthVal::FitContent),
        ("fit-content(200px)", LengthVal::FitContent),
    ] {
        let tree = parse(&format!(r#"<body><div style="width: {css}">x</div></body>"#));
        let div = find_tag(&tree.root, "div").expect("div");
        assert_eq!(div.width, expected, "width: {css}");
    }
}

/// Computa el layout de un `View` y devuelve el ancho del nodo hijo directo
/// `idx` de la raíz. Usa la misma secuencia que el eventloop / `dump_container`
/// (mount → compute_with_measure con `measure_text_node`).
fn child_width(view: View<Msg>, viewport_w: f32, idx: usize) -> f32 {
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, view);
    let mut ts = Typesetter::new();
    let tmap = &mounted.text_measures;
    layout
        .compute_with_measure(mounted.root, (viewport_w, 2000.0), |nid, known, avail| {
            match tmap.get(&nid) {
                Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                None => taffy::Size::ZERO,
            }
        })
        .expect("layout");
    let tree = layout.inner();
    let child = tree.child_at_index(mounted.root, idx).expect("hijo");
    tree.layout(child).expect("layout hijo").size.width
}

/// Construye un `View` de una caja bloque con una hoja de texto, usando el
/// `box_style` real. `width_kw` controla el `width` del BoxNode.
fn caja_con_texto(width: LengthVal, texto: &str) -> View<Msg> {
    // BoxNode del div: lo obtenemos de un parseo y le forzamos el width, así
    // no construimos el struct gigante a mano (no tiene Default).
    let tree = parse(&format!(r#"<body><div>{texto}</div></body>"#));
    let div = find_tag(&tree.root, "div").expect("div").clone();
    let mut div = div;
    div.width = width;
    let text_child = div.children.first().expect("hoja de texto").clone();

    let leaf = View::<Msg>::new(box_style(&text_child, 1.0)).text_aligned(
        texto.to_string(),
        text_child.font_size,
        llimphi_raster::peniko::Color::BLACK,
        Alignment::Start,
    );
    View::<Msg>::new(box_style(&div, 1.0)).children(vec![leaf])
}

#[test]
fn max_content_encoge_vs_bloque_normal() {
    // Contenedor de 600px, columna (como un bloque). El texto a 16px ocupa
    // mucho menos de 600px.
    const VPW: f32 = 600.0;
    let texto = "contenido de prueba intrinseco";

    let parent = |child: View<Msg>| {
        View::<Msg>::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: length(VPW), height: auto() },
            ..Default::default()
        })
        .children(vec![child])
    };

    // max-content: la caja se encoge a su contenido (< mitad del contenedor).
    let w_max = child_width(parent(caja_con_texto(LengthVal::MaxContent, texto)), VPW, 0);
    // bloque normal: llena el ancho del contenedor.
    let w_block = child_width(parent(caja_con_texto(LengthVal::Auto, texto)), VPW, 0);

    assert!(
        w_block >= VPW - 1.0,
        "bloque normal debe llenar el contenedor: {w_block} (vpw {VPW})"
    );
    assert!(
        w_max < VPW * 0.6,
        "max-content debe encogerse al contenido: {w_max} (vpw {VPW})"
    );
    assert!(w_max > 0.0, "max-content no debe colapsar a 0: {w_max}");
}
