//! Verifica que un párrafo largo, dentro de un bloque angosto, reserva el
//! alto de **varias líneas** (no se aplasta en una). Es el regresor del bug
//! "textos aplastados" de puriy: sin medición con parley, taffy le daba a la
//! hoja de texto una sola línea de alto y las líneas envueltas se solapaban.

use llimphi_compositor::{measure_text_node, mount, View};
use llimphi_layout::taffy::prelude::*;
use llimphi_layout::taffy::Size as TSize;
use llimphi_layout::LayoutTree;

#[derive(Clone)]
enum Msg {}

#[test]
fn parrafo_largo_reserva_varias_lineas() {
    // Bloque de 200px de ancho con un párrafo que claramente excede una línea.
    let texto = "Lorem ipsum dolor sit amet consectetur adipiscing elit sed do \
                 eiusmod tempor incididunt ut labore et dolore magna aliqua ut \
                 enim ad minim veniam quis nostrud exercitation ullamco laboris.";
    let block: View<Msg> = View::new(Style {
        size: TSize { width: length(200.0_f32), height: auto() },
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        ..Default::default()
    })
    .children(vec![View::new(Style {
        size: TSize { width: auto(), height: auto() },
        flex_shrink: 1.0,
        ..Default::default()
    })
    .text_aligned(texto, 16.0_f32, vello::peniko::Color::BLACK, llimphi_text::Alignment::Start)]);

    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, block);
    let mut ts = llimphi_text::Typesetter::new();
    let tmap = &mounted.text_measures;
    assert_eq!(tmap.len(), 1, "debería haber exactamente una hoja de texto");

    let computed = layout
        .compute_with_measure(mounted.root, (800.0, 600.0), |nid, known, avail| match tmap.get(&nid)
        {
            Some(tm) => measure_text_node(&mut ts, tm, known, avail),
            None => TSize::ZERO,
        })
        .expect("layout");

    // El nodo de texto es el segundo en orden DFS (root, luego la hoja).
    let leaf_id = mounted.nodes[1].id;
    let rect = computed.get(leaf_id).expect("rect de la hoja");
    // A 16px y ~1.2 de interlínea, una línea ≈ 19px. Con ~150px de texto en
    // 200px de ancho deberían ser >= 4 líneas → bastante más de una.
    assert!(
        rect.h > 40.0,
        "el párrafo se aplastó: alto={} (esperaba varias líneas)",
        rect.h
    );
    assert!(rect.w <= 200.0 + 1.0, "no debería exceder el ancho del bloque");
}

#[test]
fn line_height_mayor_reserva_mas_alto() {
    let texto = "una línea de texto que envuelve en dos o tres renglones según \
                 el ancho disponible para el bloque contenedor angosto";
    let medir = |lh: f32| -> f32 {
        let mut ts = llimphi_text::Typesetter::new();
        let tm = llimphi_compositor::TextMeasure {
            content: texto.to_string(),
            size_px: 16.0,
            alignment: llimphi_text::Alignment::Start,
            italic: false,
            font_family: None,
            line_height: lh,
            weight: 400.0,
            max_lines: None,
            ellipsis: false,
            underline: false,
            strikethrough: false,
            spans: None,
            letter_spacing: 0.0,
            word_spacing: 0.0,
        };
        let known = TSize { width: Some(180.0_f32), height: None };
        let avail = TSize {
            width: AvailableSpace::Definite(180.0),
            height: AvailableSpace::MaxContent,
        };
        measure_text_node(&mut ts, &tm, known, avail).height
    };
    let compacto = medir(1.0);
    let comodo = medir(2.0);
    assert!(
        comodo > compacto * 1.5,
        "line-height: 2 debería reservar bastante más alto que 1.0 (got {compacto} vs {comodo})"
    );
}
