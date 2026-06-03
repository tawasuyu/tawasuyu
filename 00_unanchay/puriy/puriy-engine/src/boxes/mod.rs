//! Box tree — output del engine, entrada de `llimphi-raster`.
//!
//! Un [`BoxNode`] es la unidad de pintado: rectángulo con fondo opcional
//! + texto opcional + lista ordenada de hijos. No hay layout real (no
//! corremos taffy todavía) — sólo posicionamiento naive: cada bloque
//! apila vertical, cada inline se concatena en la línea. Es suficiente
//! para que Llimphi pueda dibujar example.com legible.
//!
//! Fase 3 reemplazará este pase por `llimphi-layout` con taffy.

use markup5ever_rcdom::{Handle, NodeData};

use crate::dom::{self, DomTree};
use crate::style::{
    AlignContent, AlignItems, AlignSelf, BackgroundClip, BackgroundOrigin, BackgroundPosition,
    BackgroundRepeat, BackgroundSize, BorderLineStyle,
    BoxShadow, BoxSizing, ComputedStyle, Corners, Cursor, Direction,
    FlexDirection, FlexWrap,
    GridTrackSize, Hyphens, ImageRendering, JustifyContent, LengthVal, LinearGradient,
    ListStyleType, ObjectFit, Outline, Overflow, OverflowWrap, PointerEvents, Position, Resize,
    ScrollBehavior, Sides, StyleEngine, TabSize, TextAlign, TextDecorationLine,
    TextDecorationStyle, TextOverflow, TextShadow,
    TextTransform, Transform, UnicodeBidi, UserSelect, VerticalAlign, Visibility, WhiteSpace,
    WordBreak, WritingMode,
};

/// Modelo de datos (`Color`/`Display`/`BoxNode`/`BoxTree` + tipos auxiliares).
mod model;
pub use model::*;
/// Mutación/restyle del árbol ya construido (APIs del chrome + set_box_visual).
mod mutate;
pub use mutate::*;
/// Construcción del árbol desde DOM+StyleEngine (build/build_node, svg, imágenes).
mod build;
pub use build::*;

#[cfg(test)]
mod tests {
    use super::Display;
    use crate::Engine;

    #[test]
    fn box_tree_no_vacio() {
        let html = "<html><body><h1>Hola</h1><p>Mundo</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        assert!(doc.box_tree.descendants_count() >= 3);
    }

    #[test]
    fn node_ids_son_unicos_y_no_cero() {
        let html = "<html><body><div><h1>Hola</h1><p>Mundo</p></div></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut ids = Vec::new();
        doc.box_tree.walk(|b| ids.push(b.node_id));
        assert!(ids.iter().all(|&id| id != 0), "ningún nodo queda en 0");
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "los node_id son únicos");
        // DFS pre-orden arranca en 1 sobre la raíz (body).
        assert_eq!(doc.box_tree.root.node_id, 1);
    }

    #[test]
    fn display_none_excluye_head() {
        let html = "<html><head><title>t</title></head><body><p>x</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // El árbol parte de body — head no debe haber aportado nada.
        let mut tags = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.tag {
                tags.push(t.clone());
            }
        });
        assert!(!tags.contains(&"title".to_string()));
        assert!(!tags.contains(&"head".to_string()));
    }

    #[test]
    fn ol_li_recibe_marker_decimal() {
        let html =
            "<html><body><ol><li>uno</li><li>dos</li><li>tres</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["1. ".to_string(), "2. ".into(), "3. ".into()]);
    }

    #[test]
    fn ul_li_recibe_marker_bullet() {
        let html = "<html><body><ul><li>a</li><li>b</li></ul></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.starts_with('•') {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers.len(), 2);
    }

    #[test]
    fn unidades_viewport_resuelven_contra_el_viewport_real() {
        use crate::style::{LengthVal, Viewport};
        // `vw/vh/vmin/vmax` deben resolver contra el ancho/alto REAL de la
        // ventana, no contra DEFAULT_VIEWPORT (1280×800). Con viewport 800×600
        // y `style="…"` inline (que parsea `boxes::build`, no la hoja):
        //   50vw   = 50% de 800            = 400
        //   50vh   = 50% de 600            = 300
        //   50vmin = 50% de min(800,600)   = 300
        //   50vmax = 50% de max(800,600)   = 400
        let html = r#"<html><body>
            <div id="vw" style="width:50vw"></div>
            <div id="vh" style="width:50vh"></div>
            <div id="vmin" style="width:50vmin"></div>
            <div id="vmax" style="width:50vmax"></div>
        </body></html>"#;
        let vp = Viewport { width: 800.0, height: 600.0, dpr: 1.0 };
        let doc = Engine::new().with_viewport(vp).load_html("about:test", html);
        let mut widths = std::collections::HashMap::new();
        doc.box_tree.walk(|b| {
            if let Some(id) = b.element_id.as_deref() {
                widths.insert(id.to_string(), b.width);
            }
        });
        assert_eq!(widths.get("vw"), Some(&LengthVal::Px(400.0)));
        assert_eq!(widths.get("vh"), Some(&LengthVal::Px(300.0)));
        assert_eq!(widths.get("vmin"), Some(&LengthVal::Px(300.0)));
        assert_eq!(widths.get("vmax"), Some(&LengthVal::Px(400.0)));
    }

    #[test]
    fn unidades_viewport_default_sin_viewport_real() {
        use crate::style::LengthVal;
        // Sin `with_viewport`, el Engine usa DEFAULT_VIEWPORT (1280×800):
        // 50vw = 640. Garantiza que el scope no contamina el path por defecto
        // (se restaura al dropear al volver de `load_html`).
        let html = r#"<html><body><div id="x" style="width:50vw"></div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let mut w = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                w = Some(b.width);
            }
        });
        assert_eq!(w, Some(LengthVal::Px(640.0)));
    }

    fn box_by_id(bt: &super::BoxTree, id: &str) -> Option<super::BoxNode> {
        let mut found = None;
        bt.walk(|b| {
            if found.is_none() && b.element_id.as_deref() == Some(id) {
                found = Some(b.clone());
            }
        });
        found
    }

    #[test]
    fn restyle_aplica_regla_de_clase_agregada() {
        // `.on` (no presente al cargar) + un selector descendiente `.on .child`.
        // Tras agregar la clase y recascadear, el fondo del box y el color del
        // hijo deben aparecer.
        let html = r#"<html><head><style>
            .on { background: red; }
            .on .child { color: blue; }
        </style></head><body>
            <div id="box"><p id="p" class="child">x</p></div>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "box").unwrap().background, None);
        assert!(doc.box_tree.set_element_class_list("box", vec!["on".to_string()]));
        doc.box_tree.restyle();
        assert_eq!(
            box_by_id(&doc.box_tree, "box").unwrap().background,
            Some(super::Color::rgb(255, 0, 0))
        );
        assert_eq!(box_by_id(&doc.box_tree, "p").unwrap().color, super::Color::rgb(0, 0, 255));
    }

    #[test]
    fn restyle_quitar_clase_revierte_estilo() {
        let html = r#"<html><head><style>
            #box { background: green; }
            #box.on { background: red; }
        </style></head><body><div id="box" class="on">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(
            box_by_id(&doc.box_tree, "box").unwrap().background,
            Some(super::Color::rgb(255, 0, 0))
        );
        doc.box_tree.set_element_class_list("box", vec![]);
        doc.box_tree.restyle();
        // Sin `.on`, gana la regla base `#box { background: green }`.
        assert_eq!(
            box_by_id(&doc.box_tree, "box").unwrap().background,
            Some(super::Color::rgb(0, 128, 0))
        );
    }

    #[test]
    fn restyle_combinador_hermano_afecta_posterior() {
        // Cambiar la clase de #t debe afectar a su HERMANO #pnl vía `+`.
        // Sólo posible recascadeando el árbol entero, no sólo el subárbol.
        let html = r#"<html><head><style>
            .open + .panel { background: red; }
        </style></head><body>
            <div id="t" class="tab"></div>
            <div id="pnl" class="panel">x</div>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "pnl").unwrap().background, None);
        doc.box_tree
            .set_element_class_list("t", vec!["tab".into(), "open".into()]);
        doc.box_tree.restyle();
        assert_eq!(
            box_by_id(&doc.box_tree, "pnl").unwrap().background,
            Some(super::Color::rgb(255, 0, 0))
        );
    }

    #[test]
    fn restyle_toggle_display_none_oculta_y_muestra() {
        let html = r#"<html><head><style>
            .hidden { display: none; }
        </style></head><body><div id="box">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_ne!(box_by_id(&doc.box_tree, "box").unwrap().display, super::Display::None);
        doc.box_tree.set_element_class_list("box", vec!["hidden".into()]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "box").unwrap().display, super::Display::None);
        doc.box_tree.set_element_class_list("box", vec![]);
        doc.box_tree.restyle();
        assert_ne!(box_by_id(&doc.box_tree, "box").unwrap().display, super::Display::None);
    }

    #[test]
    fn restyle_sin_cambios_es_idempotente() {
        let html = r#"<html><head><style>
            #box { background: red; color: green; padding: 5px; font-size: 20px; }
        </style></head><body><div id="box"><span id="s">hi</span></div></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        let before_box = box_by_id(&doc.box_tree, "box").unwrap();
        let before_s = box_by_id(&doc.box_tree, "s").unwrap();
        doc.box_tree.restyle();
        let after_box = box_by_id(&doc.box_tree, "box").unwrap();
        let after_s = box_by_id(&doc.box_tree, "s").unwrap();
        assert_eq!(before_box.background, after_box.background);
        assert_eq!(before_box.color, after_box.color);
        assert_eq!(before_box.display, after_box.display);
        assert_eq!(before_box.padding.top, after_box.padding.top);
        assert_eq!(before_box.font_size, after_box.font_size);
        // El span hereda color/font del padre, igual antes y después.
        assert_eq!(before_s.color, after_s.color);
        assert_eq!(before_s.font_size, after_s.font_size);
    }

    #[test]
    fn restyle_preserva_estilo_inline_seteado_por_js() {
        // `el.style.color='red'` (via set_element_style) debe sobrevivir a un
        // restyle posterior por classList: la cascada re-parsea el atributo
        // `style` y el inline gana sobre la regla `.on { color: blue }`.
        let html = r#"<html><head><style>.on { color: blue; }</style></head>
            <body><p id="p">x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        doc.box_tree.set_element_style("p", "color", "red");
        assert_eq!(box_by_id(&doc.box_tree, "p").unwrap().color, super::Color::rgb(255, 0, 0));
        doc.box_tree.set_element_class_list("p", vec!["on".into()]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "p").unwrap().color, super::Color::rgb(255, 0, 0));
    }

    #[test]
    fn build_retiene_display_none_de_autor_y_descarta_ua() {
        // Fase 7.185 — un elemento ocultado por CSS de autor se RETIENE en el
        // box tree (oculto, con su subárbol) para poder mostrarlo luego; el
        // ruido UA (`<script>`) se sigue descartando.
        let html = r#"<html><head><style>
            .modal { display: none; }
        </style></head><body>
            <div id="m" class="modal"><p id="inner">contenido</p></div>
            <script>var x = 1;</script>
            <span id="s">visible</span>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let m = box_by_id(&doc.box_tree, "m").expect("modal de autor retenido");
        assert_eq!(m.display, super::Display::None);
        assert!(box_by_id(&doc.box_tree, "inner").is_some(), "subárbol retenido");
        let mut script_text = false;
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.contains("var x") {
                    script_text = true;
                }
            }
        });
        assert!(!script_text, "el texto del <script> no debe filtrarse al box tree");
        assert!(box_by_id(&doc.box_tree, "s").is_some());
    }

    #[test]
    fn restyle_muestra_modal_oculto_al_cargar() {
        // El patrón clásico: modal arranca `display:none`, JS agrega `.open`
        // para mostrarlo. Posible porque retenemos el box oculto al cargar.
        let html = r#"<html><head><style>
            .modal { display: none; }
            .modal.open { display: block; }
        </style></head><body>
            <div id="m" class="modal">hola</div>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "m").unwrap().display, super::Display::None);
        doc.box_tree
            .set_element_class_list("m", vec!["modal".into(), "open".into()]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "m").unwrap().display, super::Display::Block);
    }

    #[test]
    fn pseudo_estado_checked_disabled_enabled() {
        let html = r#"<html><head><style>
            input:checked { background: red; }
            input:disabled { color: green; }
            input:enabled { color: blue; }
            input:required { background: yellow; }
        </style></head><body>
            <input id="a" type="checkbox" checked>
            <input id="b" type="checkbox">
            <input id="c" type="text" disabled>
            <input id="d" type="text" required>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let red = super::Color::rgb(255, 0, 0);
        let blue = super::Color::rgb(0, 0, 255);
        // a: checked → fondo rojo; enabled → color azul.
        assert_eq!(box_by_id(&doc.box_tree, "a").unwrap().background, Some(red));
        assert_eq!(box_by_id(&doc.box_tree, "a").unwrap().color, blue);
        // b: no checked → no rojo (conserva su fondo UA); enabled → azul.
        assert_ne!(box_by_id(&doc.box_tree, "b").unwrap().background, Some(red));
        assert_eq!(box_by_id(&doc.box_tree, "b").unwrap().color, blue);
        // c: disabled → verde; NO enabled (no azul).
        assert_eq!(box_by_id(&doc.box_tree, "c").unwrap().color, super::Color::rgb(0, 128, 0));
        // d: required → fondo amarillo.
        assert_eq!(box_by_id(&doc.box_tree, "d").unwrap().background, Some(super::Color::rgb(255, 255, 0)));
    }

    #[test]
    fn pseudo_nth_of_type_y_only_of_type_y_nth_last() {
        let html = r#"<html><head><style>
            p:nth-of-type(2) { color: red; }
            li:nth-last-child(1) { color: green; }
            span:only-of-type { color: blue; }
        </style></head><body>
            <div><span id="sp">x</span><p id="p1">1</p><p id="p2">2</p></div>
            <ul><li id="l1">a</li><li id="l2">b</li></ul>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "p2").unwrap().color, super::Color::rgb(255, 0, 0));
        assert_ne!(box_by_id(&doc.box_tree, "p1").unwrap().color, super::Color::rgb(255, 0, 0));
        assert_eq!(box_by_id(&doc.box_tree, "l2").unwrap().color, super::Color::rgb(0, 128, 0));
        assert_ne!(box_by_id(&doc.box_tree, "l1").unwrap().color, super::Color::rgb(0, 128, 0));
        assert_eq!(box_by_id(&doc.box_tree, "sp").unwrap().color, super::Color::rgb(0, 0, 255));
    }

    #[test]
    fn sync_checked_y_restyle_actualiza_pseudo_checked() {
        // Fase 7.187 — togglear un checkbox actualiza el atributo `checked` y
        // recascadea: `:checked` y `:checked + label` aplican en vivo.
        let html = r#"<html><head><style>
            input:checked { background: red; }
            input:checked + label { color: blue; }
        </style></head><body>
            <input id="cb" type="checkbox"><label id="lb">L</label>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        let red = super::Color::rgb(255, 0, 0);
        let blue = super::Color::rgb(0, 0, 255);
        assert_ne!(box_by_id(&doc.box_tree, "cb").unwrap().background, Some(red));
        // Marcar (el checkbox es el control índice 0).
        doc.box_tree.sync_checked_from(&[true]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "cb").unwrap().background, Some(red));
        assert_eq!(box_by_id(&doc.box_tree, "lb").unwrap().color, blue);
        // Desmarcar revierte ambos.
        doc.box_tree.sync_checked_from(&[false]);
        doc.box_tree.restyle();
        assert_ne!(box_by_id(&doc.box_tree, "cb").unwrap().background, Some(red));
        assert_ne!(box_by_id(&doc.box_tree, "lb").unwrap().color, blue);
    }

    #[test]
    fn pseudo_is_y_where_matchean_lista() {
        let html = r#"<html><head><style>
            :is(h1, h2) { color: red; }
            .box :where(.a, .b) { background: green; }
            #x:is(.on, .off) { color: blue; }
        </style></head><body>
            <h2 id="h">t</h2>
            <div class="box"><span id="s" class="b">x</span></div>
            <p id="x" class="on">p</p>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "h").unwrap().color, super::Color::rgb(255, 0, 0));
        assert_eq!(
            box_by_id(&doc.box_tree, "s").unwrap().background,
            Some(super::Color::rgb(0, 128, 0))
        );
        assert_eq!(box_by_id(&doc.box_tree, "x").unwrap().color, super::Color::rgb(0, 0, 255));
    }

    #[test]
    fn pseudo_where_no_aporta_especificidad() {
        // `:where(#hero)` tiene especificidad 0 → lo vence el selector de tag
        // `p` (que llega después y tiene especificidad 1). Si `:where` aportara
        // los 100 del `#id`, ganaría el rojo.
        let html = r#"<html><head><style>
            :where(#hero) { color: red; }
            p { color: green; }
        </style></head><body><p id="hero">x</p></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "hero").unwrap().color, super::Color::rgb(0, 128, 0));
    }

    #[test]
    fn shorthand_inset_y_flex_flow() {
        use crate::style::LengthVal;
        let html = r#"<html><head><style>
            #a { position: absolute; inset: 10px 20px; }
            #b { display: flex; flex-flow: column wrap; }
        </style></head><body>
            <div id="a">x</div><div id="b">y</div>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // `inset: 10px 20px` → top/bottom=10, right/left=20.
        assert_eq!(a.inset_top, LengthVal::Px(10.0));
        assert_eq!(a.inset_right, LengthVal::Px(20.0));
        assert_eq!(a.inset_bottom, LengthVal::Px(10.0));
        assert_eq!(a.inset_left, LengthVal::Px(20.0));
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        assert_eq!(b.flex_direction, super::FlexDirection::Column);
        assert_eq!(b.flex_wrap, super::FlexWrap::Wrap);
    }

    #[test]
    fn pseudo_not_con_lista() {
        // CSS4: `:not(.a, .b)` no matchea si el elemento tiene .a O .b.
        let html = r#"<html><head><style>
            li:not(.skip, .hidden) { color: red; }
        </style></head><body><ul>
            <li id="n1">uno</li>
            <li id="n2" class="skip">dos</li>
            <li id="n3" class="hidden">tres</li>
        </ul></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let red = super::Color::rgb(255, 0, 0);
        assert_eq!(box_by_id(&doc.box_tree, "n1").unwrap().color, red); // sin clases → rojo
        assert_ne!(box_by_id(&doc.box_tree, "n2").unwrap().color, red); // .skip → excluido
        assert_ne!(box_by_id(&doc.box_tree, "n3").unwrap().color, red); // .hidden → excluido
    }

    #[test]
    fn propiedades_logicas_de_caja() {
        let html = r#"<html><head><style>
            #a { margin-inline: 10px 20px; padding-block: 5px; }
            #b { margin-inline-start: 8px; padding-block-end: 12px; }
        </style></head><body><div id="a">x</div><div id="b">y</div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // margin-inline: 10 20 → left=10 (start), right=20 (end), LTR.
        assert_eq!(a.margin.left, 10.0);
        assert_eq!(a.margin.right, 20.0);
        // padding-block: 5 → top=bottom=5.
        assert_eq!(a.padding.top, 5.0);
        assert_eq!(a.padding.bottom, 5.0);
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        assert_eq!(b.margin.left, 8.0); // inline-start = left (LTR)
        assert_eq!(b.padding.bottom, 12.0); // block-end = bottom
    }

    #[test]
    fn inset_logico_inline_y_block() {
        use crate::style::LengthVal;
        let html = r#"<html><head><style>
            #a { position: absolute; inset-inline: 10px 20px; inset-block: 5px; }
            #b { position: absolute; inset-inline-start: 8px; inset-block-end: 12px; }
        </style></head><body><div id="a">x</div><div id="b">y</div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // inset-inline: 10 20 → left=10 (start), right=20 (end), LTR.
        assert_eq!(a.inset_left, LengthVal::Px(10.0));
        assert_eq!(a.inset_right, LengthVal::Px(20.0));
        // inset-block: 5 → top=bottom=5.
        assert_eq!(a.inset_top, LengthVal::Px(5.0));
        assert_eq!(a.inset_bottom, LengthVal::Px(5.0));
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        assert_eq!(b.inset_left, LengthVal::Px(8.0)); // inline-start = left (LTR)
        assert_eq!(b.inset_bottom, LengthVal::Px(12.0)); // block-end = bottom
    }

    #[test]
    fn height_explicito_se_propaga_al_box() {
        use crate::style::LengthVal;
        let html = r#"<html><head><style>
            #a { height: 200px; }
            #b { height: 50%; }
            #c { width: 100px; }
        </style></head><body>
            <div id="a">x</div><div id="b">y</div><div id="c">z</div>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "a").unwrap().height, LengthVal::Px(200.0));
        assert_eq!(box_by_id(&doc.box_tree, "b").unwrap().height, LengthVal::Pct(50.0));
        // Sin `height` declarado → Auto (lo dimensiona el contenido).
        assert_eq!(box_by_id(&doc.box_tree, "c").unwrap().height, LengthVal::Auto);
    }

    #[test]
    fn tamanos_logicos_inline_block() {
        use crate::style::LengthVal;
        let html = r#"<html><head><style>
            #a { inline-size: 120px; block-size: 80px; }
            #b { min-inline-size: 10px; max-block-size: 200px; }
        </style></head><body><div id="a">x</div><div id="b">y</div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // inline-size → width, block-size → height (LTR/horizontal).
        assert_eq!(a.width, LengthVal::Px(120.0));
        assert_eq!(a.height, LengthVal::Px(80.0));
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        assert_eq!(b.min_width, LengthVal::Px(10.0));
        assert_eq!(b.max_height, LengthVal::Px(200.0));
    }

    #[test]
    fn border_logico_inline_y_block() {
        let html = r#"<html><head><style>
            #a { border-inline: 3px solid red; }
            #b { border-block-start: 5px solid blue; border-inline-end-width: 7px; }
        </style></head><body><div id="a">x</div><div id="b">y</div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let red = super::Color::rgb(255, 0, 0);
        let blue = super::Color::rgb(0, 0, 255);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // border-inline: 3px solid red → left y right (LTR), no top/bottom.
        assert_eq!(a.border_widths.left, 3.0);
        assert_eq!(a.border_widths.right, 3.0);
        assert_eq!(a.border_widths.top, 0.0);
        assert_eq!(a.border_colors.left, Some(red));
        assert_eq!(a.border_colors.right, Some(red));
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        // border-block-start = top.
        assert_eq!(b.border_widths.top, 5.0);
        assert_eq!(b.border_colors.top, Some(blue));
        // border-inline-end-width = right-width.
        assert_eq!(b.border_widths.right, 7.0);
    }

    #[test]
    fn list_style_none_suprime_marker() {
        let html = r#"<html><head><style>
            ul { list-style-type: none }
        </style></head><body><ul><li>uno</li><li>dos</li></ul></body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut has_bullet = false;
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.contains('•') {
                    has_bullet = true;
                }
            }
        });
        assert!(!has_bullet, "no debería haber marker con list-style-type:none");
    }

    #[test]
    fn ol_start_corre_el_contador() {
        let html =
            "<html><body><ol start=\"5\"><li>x</li><li>y</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["5. ".to_string(), "6. ".into()]);
    }

    #[test]
    fn li_value_resetea_el_contador() {
        let html = "<html><body><ol><li>x</li><li value=\"10\">y</li><li>z</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["1. ".to_string(), "10. ".into(), "11. ".into()]);
    }

    #[test]
    fn lower_roman_y_lower_alpha_aplican() {
        let html = r#"<html><head><style>
            .roman { list-style-type: lower-roman }
            .alpha { list-style-type: upper-alpha }
        </style></head><body>
          <ol class="roman"><li>a</li><li>b</li><li>c</li></ol>
          <ol class="alpha"><li>a</li><li>b</li></ol>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        // ol.roman → i. ii. iii.   ol.alpha → A. B.
        assert_eq!(
            markers,
            vec![
                "i. ".to_string(),
                "ii. ".into(),
                "iii. ".into(),
                "A. ".into(),
                "B. ".into(),
            ]
        );
    }

    #[test]
    fn to_alpha_y_to_roman_son_correctos() {
        use super::{to_alpha, to_roman};
        assert_eq!(to_alpha(1, false), "a");
        assert_eq!(to_alpha(26, false), "z");
        assert_eq!(to_alpha(27, false), "aa");
        assert_eq!(to_alpha(28, false), "ab");
        assert_eq!(to_alpha(52, true), "AZ");
        assert_eq!(to_roman(4, false), "iv");
        assert_eq!(to_roman(9, true), "IX");
        assert_eq!(to_roman(1994, false), "mcmxciv");
        assert_eq!(to_roman(3999, true), "MMMCMXCIX");
        // Fuera de rango → decimal fallback.
        assert_eq!(to_roman(4000, false), "4000");
        assert_eq!(to_roman(0, true), "0");
    }

    #[test]
    fn estilo_inline_aplica_color() {
        let html = r#"<html><body><p style="color: #ff0000">x</p></body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found_red = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::Color::rgb(255, 0, 0) {
                found_red = true;
            }
        });
        assert!(found_red, "no se encontró <p> con color rojo");
    }

    #[test]
    fn link_stylesheet_externo_data_url_aplica() {
        // `<link rel="stylesheet" href="data:text/css,...">` — la hoja externa
        // se baja (acá vía data:, sin red) y sus reglas entran a la cascada.
        let html = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3A%23008000%7D">
        </head><body><p>verde</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::Color::rgb(0, 128, 0) {
                found = true;
            }
        });
        assert!(found, "la regla de la hoja externa data: no se aplicó al <p>");
    }

    #[test]
    fn link_relativo_resuelve_contra_base_href() {
        // `<base href="file://<dir>/">` + `<link href="x.css">` relativo debe
        // bajar `<dir>/x.css` (no contra la URL del documento). file:// = sin red.
        let mut dir = std::env::temp_dir();
        dir.push(format!("puriy_basehref_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.css"), "p { color: #00ff00 }").unwrap();
        let base = format!("file://{}/", dir.display());
        let html = format!(
            r##"<html><head><base href="{base}"><link rel="stylesheet" href="x.css"></head><body><p>v</p></body></html>"##
        );
        let eng = Engine::new();
        let doc = eng.load_html("about:test", &html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::Color::rgb(0, 255, 0) {
                found = true;
            }
        });
        let _ = std::fs::remove_dir_all(&dir);
        assert!(found, "el <link> relativo no resolvió contra <base href>");
    }

    #[test]
    fn import_en_style_inline_se_sigue() {
        // `@import` de un data: CSS dentro de un <style> — sus reglas aplican.
        let html = r##"<html><head><style>
            @import url("data:text/css,p%7Bcolor%3A%23ff0000%7D");
        </style></head><body><p>x</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::Color::rgb(255, 0, 0) {
                found = true;
            }
        });
        assert!(found, "la regla del @import no se aplicó");
    }

    #[test]
    fn import_precede_a_las_reglas_propias_en_cascada() {
        // @import pone rojo; la regla propia (después) lo pisa a azul → azul.
        let html = r##"<html><head><style>
            @import url("data:text/css,p%7Bcolor%3Ared%7D");
            p { color: #0000ff }
        </style></head><body><p>x</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut p_color = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") {
                p_color = Some(b.color);
            }
        });
        assert_eq!(p_color, Some(super::Color::rgb(0, 0, 255)), "la regla propia debe ganar al @import");
    }

    #[test]
    fn link_media_print_no_aplica_en_pantalla() {
        // `<link media="print">` no debe aplicar al render de pantalla; la
        // misma regla con `media="screen"` sí. DEFAULT_VIEWPORT es screen.
        let print = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3Ared%7D" media="print">
        </head><body><p>x</p></body></html>"##;
        let screen = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3Ared%7D" media="screen">
        </head><body><p>x</p></body></html>"##;
        let eng = Engine::new();
        let red = super::Color::rgb(255, 0, 0);
        let color_of = |html: &str| {
            let doc = eng.load_html("about:test", html);
            let mut c = None;
            doc.box_tree.walk(|b| {
                if b.tag.as_deref() == Some("p") {
                    c = Some(b.color);
                }
            });
            c
        };
        assert_ne!(color_of(print), Some(red), "media=print no debía aplicar en pantalla");
        assert_eq!(color_of(screen), Some(red), "media=screen sí debía aplicar");
    }

    #[test]
    fn link_stylesheet_cascada_respeta_orden_de_documento() {
        // Hoja externa (data:) declara color rojo; un `<style>` posterior lo
        // pisa a azul — el orden de documento debe ganar (azul), no el externo.
        let html = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3Ared%7D">
            <style>p { color: #0000ff }</style>
        </head><body><p>azul</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut p_color = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") {
                p_color = Some(b.color);
            }
        });
        assert_eq!(p_color, Some(super::Color::rgb(0, 0, 255)), "el <style> posterior debe ganar");
    }

    #[test]
    fn details_sin_open_attr_arranca_cerrado() {
        let html = r#"<html><body>
            <details><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut details_attr: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("details") {
                details_attr.push(b.details_open_attr);
            }
        });
        assert_eq!(details_attr, vec![false]);
    }

    #[test]
    fn details_con_open_attr_lo_refleja() {
        let html = r#"<html><body>
            <details open><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut details_attr: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("details") {
                details_attr.push(b.details_open_attr);
            }
        });
        assert_eq!(details_attr, vec![true]);
    }

    #[test]
    fn details_summary_se_parsean_como_tags() {
        let html = r#"<html><body>
            <details><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut saw_details = false;
        let mut saw_summary = false;
        doc.box_tree.walk(|b| {
            match b.tag.as_deref() {
                Some("details") => saw_details = true,
                Some("summary") => saw_summary = true,
                _ => {}
            }
        });
        assert!(saw_details, "no se encontró <details> en el box tree");
        assert!(saw_summary, "no se encontró <summary> en el box tree");
    }

    #[test]
    fn details_open_attr_es_false_para_nodos_no_details() {
        let html = "<html><body><p>x</p><h1>y</h1></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() != Some("details") {
                assert!(!b.details_open_attr, "{:?} no debería tener details_open_attr=true", b.tag);
            }
        });
    }

    #[test]
    fn ws_entre_blocks_se_filtra() {
        // El "\n  " entre </h1> y <p> produce un Text node " " que NO
        // debería rendear como un row vacío.
        let html = "<html><body><h1>A</h1>\n  <p>B</p>\n  <h2>C</h2></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Walk del body. Esperamos sólo h1, p, h2 como children directos
        // (sin text-leaves de whitespace entre ellos).
        let body = &doc.box_tree.root;
        // Body envuelve un Inline de transición (collapse_whitespace puede
        // dejar uno leading o trailing). Recorremos directamente.
        let mut top_tags: Vec<Option<String>> = body
            .children
            .iter()
            .filter(|c| !super::is_ws_only_inline(c))
            .map(|c| c.tag.clone())
            .collect();
        // Aseguramos que el filtrado sólo dejó tags reales.
        top_tags.retain(|t| t.is_some());
        let names: Vec<&str> = top_tags
            .iter()
            .map(|t| t.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(names, vec!["h1", "p", "h2"]);
        // Y verificamos que NO hay inlines whitespace-only entre ellos en
        // el árbol real (post-strip).
        for c in &body.children {
            assert!(
                !super::is_ws_only_inline(c),
                "el body no debería tener inlines ws-only entre blocks: {:?}",
                c.text
            );
        }
    }

    #[test]
    fn ws_alrededor_de_inline_se_preserva() {
        // El espacio entre "foo " y <strong>bar</strong> y " baz" sí
        // tiene valor — debe quedarse para no pegar tokens.
        let html = "<html><body><p>foo <strong>bar</strong> baz</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Encontramos el <p> y verificamos que sus children contengan
        // textos con espacios donde corresponde.
        let mut texts: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") {
                for c in &b.children {
                    if let Some(t) = &c.text {
                        texts.push(t.clone());
                    }
                    // Si es <strong>, mirá su hijo
                    if c.tag.as_deref() == Some("strong") {
                        for cc in &c.children {
                            if let Some(t) = &cc.text {
                                texts.push(format!("[strong]{t}"));
                            }
                        }
                    }
                }
            }
        });
        // Esperamos que "foo " conserve el espacio trailing y " baz" el leading.
        assert!(
            texts.iter().any(|t| t.ends_with(' ')),
            "esperaba un text con espacio trailing en {:?}",
            texts
        );
        assert!(
            texts.iter().any(|t| t.starts_with(' ')),
            "esperaba un text con espacio leading en {:?}",
            texts
        );
        assert!(
            texts.iter().any(|t| t == "[strong]bar"),
            "esperaba `bar` dentro de strong en {:?}",
            texts
        );
    }

    #[test]
    fn link_target_blank_marca_link_new_tab() {
        let html = r#"<html><body>
            <a href="https://a.test/" target="_blank">A</a>
            <a href="https://b.test/">B</a>
            <a href="https://c.test/" target="_self">C</a>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut links: Vec<(String, bool)> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                if let Some(target) = &b.link {
                    links.push((target.clone(), b.link_new_tab));
                }
            }
        });
        assert!(links.iter().any(|(u, nt)| u.contains("a.test") && *nt));
        assert!(links.iter().any(|(u, nt)| u.contains("b.test") && !*nt));
        assert!(links.iter().any(|(u, nt)| u.contains("c.test") && !*nt));
    }

    #[test]
    fn link_mailto_y_tel_y_javascript_se_ignoran() {
        let html = r#"<html><body>
            <a href="mailto:foo@bar">M</a>
            <a href="tel:+541112345678">T</a>
            <a href="javascript:alert(1)">J</a>
            <a href="data:text/plain,hi">D</a>
            <a href="ftp://example.com/">F</a>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut clickable: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                if let Some(t) = &b.link {
                    clickable.push(t.clone());
                }
            }
        });
        assert!(clickable.is_empty(), "ningún href no-web debería ser clickable: {clickable:?}");
    }

    #[test]
    fn srcset_elige_la_densidad_mas_alta() {
        let url = super::pick_srcset("foo.png 1x, foo-2x.png 2x, foo-3x.png 3x");
        assert_eq!(url.as_deref(), Some("foo-3x.png"));
    }

    #[test]
    fn srcset_elige_el_ancho_mas_grande() {
        let url = super::pick_srcset("a.png 320w, b.png 800w, c.png 1600w");
        assert_eq!(url.as_deref(), Some("c.png"));
    }

    #[test]
    fn srcset_sin_descriptor_usa_la_primera_con_1x_implicito() {
        // En la práctica un srcset sin descriptor es equivalente a 1x.
        let url = super::pick_srcset("a.png, b.png");
        // No importa el orden interno — basta con que devuelva alguno.
        assert!(url.is_some());
    }

    #[test]
    fn svg_parsea_polygon_y_polyline() {
        let html = r##"<html><body>
            <svg width="100" height="100">
                <polygon points="0,0 50,0 50,50" fill="red"/>
                <polyline points="0,100 100,50 100,0" stroke="blue"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut prim_count = 0;
        let mut had_closed = false;
        let mut had_open = false;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                for p in &s.prims {
                    if let crate::SvgPrim::Polyline { points, closed, .. } = p {
                        prim_count += 1;
                        if *closed {
                            had_closed = true;
                            assert_eq!(points.len(), 3);
                        } else {
                            had_open = true;
                        }
                    }
                }
            }
        });
        assert_eq!(prim_count, 2);
        assert!(had_closed);
        assert!(had_open);
    }

    #[test]
    fn svg_parsea_path_minimal() {
        let html = r##"<html><body>
            <svg width="100" height="100">
                <path d="M 10 10 L 90 10 L 50 90 Z" fill="green"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut cmds_count = 0;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                for p in &s.prims {
                    if let crate::SvgPrim::Path { d, .. } = p {
                        cmds_count = d.len();
                    }
                }
            }
        });
        // M, L, L, Z → 4 cmds.
        assert_eq!(cmds_count, 4);
    }

    #[test]
    fn svg_recolecta_rect_circle_y_line() {
        let html = r##"<html><body>
            <svg width="200" height="100" viewBox="0 0 200 100">
                <rect x="10" y="10" width="50" height="30" fill="red" stroke="black" stroke-width="2"/>
                <circle cx="120" cy="50" r="20" fill="blue"/>
                <line x1="0" y1="0" x2="200" y2="100" stroke="green" stroke-width="3"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut scene: Option<crate::SvgScene> = None;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                scene = Some(s.clone());
            }
        });
        let scene = scene.expect("debería haber un <svg>");
        assert_eq!(scene.width, 200.0);
        assert_eq!(scene.height, 100.0);
        assert_eq!(scene.view_box, Some((0.0, 0.0, 200.0, 100.0)));
        assert_eq!(scene.prims.len(), 3);
        match &scene.prims[0] {
            crate::SvgPrim::Rect { x, y, w, h, fill, stroke, .. } => {
                assert_eq!(*x, 10.0);
                assert_eq!(*y, 10.0);
                assert_eq!(*w, 50.0);
                assert_eq!(*h, 30.0);
                assert!(fill.is_some());
                assert!(stroke.is_some());
            }
            _ => panic!("primera prim debería ser Rect"),
        }
        match &scene.prims[1] {
            crate::SvgPrim::Circle { cx, cy, r, .. } => {
                assert_eq!(*cx, 120.0);
                assert_eq!(*cy, 50.0);
                assert_eq!(*r, 20.0);
            }
            _ => panic!("segunda prim debería ser Circle"),
        }
        match &scene.prims[2] {
            crate::SvgPrim::Line { x1, y2, .. } => {
                assert_eq!(*x1, 0.0);
                assert_eq!(*y2, 100.0);
            }
            _ => panic!("tercera prim debería ser Line"),
        }
    }

    #[test]
    fn select_recolecta_options_y_seleccionado_inicial() {
        let html = r##"<html><body>
            <form action="/p">
                <select name="lang">
                    <option value="es">Español</option>
                    <option value="en" selected>English</option>
                    <option>Otro</option>
                </select>
            </form>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/", html);
        let mut info: Option<crate::SelectInfo> = None;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.select {
                info = Some(s.clone());
                assert_eq!(b.input_name.as_deref(), Some("lang"));
                assert_eq!(b.form_idx, Some(0));
            }
        });
        let info = info.expect("debería haber un <select>");
        assert_eq!(info.options.len(), 3);
        assert_eq!(info.options[0].value, "es");
        assert_eq!(info.options[0].label, "Español");
        assert_eq!(info.options[2].label, "Otro");
        assert_eq!(info.options[2].value, "Otro"); // fallback al label
        assert_eq!(info.initial, 1); // <option selected> es el segundo
    }

    #[test]
    fn form_asigna_form_idx_a_inputs_que_contiene() {
        let html = r##"<html><body>
            <form action="/search" method="get">
                <input type="text" name="q" value="hola">
                <input type="text" name="lang" value="es">
            </form>
            <input type="text" name="outside">
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/", html);
        assert_eq!(doc.box_tree.forms.len(), 1);
        let mut names_inside: Vec<String> = Vec::new();
        let mut outside_form_idx: Option<usize> = None;
        doc.box_tree.walk(|b| {
            if let Some(name) = &b.input_name {
                if b.form_idx == Some(0) {
                    names_inside.push(name.clone());
                } else if b.input_kind.is_some() && name == "outside" {
                    outside_form_idx = b.form_idx;
                }
            }
        });
        assert_eq!(names_inside, vec!["q".to_string(), "lang".into()]);
        assert_eq!(outside_form_idx, None);
        assert_eq!(
            doc.box_tree.forms[0].action.as_deref(),
            Some("https://example.com/search")
        );
    }

    #[test]
    fn em_y_i_y_cite_son_italic_por_default() {
        let html = "<html><body><em>a</em><i>b</i><cite>c</cite><p>d</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Vec<(String, crate::FontStyle)> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(tag) = &b.tag {
                if matches!(tag.as_str(), "em" | "i" | "cite" | "p") {
                    found.push((tag.clone(), b.font_style));
                }
            }
        });
        let em = found.iter().find(|(t, _)| t == "em").unwrap();
        let i = found.iter().find(|(t, _)| t == "i").unwrap();
        let cite = found.iter().find(|(t, _)| t == "cite").unwrap();
        let p = found.iter().find(|(t, _)| t == "p").unwrap();
        assert_eq!(em.1, crate::FontStyle::Italic);
        assert_eq!(i.1, crate::FontStyle::Italic);
        assert_eq!(cite.1, crate::FontStyle::Italic);
        assert_eq!(p.1, crate::FontStyle::Normal);
    }

    #[test]
    fn font_style_normal_override_padre_italic() {
        let html = r##"<html><head><style>
            .x { font-style: normal }
        </style></head><body><em>fuera<span class="x">dentro</span></em></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut span_style: Option<crate::FontStyle> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("span") {
                span_style = Some(b.font_style);
            }
        });
        assert_eq!(span_style, Some(crate::FontStyle::Normal));
    }

    #[test]
    fn focus_pseudo_aporta_a_focus_background() {
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            input { background: white }
            input:focus { background: #ffeecc }
        </style></head><body><input type="text"></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let input = dom.find("input").unwrap();
        let base = styles.compute_with_parent_for_state(&input, None, false, false);
        let focused = styles.compute_with_parent_for_state(&input, None, false, true);
        // base es blanco (255,255,255), focused es #ffeecc (255,238,204).
        assert_eq!(base.background.map(|c| (c.r, c.g, c.b)), Some((255, 255, 255)));
        assert_eq!(focused.background.map(|c| (c.r, c.g, c.b)), Some((255, 238, 204)));
    }

    #[test]
    fn box_tree_expone_focus_background() {
        let html = r##"<html><head><style>
            input:focus { background: #abcdef }
        </style></head><body><input type="text"></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("input") {
                assert_eq!(
                    b.focus_background.map(|c| (c.r, c.g, c.b)),
                    Some((0xab, 0xcd, 0xef))
                );
                found = true;
            }
        });
        assert!(found, "no se encontró <input> en el box tree");
    }

    #[test]
    fn parsea_background_image_url_a_computed_style_y_no_descarga_si_url_no_resuelve() {
        // Sin red, fetch_and_decode falla y background_image queda None.
        // Pero el url SÍ debe quedar capturado en computed.background_image_url
        // (visible al re-parsear el stylesheet).
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            .hero { background-image: url("https://nope.invalid/bg.png") }
        </style></head><body><div class="hero">x</div></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let div = dom.find("div").expect("debería encontrar <div>");
        let s = styles.compute_with_parent(&div, None);
        assert_eq!(
            s.background_image_url.as_deref(),
            Some("https://nope.invalid/bg.png")
        );
    }

    #[test]
    fn background_image_none_limpia_url() {
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            .hero { background-image: url(a.png) }
            .hero.off { background-image: none }
        </style></head><body><div class="hero off">x</div></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let div = dom.find("div").expect("debería encontrar <div>");
        let s = styles.compute_with_parent(&div, None);
        assert!(s.background_image_url.is_none());
    }

    #[test]
    fn link_fragmento_se_resuelve_a_base_mas_frag() {
        // Antes: `#top` se ignoraba (None). Ahora resuelve contra la
        // base — el chrome detecta same-page y scrollea en lugar de
        // recargar la URL.
        let html = r##"<html><body><a href="#top">arriba</a></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/doc", html);
        let mut links: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(l) = &b.link {
                links.push(l.clone());
            }
        });
        assert_eq!(links, vec!["https://example.com/doc#top".to_string()]);
    }

    #[test]
    fn iframe_se_renderea_como_placeholder_con_url() {
        let html = r##"<html><body>
            <iframe src="https://embed.example.com/video"></iframe>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Option<String> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("iframe") {
                if let Some(first) = b.children.first() {
                    found = first.text.clone();
                }
            }
        });
        assert_eq!(
            found.as_deref(),
            Some("[iframe: https://embed.example.com/video]")
        );
    }

    #[test]
    fn iframe_sin_src_muestra_label_generico() {
        let html = "<html><body><iframe></iframe></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Option<String> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("iframe") {
                found = b.children.first().and_then(|c| c.text.clone());
            }
        });
        assert_eq!(found.as_deref(), Some("[iframe sin src]"));
    }

    #[test]
    fn content_url_parser_acepta_quoted_y_unquoted() {
        use crate::ContentItem;
        let html = r##"<html><head><style>
            .a::before { content: url("https://x/y.png") }
            .b::before { content: url(https://x/z.png) }
        </style></head><body>
            <p class="a"></p>
            <p class="b"></p>
        </body></html>"##;
        let dom = crate::DomTree::parse(html);
        let eng = crate::StyleEngine::from_dom(&dom);
        let ps_a = dom.find("p").unwrap();
        let before = eng.compute_pseudo(&ps_a, crate::PseudoElement::Before, None);
        assert_eq!(
            before.and_then(|s| s.content),
            Some(vec![ContentItem::Url("https://x/y.png".into())])
        );
    }

    #[test]
    fn margin_collapse_padre_promueve_margin_del_primer_hijo() {
        // <body style="margin: 8px"> con primer hijo
        // <div style="margin: 20px 0 0 0">: el body no tiene padding/
        // border arriba, así que el margin_top del div se promueve al
        // body. Final: body.margin.top = max(8, 20) = 20; div.margin.top = 0.
        let html = r##"<html><body style="margin: 8px">
            <div style="margin: 20px 0 0 0">x</div>
            <div style="margin: 0 0 12px 0">y</div>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // body es el root del box tree (BoxTree.root viene de
        // dom.find("body")).
        assert_eq!(doc.box_tree.root.tag.as_deref(), Some("body"));
        assert_eq!(doc.box_tree.root.margin.top, 20.0);
        assert_eq!(doc.box_tree.root.margin.bottom, 12.0);
        // El primer hijo div quedó con margin.top = 0 (promovido).
        let first_div = &doc.box_tree.root.children[0];
        assert_eq!(first_div.margin.top, 0.0);
        // El último div: margin.bottom promovido al body.
        let last_div = doc.box_tree.root.children.last().unwrap();
        assert_eq!(last_div.margin.bottom, 0.0);
    }

    #[test]
    fn margin_collapse_padre_bloqueado_por_padding() {
        // Si el body tiene padding-top, el margin del primer hijo NO
        // colapsa contra el body — el padding es la "barrera".
        let html = r##"<html><body style="margin: 8px; padding: 10px 0 0 0">
            <div style="margin: 20px 0 0 0">x</div>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        assert_eq!(doc.box_tree.root.margin.top, 8.0);
        let first_div = &doc.box_tree.root.children[0];
        assert_eq!(first_div.margin.top, 20.0);
    }

    #[test]
    fn margin_collapsing_max_entre_block_siblings() {
        // `<h2 style="margin: 0 0 20px 0">` seguido de `<p style="margin: 10px 0 0 0">`:
        // gap esperado es max(20, 10) = 20. El margin_bottom del h2
        // queda intacto (20), el margin_top del p baja a 0.
        let html = r##"<html><body>
            <h2 style="margin: 0 0 20px 0">Heading</h2>
            <p style="margin: 10px 0 0 0">Para</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut h2_margin_bottom: Option<f32> = None;
        let mut p_margin_top: Option<f32> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("h2") {
                h2_margin_bottom = Some(b.margin.bottom);
            }
            if b.tag.as_deref() == Some("p") {
                p_margin_top = Some(b.margin.top);
            }
        });
        assert_eq!(h2_margin_bottom, Some(20.0));
        // 10 - min(20, 10) = 10 - 10 = 0. Gap total = 20 + 0 = 20 = max.
        assert_eq!(p_margin_top, Some(0.0));
    }

    #[test]
    fn margin_collapsing_no_aplica_a_inline() {
        // Block + inline no colapsan — el inline vive en otro flow.
        let html = r##"<html><body>
            <p style="margin: 0 0 10px 0">Para</p>
            <span style="margin: 5px 0 0 0">inline</span>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut span_margin_top: Option<f32> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("span") {
                span_margin_top = Some(b.margin.top);
            }
        });
        // No tocado.
        assert_eq!(span_margin_top, Some(5.0));
    }

    #[test]
    fn prefetch_no_crashea_sin_imagenes() {
        // Sanity: páginas sin imágenes no deben fallar el prefetch.
        let html = "<html><body><p>solo texto</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Si llegó acá sin panic, OK.
        assert!(doc.box_tree.descendants_count() > 0);
    }

    #[test]
    fn prefetch_skip_de_urls_no_http() {
        // URLs `about:`/`file:`/`data:` no deben encolarse al pool —
        // sería un round-trip al timeout para nada. El test pone una
        // base `about:test` con `<img src="...">` que resuelve a
        // about:... y verifica que la carga termina rápido (sin
        // esperar timeouts de red).
        let html = r##"<html><body><img src="x.png"></body></html>"##;
        let eng = Engine::new();
        let t0 = std::time::Instant::now();
        let _ = eng.load_html("about:test", html);
        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_millis() < 500,
            "load_html con base about: y un <img> debería ser instantáneo, fue {elapsed:?}"
        );
    }

    #[test]
    fn img_data_url_se_decodifica_inline() {
        // `<img src="data:image/png;base64,...">` con un PNG 1×1 (un pixel rojo).
        // `resolve_href` bloquea data: (no navegable), pero como fuente de
        // imagen `fetch_image_src` lo decodifica sin tocar la red.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let html = format!(r##"<html><body><img src="{png_1x1}"></body></html>"##);
        let eng = Engine::new();
        let doc = eng.load_html("about:test", &html);
        let mut img_dims: Option<(u32, u32)> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("img") {
                if let Some(img) = &b.image {
                    img_dims = Some((img.width, img.height));
                }
            }
        });
        assert_eq!(img_dims, Some((1, 1)), "el PNG data: debería decodificar a 1×1");
    }

    #[test]
    fn canvas_genera_box_con_tamano_intrinseco() {
        // `<canvas>` ya no es display:none (Fase 7.196): produce un box con
        // `canvas: Some((w, h))` tomado de los atributos, default 300×150.
        let html = r##"<html><body>
            <canvas id="c1" width="200" height="120"></canvas>
            <canvas id="c2"></canvas>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Vec<(String, Option<(f32, f32)>)> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("canvas") {
                found.push((b.element_id.clone().unwrap_or_default(), b.canvas));
            }
        });
        assert_eq!(
            found,
            vec![
                ("c1".to_string(), Some((200.0, 120.0))),
                ("c2".to_string(), Some((300.0, 150.0))),
            ],
            "canvas con atributos toma su tamaño; sin atributos cae a 300×150"
        );
    }

    #[test]
    fn counter_numera_h2_sequencialmente() {
        // Patrón clásico: body resetea el contador a 0, cada h2::before
        // lo incrementa y muestra el valor — h2 numerados 1, 2, 3.
        let html = r##"<html><head><style>
            body { counter-reset: sec }
            h2::before { counter-increment: sec; content: counter(sec) ". " }
        </style></head><body>
            <h2>Intro</h2>
            <h2>Cuerpo</h2>
            <h2>Cierre</h2>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Recolectamos el primer text leaf de cada h2 (el ::before).
        let mut h2_prefixes: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("h2") {
                if let Some(first) = b.children.first() {
                    if let Some(t) = &first.text {
                        h2_prefixes.push(t.clone());
                    }
                }
            }
        });
        assert_eq!(h2_prefixes, vec!["1. ", "2. ", "3. "]);
    }

    #[test]
    fn attr_en_content_lee_del_padre_del_pseudo() {
        // `<a data-tag="X">` con `a::after { content: " [" attr(data-tag) "]" }`
        // debe inyectar " [X]" después del texto del link.
        let html = r##"<html><head><style>
            a::after { content: " [" attr(data-tag) "]" }
        </style></head><body>
            <a href="#" data-tag="ALPHA">link</a>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut a_children: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") && a_children.is_empty() {
                a_children = b
                    .children
                    .iter()
                    .filter_map(|c| c.text.clone())
                    .collect();
            }
        });
        assert_eq!(a_children, vec!["link".to_string(), " [ALPHA]".to_string()]);
    }

    #[test]
    fn before_y_after_se_inyectan_como_children() {
        let html = r##"<html><head><style>
            .badge::before { content: "▸ " }
            .badge::after  { content: " !" }
        </style></head><body>
            <p class="badge">Hola</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // El `<p>` tiene 3 hijos: el ::before, el text leaf "Hola", el ::after.
        let mut p_children: Option<Vec<String>> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && p_children.is_none() {
                p_children = Some(
                    b.children
                        .iter()
                        .filter_map(|c| c.text.clone())
                        .collect(),
                );
            }
        });
        let texts = p_children.expect("debería encontrar <p>");
        assert_eq!(texts, vec!["▸ ".to_string(), "Hola".to_string(), " !".to_string()]);
    }

    #[test]
    fn find_y_of_match_devuelve_y_creciente_por_match() {
        let html = r##"<html><body>
            <p>alfa</p><p>beta</p><p>alfa beta</p><p>alfa</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let bt = &doc.box_tree;
        let y1 = bt.find_y_of_match("alfa", 1).expect("match 1");
        let y2 = bt.find_y_of_match("alfa", 2).expect("match 2");
        let y3 = bt.find_y_of_match("alfa", 3).expect("match 3");
        assert!(y2 > y1, "match 2 debe quedar más abajo que match 1");
        assert!(y3 > y2);
        // Sin match para el 4to.
        assert!(bt.find_y_of_match("alfa", 4).is_none());
        // Query vacía o nth=0 devuelven None.
        assert!(bt.find_y_of_match("", 1).is_none());
        assert!(bt.find_y_of_match("alfa", 0).is_none());
    }

    #[test]
    fn input_autofocus_se_marca_solo_para_inputs_con_attr() {
        let html = r##"<html><body>
            <form>
                <input type="text" name="a">
                <input type="text" name="b" autofocus>
                <input type="text" name="c" autofocus>
            </form>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut flags: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.input_kind.is_some() {
                flags.push(b.input_autofocus);
            }
        });
        assert_eq!(flags, vec![false, true, true]);
    }

    #[test]
    fn element_id_se_extrae_del_attr() {
        let html = r##"<html><body>
            <h2 id="intro">Intro</h2>
            <p id="">vacío no cuenta</p>
            <p>sin id</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut ids: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(id) = &b.element_id {
                ids.push(id.clone());
            }
        });
        assert_eq!(ids, vec!["intro".to_string()]);
    }

    #[test]
    fn ws_solo_inline_no_se_dropea_si_padre_es_inline_flow() {
        // <p>foo<span> </span>bar</p> — el espacio dentro de span sí debe
        // quedar porque separa "foo" de "bar".
        let html = "<html><body><p>foo<span> </span>bar</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found_space = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("span") {
                for c in &b.children {
                    if c.text.as_deref().map(|s| s.contains(' ')).unwrap_or(false) {
                        found_space = true;
                    }
                }
            }
        });
        assert!(found_space, "el espacio dentro de <span> debería preservarse");
    }

    #[test]
    fn set_element_text_content_reemplaza_hoja() {
        let html = r#"<html><body><h1 id="hero">Hola</h1></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_text_content("hero", "Adiós");
        assert!(ok);
        // Verificar que la hoja de texto se actualizó.
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("Adiós") {
                found = true;
            }
        });
        assert!(found, "no se encontró 'Adiós' en el árbol post-mutación");
    }

    #[test]
    fn set_element_text_content_no_encuentra_id_devuelve_false() {
        let html = r#"<html><body><p>x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_text_content("fantasma", "x");
        assert!(!ok);
    }

    #[test]
    fn set_element_style_color_actualiza_text_leaves() {
        let html = r#"<html><body><h1 id="h">hola</h1></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_style("h", "color", "red");
        assert!(ok);
        // El leaf de texto debe haber heredado el color rojo.
        let mut color_changed = false;
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("hola") {
                if b.color.r == 255 && b.color.g == 0 && b.color.b == 0 {
                    color_changed = true;
                }
            }
        });
        assert!(color_changed);
    }

    #[test]
    fn set_element_style_background_hex() {
        let html = r#"<html><body><div id="d">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_style("d", "background", "#abc"));
        let mut bg_set = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("d") {
                if let Some(c) = b.background {
                    if c.r == 0xaa && c.g == 0xbb && c.b == 0xcc {
                        bg_set = true;
                    }
                }
            }
        });
        assert!(bg_set);
    }

    #[test]
    fn set_element_style_display_none_oculta() {
        let html = r#"<html><body><div id="d">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_style("d", "display", "none"));
        let mut hidden = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("d") {
                if matches!(b.display, Display::None) {
                    hidden = true;
                }
            }
        });
        assert!(hidden);
    }

    #[test]
    fn set_element_style_prop_desconocida_devuelve_false() {
        let html = r#"<html><body><div id="d">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(!doc.box_tree.set_element_style("d", "transform", "rotate(45deg)"));
    }

    #[test]
    fn set_element_style_id_inexistente_devuelve_false() {
        let html = r#"<html><body><p>x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(!doc.box_tree.set_element_style("fantasma", "color", "red"));
    }

    // ============= Fase 7.16 — attributes genéricos =============

    #[test]
    fn box_node_attributes_contiene_todos_los_attrs_html() {
        let html = r#"<html><body><a id="x" href="https://gioser.net" aria-current="page" data-track="hero" rel="noopener">x</a></body></html>"#;
        let doc = Engine::new().load_html("about:t", html);
        let mut found: Option<Vec<(String, String)>> = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                found = Some(b.attributes.clone());
            }
        });
        let attrs = found.expect("a#x existe");
        // Todos los attrs aparecen, lowercase names, values literales.
        assert!(attrs.iter().any(|(k, v)| k == "href" && v == "https://gioser.net"));
        assert!(attrs.iter().any(|(k, v)| k == "aria-current" && v == "page"));
        assert!(attrs.iter().any(|(k, v)| k == "data-track" && v == "hero"));
        assert!(attrs.iter().any(|(k, v)| k == "rel" && v == "noopener"));
        // El attr id también aparece — no se filtra (el getAttribute('id')
        // resuelve por la rama especial del JS, pero el campo se mantiene
        // uniforme para evitar ramas adicionales en el chrome).
        assert!(attrs.iter().any(|(k, v)| k == "id" && v == "x"));
    }

    #[test]
    fn box_node_dataset_filter_view_devuelve_solo_data_attrs() {
        let html = r##"<html><body><div id="x" data-foo="1" aria-label="hi" data-bar-baz="2" href="#">y</div></body></html>"##;
        let doc = Engine::new().load_html("about:t", html);
        let mut found: Option<Vec<(String, String)>> = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                found = Some(b.dataset().into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
            }
        });
        let ds = found.expect("div#x existe");
        assert_eq!(ds.len(), 2);
        assert!(ds.iter().any(|(k, v)| k == "foo" && v == "1"));
        assert!(ds.iter().any(|(k, v)| k == "bar-baz" && v == "2"));
    }

    #[test]
    fn set_element_attribute_agrega_attr_nuevo() {
        let html = r#"<html><body><div id="x">y</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_attribute("x", "aria-current", "step"));
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, v)| k == "aria-current" && v == "step")
            {
                found = true;
            }
        });
        assert!(found);
    }

    #[test]
    fn set_element_attribute_reemplaza_attr_existente() {
        let html = r#"<html><body><a id="x" href="/old">y</a></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_attribute("x", "href", "/nuevo"));
        let mut count_href = 0;
        let mut val = String::new();
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                for (k, v) in &b.attributes {
                    if k == "href" {
                        count_href += 1;
                        val = v.clone();
                    }
                }
            }
        });
        assert_eq!(count_href, 1, "href no debe duplicarse al reemplazar");
        assert_eq!(val, "/nuevo");
    }

    #[test]
    fn remove_element_attribute_quita_la_key() {
        let html = r#"<html><body><a id="x" href="/x" aria-label="hi">y</a></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.remove_element_attribute("x", "aria-label"));
        let mut still = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, _)| k == "aria-label")
            {
                still = true;
            }
        });
        assert!(!still);
    }

    #[test]
    fn set_element_dataset_wrapper_usa_set_element_attribute() {
        let html = r#"<html><body><div id="x">y</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        // El wrapper de Fase 7.11 ahora delega a set_element_attribute
        // con el prefijo data-; verificamos que ambos vean el mismo store.
        assert!(doc.box_tree.set_element_dataset("x", "role", "main"));
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, v)| k == "data-role" && v == "main")
            {
                found = true;
            }
        });
        assert!(found, "set_element_dataset debe poblar attributes con data-<key>");
    }

    #[test]
    fn set_element_attribute_id_inexistente_devuelve_false() {
        let html = r#"<html><body><p>x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(!doc.box_tree.set_element_attribute("fantasma", "href", "/"));
    }

    #[test]
    fn set_element_text_content_reemplaza_primer_leaf_no_los_demas() {
        let html = r#"<html><body><div id="d"><span>uno</span><span>dos</span></div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_text_content("d", "X");
        assert!(ok);
        let mut texts = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if !t.trim().is_empty() {
                    texts.push(t.clone());
                }
            }
        });
        // El primer text leaf "uno" pasa a "X"; "dos" sigue intacto.
        assert!(texts.contains(&"X".to_string()), "texts: {texts:?}");
        assert!(texts.contains(&"dos".to_string()), "texts: {texts:?}");
        assert!(!texts.contains(&"uno".to_string()), "texts: {texts:?}");
    }

    #[test]
    fn box_tree_resuelve_animation_contra_keyframes() {
        // `animation: fade …` + `@keyframes fade` debe poblar BoxNode.animation
        // (Tier B: wiring del runtime de tween rescatado de engine).
        let html = r##"<html><head><style>
            @keyframes fade { from { opacity: 0 } to { opacity: 1 } }
            #target { animation: fade 2s linear }
        </style></head><body><div id="target">hola</div></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("target") {
                let inst = b.animation.as_ref().expect("div animado sin AnimationInstance");
                assert_eq!(inst.binding.name, "fade");
                // A mitad de los 2s (linear) la opacity interpolada ≈ 0.5.
                let p = crate::anim::animation_progress(&inst.binding, 1.0).unwrap();
                let ov = crate::anim::sample_keyframes(&inst.keyframes, p);
                let op = ov.opacity.expect("keyframes fade interpola opacity");
                assert!((op - 0.5).abs() < 0.05, "opacity a mitad: {op}");
                found = true;
            }
        });
        assert!(found, "no se encontró #target en el box tree");
    }

    #[test]
    fn box_tree_animation_none_sin_keyframes_match() {
        // `animation: <name>` sin `@keyframes <name>` → animation: None.
        let html = r##"<html><head><style>
            #x { animation: noexiste 1s }
        </style></head><body><div id="x">a</div></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut checked = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                assert!(b.animation.is_none(), "no debería resolver sin @keyframes");
                checked = true;
            }
        });
        assert!(checked);
    }
}
