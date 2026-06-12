//! Tests del motor de estilo (grupo 09, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn font_size_relativo_em_pct_keywords() {
        // `em`/`%`/`larger` resuelven contra el font-size HEREDADO (20px);
        // `rem` y los keywords absolutos quedan fijos.
        let html = r#"<html><head><style>
            .em{font-size:1.5em}
            .pct{font-size:150%}
            .larger{font-size:larger}
            .large{font-size:large}
            .rem{font-size:2rem}
        </style></head><body>
            <div style="font-size:20px">
                <p class="em">a</p><p class="pct">b</p>
                <p class="larger">c</p><p class="large">d</p>
                <p class="rem">e</p>
            </div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut div = None;
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("div") => div = Some(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        // El `<div style="font-size:20px">` es el padre heredado.
        let parent = eng.compute(div.as_ref().unwrap());
        assert_eq!(parent.font_size, 20.0);
        let fs = |i: usize| eng.compute_with_parent(&ps[i], Some(&parent)).font_size;
        assert_eq!(fs(0), 30.0); // 1.5em × 20
        assert_eq!(fs(1), 30.0); // 150% × 20
        assert!((fs(2) - 24.0).abs() < 1e-3); // larger = ×1.2 × 20
        assert_eq!(fs(3), 18.0); // large = absoluto
        assert_eq!(fs(4), 32.0); // 2rem = root 16
    }

    #[test]
    fn margin_auto_centra_horizontal() {
        // `margin: 0 auto` y longhands con `auto` marcan el flag de centrado
        // sin perder los px verticales.
        let html = r#"<html><head><style>
            .a{margin:0 auto}
            .b{margin:10px 20px 30px auto}
            .c{margin-left:auto; margin-right:auto}
            .d{margin:8px}
            .e{margin-left:auto}
            .e{margin-left:12px}
        </style></head><body>
            <div class="a">a</div><div class="b">b</div>
            <div class="c">c</div><div class="d">d</div>
            <div class="e">e</div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ds = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                ds.push(n.clone());
            }
        });
        // .a — `0 auto`: top/bottom 0, left/right auto.
        let a = eng.compute(&ds[0]);
        assert!(a.margin_left_auto && a.margin_right_auto);
        assert_eq!(a.margin.top, 0.0);
        // .b — `10 20 30 auto`: sólo left es auto; right=20px no.
        let b = eng.compute(&ds[1]);
        assert!(b.margin_left_auto && !b.margin_right_auto);
        assert_eq!(b.margin.top, 10.0);
        assert_eq!(b.margin.right, 20.0);
        assert_eq!(b.margin.bottom, 30.0);
        // .c — longhands auto en ambos lados.
        let c = eng.compute(&ds[2]);
        assert!(c.margin_left_auto && c.margin_right_auto);
        // .d — sin auto.
        let d = eng.compute(&ds[3]);
        assert!(!d.margin_left_auto && !d.margin_right_auto);
        assert_eq!(d.margin.left, 8.0);
        // .e — un px posterior pisa el auto previo (mismo selector/orden).
        let e = eng.compute(&ds[4]);
        assert!(!e.margin_left_auto);
        assert_eq!(e.margin.left, 12.0);
    }

    #[test]
    fn parsea_calc_solo_px() {
        // calc(10px + 5px) resuelve a Px(15) en parse time.
        assert_eq!(parse_length_or_pct("calc(10px + 5px)"), Some(LengthVal::Px(15.0)));
        assert_eq!(parse_length_or_pct("calc(20px - 5px)"), Some(LengthVal::Px(15.0)));
    }

    #[test]
    fn parsea_calc_solo_pct() {
        assert_eq!(parse_length_or_pct("calc(80% - 10%)"), Some(LengthVal::Pct(70.0)));
        assert_eq!(parse_length_or_pct("calc(50% + 20%)"), Some(LengthVal::Pct(70.0)));
    }

    #[test]
    fn parsea_calc_mixto_pierde_offset_px() {
        // Mezcla pct + px: conservamos el Pct e ignoramos el px (no
        // tenemos container width acá; taffy no soporta calc nativo).
        // Esto es una limitación documentada del soporte de calc.
        assert_eq!(parse_length_or_pct("calc(100% - 20px)"), Some(LengthVal::Pct(100.0)));
        assert_eq!(parse_length_or_pct("calc(50% + 10px)"), Some(LengthVal::Pct(50.0)));
    }

    #[test]
    fn parsea_calc_invalido_devuelve_none() {
        // Tokens incompletos / mismatched parens / op desconocido.
        assert!(parse_length_or_pct("calc(10px +)").is_none());
        assert!(parse_length_or_pct("calc(10px").is_none());
        // Sumar número y longitud es inválido (CSS).
        assert!(parse_length_or_pct("calc(10px + 2)").is_none());
        // longitud * longitud inválido.
        assert!(parse_length_or_pct("calc(10px * 5px)").is_none());
        // división por cero.
        assert!(parse_length_or_pct("calc(10px / 0)").is_none());
    }

    #[test]
    fn parsea_calc_mul_div_y_precedencia() {
        // `*` y `/` por escalar.
        assert_eq!(parse_length_or_pct("calc(10px * 2)"), Some(LengthVal::Px(20.0)));
        assert_eq!(parse_length_or_pct("calc(2 * 10px)"), Some(LengthVal::Px(20.0)));
        assert_eq!(parse_length_or_pct("calc(100px / 4)"), Some(LengthVal::Px(25.0)));
        // Precedencia: `*` antes que `+`.
        assert_eq!(parse_length_or_pct("calc(10px + 2 * 5px)"), Some(LengthVal::Px(20.0)));
        // Paréntesis fuerzan el orden.
        assert_eq!(parse_length_or_pct("calc((10px + 2px) * 3)"), Some(LengthVal::Px(36.0)));
        // % puro con `/`.
        assert_eq!(parse_length_or_pct("calc(90% / 3)"), Some(LengthVal::Pct(30.0)));
        // Unidades absolutas: rem→px (×16).
        assert_eq!(parse_length_or_pct("calc(1rem + 4px)"), Some(LengthVal::Px(20.0)));
    }

    #[test]
    fn parsea_min_max_clamp() {
        // min/max con px puro → exacto.
        assert_eq!(parse_length_or_pct("min(10px, 20px)"), Some(LengthVal::Px(10.0)));
        assert_eq!(parse_length_or_pct("max(10px, 20px, 5px)"), Some(LengthVal::Px(20.0)));
        // clamp(lo, val, hi) acota.
        assert_eq!(parse_length_or_pct("clamp(10px, 15px, 20px)"), Some(LengthVal::Px(15.0)));
        assert_eq!(parse_length_or_pct("clamp(10px, 5px, 20px)"), Some(LengthVal::Px(10.0)));
        assert_eq!(parse_length_or_pct("clamp(10px, 25px, 20px)"), Some(LengthVal::Px(20.0)));
        // Unidades mezcladas pero todas absolutas (rem→px) → exacto.
        assert_eq!(parse_length_or_pct("clamp(1rem, 2rem, 3rem)"), Some(LengthVal::Px(32.0)));
        // % puro.
        assert_eq!(parse_length_or_pct("max(50%, 80%)"), Some(LengthVal::Pct(80.0)));
        // Mezcla px/% incomparable → degrada al primer arg.
        assert_eq!(parse_length_or_pct("min(100%, 600px)"), Some(LengthVal::Pct(100.0)));
        // clamp incomparable → degrada al valor central.
        assert_eq!(parse_length_or_pct("clamp(1rem, 50%, 3rem)"), Some(LengthVal::Pct(50.0)));
        // calc anidado dentro de min.
        assert_eq!(parse_length_or_pct("min(calc(10px + 5px), 20px)"), Some(LengthVal::Px(15.0)));
    }

    #[test]
    fn parsea_regla_simple() {
        let rules = parse_stylesheet("p { color: red; font-size: 14px; }", &HashMap::new(), DEFAULT_VIEWPORT);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.compounds.len(), 1);
        assert!(matches!(
            &rules[0].selector.compounds[0].tag,
            TagPart::Type(t) if t == "p"
        ));
        assert_eq!(rules[0].decls.len(), 2);
    }

    #[test]
    fn selector_compound_matchea() {
        // `a.btn` matchea sólo `<a class="btn">`.
        let html = r##"<html><head><style>a.btn{color:red}</style></head><body>
                <a class="btn" href="#">click</a>
                <a href="#">otro</a>
                <span class="btn">no soy a</span>
            </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut anchors = Vec::new();
        let mut spans = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("a") => anchors.push(n.clone()),
                Some("span") => spans.push(n.clone()),
                _ => {}
            }
        });
        assert_eq!(anchors.len(), 2);
        assert_eq!(spans.len(), 1);
        // anchors[0] tiene class="btn" — `.btn { color: red }` pisa
        // el azul-de-link del UA stylesheet.
        assert_eq!(eng.compute(&anchors[0]).color, Color::rgb(255, 0, 0));
        // anchors[1] sin class — sólo aplica el UA, que pinta `<a>`
        // con el azul clásico de browser (0, 0, 238).
        assert_eq!(eng.compute(&anchors[1]).color, Color::rgb(0, 0, 238));
        // span.btn no es <a> — no aplica el UA de link.
        assert_eq!(eng.compute(&spans[0]).color, Color::BLACK);
    }

    #[test]
    fn current_color_se_resuelve_al_color() {
        let html = r#"<html><head><style>
            .a { color: red; border-color: currentColor; }
            .b { border: 2px solid currentColor; color: rgb(0,128,0); }
            .c { background-color: currentColor; color: blue; }
            .d { outline: 2px solid currentColor; color: #ff8800; }
        </style></head><body>
            <div class="a"></div>
            <div class="b"></div>
            <div class="c"></div>
            <div class="d"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 4);
        // .a — border-color: currentColor = rojo en los 4 lados.
        let a = eng.compute(&divs[0]);
        assert_eq!(a.border_colors.top, Some(Color::rgb(255, 0, 0)));
        assert_eq!(a.border_colors.left, Some(Color::rgb(255, 0, 0)));
        // El buffer transitorio queda vacío (no se hereda ni viaja al box).
        assert!(a.current_color.is_empty());
        // .b — el `color` se declara DESPUÉS del border en la regla; la
        // resolución post-pass igual lo toma (verde), no el negro previo.
        let b = eng.compute(&divs[1]);
        assert_eq!(b.border_colors.top, Some(Color::rgb(0, 128, 0)));
        assert_eq!(b.border_widths.top, 2.0);
        // .c — background = el color del elemento (azul).
        let c = eng.compute(&divs[2]);
        assert_eq!(c.background, Some(Color::rgb(0, 0, 255)));
        // .d — outline color = el color (#ff8800).
        let d = eng.compute(&divs[3]);
        assert_eq!(d.outline.color, Some(Color::rgb(255, 136, 0)));
        assert_eq!(d.outline.width, 2.0);
    }

    #[test]
    fn current_color_hereda_el_color_del_ancestro() {
        let html = r#"<html><head><style>
            .parent { color: rgb(10,20,30); }
            .child { border-color: currentColor; }
        </style></head><body>
            <div class="parent"><span class="child"></span></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let (mut parent, mut child) = (None, None);
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("div") => parent = Some(n.clone()),
                Some("span") => child = Some(n.clone()),
                _ => {}
            }
        });
        let parent = parent.unwrap();
        let child = child.unwrap();
        let ps = eng.compute(&parent);
        // El hijo no declara `color`: `currentColor` toma el heredado.
        let cs = eng.compute_with_parent(&child, Some(&ps));
        assert_eq!(cs.color, Color::rgb(10, 20, 30)); // heredado
        assert_eq!(cs.border_colors.top, Some(Color::rgb(10, 20, 30)));
    }

    #[test]
    fn pseudo_empty_matchea() {
        let html = r#"<html><head><style>div:empty{color:red}</style></head><body>
            <div class="vacio"></div>
            <div class="ws">   </div>
            <div class="texto">hola</div>
            <div class="hijo"><span></span></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let red = Color::rgb(255, 0, 0);
        assert_eq!(eng.compute(&divs[0]).color, red); // vacío
        assert_eq!(eng.compute(&divs[1]).color, red); // sólo whitespace → :empty
        assert_eq!(eng.compute(&divs[2]).color, Color::BLACK); // tiene texto
        assert_eq!(eng.compute(&divs[3]).color, Color::BLACK); // tiene hijo elemento
    }

    #[test]
    fn pseudo_root_matchea_html() {
        let html = r#"<html><head><style>:root{color:#008000}</style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut html_el = None;
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("html") {
                html_el = Some(n.clone());
            }
        });
        assert_eq!(eng.compute(&html_el.unwrap()).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn pseudo_any_link_matchea() {
        let html = r#"<html><head><style>:any-link{color:#0000ff}</style></head><body>
            <a href="/x">con</a><a>sin</a>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut anchors = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("a") {
                anchors.push(n.clone());
            }
        });
        assert_eq!(anchors.len(), 2);
        // <a href> matchea :any-link (especificidad 10 > UA `a`).
        assert_eq!(eng.compute(&anchors[0]).color, Color::rgb(0, 0, 255));
        // <a> sin href NO matchea :any-link.
        assert_ne!(eng.compute(&anchors[1]).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn pseudo_has_relacional() {
        let html = r#"<html><head><style>
            .has-span:has(span){color:red}
            .has-child:has(> .active){color:rgb(0,128,0)}
            .has-adj:has(+ p){color:rgb(0,0,255)}
        </style></head><body>
            <div id="d1" class="has-span"><span>x</span></div>
            <div id="d2" class="has-span"><b>y</b></div>
            <div id="d3" class="has-child"><em class="active"></em></div>
            <div id="d4" class="has-child"><p><em class="active"></em></p></div>
            <div id="d5" class="has-adj">t</div><p>z</p>
            <div id="d6" class="has-adj">t</div><span>z</span>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let by_id = |id: &str| -> Handle {
            let mut found = None;
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::attr(n, "id").as_deref() == Some(id) {
                    found = Some(n.clone());
                }
            });
            found.unwrap()
        };
        // Descendiente: matchea con span, no sin él.
        assert_eq!(eng.compute(&by_id("d1")).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&by_id("d2")).color, Color::BLACK);
        // Hijo directo (`> .active`): matchea sólo si es hijo DIRECTO.
        assert_eq!(eng.compute(&by_id("d3")).color, Color::rgb(0, 128, 0));
        assert_eq!(eng.compute(&by_id("d4")).color, Color::BLACK); // .active es nieto
        // Hermano adyacente (`+ p`): matchea sólo si el siguiente es <p>.
        assert_eq!(eng.compute(&by_id("d5")).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&by_id("d6")).color, Color::BLACK); // siguiente es <span>
    }

    #[test]
    fn pseudo_lang_matchea() {
        let html = r#"<html lang="en-US"><head><style>
            :lang(en){color:rgb(0,0,255)}
            .fr:lang(fr){color:rgb(0,128,0)}
        </style></head><body>
            <p id="hereda">x</p>
            <p id="propio" lang="fr" class="fr">y</p>
            <p id="otro" lang="de">z</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let by_id = |id: &str| -> Handle {
            let mut found = None;
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::attr(n, "id").as_deref() == Some(id) {
                    found = Some(n.clone());
                }
            });
            found.unwrap()
        };
        // Hereda `lang="en-US"` del <html> → :lang(en) matchea (subtag).
        assert_eq!(eng.compute(&by_id("hereda")).color, Color::rgb(0, 0, 255));
        // lang propio "fr" → .fr:lang(fr) matchea (verde), no :lang(en).
        assert_eq!(eng.compute(&by_id("propio")).color, Color::rgb(0, 128, 0));
        // lang "de" → ni :lang(en) ni :lang(fr).
        assert_eq!(eng.compute(&by_id("otro")).color, Color::BLACK);
    }

    #[test]
    fn selector_hijo_directo_matchea() {
        // `ul > li` matchea `<li>` que es hijo *directo* de `<ul>`. Un
        // `<li>` dentro de `<ol>` adentro de `<ul>` no debe matchear.
        let html = r#"<html><head><style>ul > li{color:#0a0}</style></head>
            <body>
              <ul><li>directo</li></ul>
              <ol><li>indirecto</li></ol>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(lis.len(), 2);
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&lis[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_hermano_adyacente_matchea() {
        // `h2 + p` matchea sólo el primer `<p>` inmediatamente después
        // de un `<h2>`.
        let html = r#"<html><head><style>h2+p{color:#00f}</style></head>
            <body>
              <h2>t</h2><p>uno</p><p>dos</p>
              <p>aislado</p>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_hermano_general_matchea() {
        // `h2 ~ p` matchea TODOS los `<p>` hermanos posteriores a un `<h2>`.
        let html = r#"<html><head><style>h2~p{color:#00f}</style></head>
            <body>
              <p>antes</p><h2>t</h2><p>uno</p><span>x</span><p>dos</p>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        // El primero está antes del h2 → no aplica.
        assert_eq!(eng.compute(&ps[0]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[1]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[2]).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn selector_descendiente_matchea() {
        // `.menu li` matchea sólo los `<li>` dentro de `.menu`.
        let html = r#"<html><head><style>.menu li{color:#00aa00}</style></head>
            <body>
              <ul class="menu"><li>uno</li><li>dos</li></ul>
              <ul><li>tres</li></ul>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(lis.len(), 3);
        // Los dos primeros viven en .menu → verde
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&lis[1]).color, Color::rgb(0, 0xaa, 0));
        // El tercero no
        assert_eq!(eng.compute(&lis[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_class_matchea() {
        let html = r#"<html><head><style>.alert{color:red}</style></head><body><p class="alert">x</p><p>y</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let ps: Vec<_> = {
            let mut acc = Vec::new();
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::element_name(n).as_deref() == Some("p") {
                    acc.push(n.clone());
                }
            });
            acc
        };
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_id_matchea() {
        let html = r#"<html><head><style>#hero{color:#0000ff}</style></head><body><p id="hero">x</p><p>y</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_presente() {
        // `[href]` matchea cualquier elemento con atributo `href`.
        let html = r#"<html><head><style>[href]{color:red}</style></head>
            <body><a href="x">link</a><a>sin</a><span>no a</span></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut elems = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if matches!(
                crate::dom::element_name(n).as_deref(),
                Some("a") | Some("span")
            ) {
                elems.push(n.clone());
            }
        });
        // a[href] → rojo (la regla `[href]{color:red}` con
        // especificidad 10 pisa el UA `a{color:#00ee}`); a sin href no
        // matchea pero recibe el UA = azul-link; span → BLACK default.
        assert_eq!(eng.compute(&elems[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&elems[1]).color, Color::rgb(0, 0, 238));
        assert_eq!(eng.compute(&elems[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_equals() {
        // `input[type="checkbox"]` matchea sólo el checkbox.
        let html = r##"<html><head><style>input[type="checkbox"]{color:#00aa00}</style></head>
            <body>
              <input type="checkbox">
              <input type="text">
              <input>
            </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut inputs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("input") {
                inputs.push(n.clone());
            }
        });
        assert_eq!(inputs.len(), 3);
        assert_eq!(eng.compute(&inputs[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&inputs[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&inputs[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_prefix_suffix_contains() {
        let html = r##"<html><head><style>
            a[href^="https"]{color:#00f}
            img[src$=".png"]{color:#0f0}
            div[class*="warn"]{color:#f00}
        </style></head>
        <body>
            <a href="https://x">seguro</a>
            <a href="http://x">inseguro</a>
            <img src="logo.png">
            <img src="logo.jpg">
            <div class="banner warn-strong">!!</div>
            <div class="banner">--</div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut anchors = Vec::new();
        let mut imgs = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| match crate::dom::element_name(n).as_deref() {
            Some("a") => anchors.push(n.clone()),
            Some("img") => imgs.push(n.clone()),
            Some("div") => divs.push(n.clone()),
            _ => {}
        });
        assert_eq!(eng.compute(&anchors[0]).color, Color::rgb(0, 0, 255));
        // anchors[1] no matchea `[href^="https"]` pero recibe el UA
        // de `<a>` (azul 0,0,238).
        assert_eq!(eng.compute(&anchors[1]).color, Color::rgb(0, 0, 238));
        assert_eq!(eng.compute(&imgs[0]).color, Color::rgb(0, 255, 0));
        assert_eq!(eng.compute(&imgs[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&divs[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&divs[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_first_last_only_child() {
        let html = r#"<html><head><style>
            li:first-child{color:#00f}
            li:last-child{background:#0f0}
            p:only-child{color:#f0f}
        </style></head>
        <body>
          <ul><li>a</li><li>b</li><li>c</li></ul>
          <section><p>solo</p></section>
          <section><p>uno</p><p>dos</p></section>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| match crate::dom::element_name(n).as_deref() {
            Some("li") => lis.push(n.clone()),
            Some("p") => ps.push(n.clone()),
            _ => {}
        });
        // li:first-child sólo el primero
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&lis[1]).color, Color::BLACK);
        // li:last-child sólo el tercero (background)
        assert!(eng.compute(&lis[0]).background.is_none());
        assert_eq!(eng.compute(&lis[2]).background, Some(Color::rgb(0, 255, 0)));
        // p:only-child el primero (único en su section), no los otros dos
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(255, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_first_last_of_type() {
        let html = r#"<html><head><style>
            p:first-of-type{color:#00f}
            p:last-of-type{color:#0a0}
        </style></head>
        <body>
          <div>x</div>
          <p>uno</p>
          <span>y</span>
          <p>dos</p>
          <p>tres</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        // primer <p> → azul (es :first-of-type aunque haya <div> y <span> antes)
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        // del medio → ninguno (last gana cascada al último pero a este ninguno)
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        // último <p> → verde
        assert_eq!(eng.compute(&ps[2]).color, Color::rgb(0, 0xaa, 0));
    }

    #[test]
    fn parsea_width_max_width() {
        let s = parse_stylesheet(
            "p { width: 80%; max-width: 800px } div { width: auto }",
            &HashMap::new(),
            DEFAULT_VIEWPORT,
        );
        assert_eq!(s.len(), 2);
        assert!(matches!(s[0].decls[0].kind, DeclKind::Width(LengthVal::Pct(80.0))));
        assert!(matches!(s[0].decls[1].kind, DeclKind::MaxWidth(LengthVal::Px(800.0))));
        assert!(matches!(s[1].decls[0].kind, DeclKind::Width(LengthVal::Auto)));
    }

    #[test]
    fn parsea_text_align() {
        let s = parse_stylesheet(
            "h1 { text-align: center } p { text-align: right }",
            &HashMap::new(),
            DEFAULT_VIEWPORT,
        );
        assert!(matches!(s[0].decls[0].kind, DeclKind::TextAlign(TextAlign::Center)));
        assert!(matches!(s[1].decls[0].kind, DeclKind::TextAlign(TextAlign::Right)));
    }

