//! Tests del motor de estilo (grupo 12, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn parsea_align_content_valores_y_alias() {
        assert_eq!(parse_align_content("space-between"), Some(AlignContent::SpaceBetween));
        assert_eq!(parse_align_content("flex-start"), Some(AlignContent::Start));
        assert_eq!(parse_align_content("flex-end"), Some(AlignContent::End));
        assert_eq!(parse_align_content("center"), Some(AlignContent::Center));
        assert_eq!(parse_align_content("stretch"), Some(AlignContent::Stretch));
        // `normal` y `baseline` colapsan al default.
        assert_eq!(parse_align_content("normal"), Some(AlignContent::Normal));
        assert_eq!(parse_align_content("baseline"), Some(AlignContent::Normal));
        assert_eq!(parse_align_content("garbage"), None);
    }

    #[test]
    fn align_content_computa_en_flex_y_default_normal() {
        let html = r#"<html><head><style>
            .multi { display: flex; flex-wrap: wrap; align-content: space-around; }
        </style></head><body>
            <div class="multi"><span>a</span></div>
            <section style="display:flex"><span>b</span></section>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let multi = dom.find("div").unwrap();
        assert_eq!(eng.compute(&multi).align_content, AlignContent::SpaceAround);
        // Sin declaración, el default es Normal (no hereda del flujo).
        let plain = dom.find("section").unwrap();
        assert_eq!(eng.compute(&plain).align_content, AlignContent::Normal);
    }

    #[test]
    fn place_shorthands_expanden_ambos_ejes() {
        let html = r#"<html><head><style>
            .a { display: grid; place-content: center space-between; }
            .b { display: grid; place-items: stretch; }
            .c { place-self: end center; }
        </style></head><body>
            <div class="a"></div><div class="b"></div>
            <span class="c">x</span>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        // place-content: align-content + justify-content.
        let pc = parse_declarations("place-content: center space-between", &HashMap::new());
        assert!(pc.iter().any(|d| matches!(d.kind, DeclKind::AlignContent(AlignContent::Center))));
        assert!(pc
            .iter()
            .any(|d| matches!(d.kind, DeclKind::JustifyContent(JustifyContent::SpaceBetween))));
        // place-items con un solo valor → align-items + justify-items iguales.
        let pi = parse_declarations("place-items: stretch", &HashMap::new());
        assert!(pi.iter().any(|d| matches!(d.kind, DeclKind::AlignItems(AlignItems::Stretch))));
        assert!(pi.iter().any(|d| matches!(d.kind, DeclKind::JustifyItems(AlignItems::Stretch))));
        // place-self: align-self + justify-self.
        let ps = parse_declarations("place-self: end center", &HashMap::new());
        assert!(ps.iter().any(|d| matches!(d.kind, DeclKind::AlignSelf(AlignSelf::End))));
        assert!(ps.iter().any(|d| matches!(d.kind, DeclKind::JustifySelf(AlignSelf::Center))));
        // Y que computa end-to-end sobre el árbol.
        let a = eng.compute(&dom.find("div").unwrap());
        assert_eq!(a.align_content, AlignContent::Center);
        assert_eq!(a.justify_content, JustifyContent::SpaceBetween);
        let c = eng.compute(&dom.find("span").unwrap());
        assert_eq!(c.align_self, AlignSelf::End);
        assert_eq!(c.justify_self, AlignSelf::Center);
    }

    #[test]
    fn justify_items_y_self_grid_parse_y_computa() {
        // Parsers (incluye alias left/right y descarte de `normal`).
        assert_eq!(parse_justify_items("center"), Some(AlignItems::Center));
        assert_eq!(parse_justify_items("left"), Some(AlignItems::Start));
        assert_eq!(parse_justify_items("right"), Some(AlignItems::End));
        assert_eq!(parse_justify_items("stretch"), Some(AlignItems::Stretch));
        assert_eq!(parse_justify_items("normal"), None);
        assert_eq!(parse_justify_self("auto"), Some(AlignSelf::Auto));
        assert_eq!(parse_justify_self("right"), Some(AlignSelf::End));
        assert_eq!(parse_justify_self("flex-start"), Some(AlignSelf::Start));

        let html = r#"<html><head><style>
            .g { display: grid; justify-items: center; }
            .cell { justify-self: end; }
        </style></head><body>
            <div class="g"><span class="cell">x</span></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let g = eng.compute(&dom.find("div").unwrap());
        assert_eq!(g.justify_items, Some(AlignItems::Center));
        let cell = eng.compute(&dom.find("span").unwrap());
        assert_eq!(cell.justify_self, AlignSelf::End);
        // Default sin declaración.
        assert_eq!(g.justify_self, AlignSelf::Auto);
    }

    #[test]
    fn aspect_ratio_propiedad_ratio_numero_y_auto() {
        let html = r#"<html><head><style>
            .wide { aspect-ratio: 16 / 9; }
            .num  { aspect-ratio: 1.5; }
            .both { aspect-ratio: auto 4/3; }
            .reset{ aspect-ratio: auto; }
        </style></head><body>
            <div class="wide"></div><div class="num"></div>
            <div class="both"></div><div class="reset"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        // Verificamos el parse vía decl_kind_from_pair (más preciso que
        // depender del orden de los div en el árbol).
        let r = |css: &str| match decl_kind_from_pair("aspect-ratio", css) {
            Some(DeclKind::AspectRatio(v)) => v,
            other => panic!("inesperado: {other:?}"),
        };
        assert!((r("16 / 9").unwrap() - 16.0 / 9.0).abs() < 1e-6);
        assert!((r("1.5").unwrap() - 1.5).abs() < 1e-6);
        assert!((r("auto 4/3").unwrap() - 4.0 / 3.0).abs() < 1e-6);
        assert_eq!(r("auto"), None);
        assert!(decl_kind_from_pair("aspect-ratio", "garbage").is_none());
        // Y que computa en el árbol (default None sin declaración).
        let plain = eng.compute(&dom.find("body").unwrap());
        assert_eq!(plain.aspect_ratio, None);
    }

    #[test]
    fn row_gap_y_column_gap_individuales_pisan_shorthand() {
        let html = r#"<html><head><style>
            div {
                display: flex;
                gap: 10px;
                row-gap: 30px;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        // row-gap pisa la mitad del shorthand; column-gap del shorthand sigue (10).
        assert_eq!(s.gap_row, 30.0);
        assert_eq!(s.gap_column, 10.0);
    }

    #[test]
    fn css_var_basico_sobre_root() {
        let html = r#"<html><head><style>
            :root { --primary: #ff0000 }
            p { color: var(--primary) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn css_var_con_fallback() {
        // `--missing` no existe → usa el fallback `blue`.
        let html = r#"<html><head><style>
            p { color: var(--missing, blue) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn css_var_se_declara_en_html_y_asterisco() {
        // Variables declaradas en `html` y `*` también valen (no solo `:root`).
        let html = r#"<html><head><style>
            html { --a: #aa0000 }
            * { --b: 5px }
            p { color: var(--a); margin: var(--b) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.color, Color::rgb(0xaa, 0, 0));
        assert_eq!(s.margin.top, 5.0);
    }

    #[test]
    fn css_var_recursiva() {
        // `--secondary` se define como `var(--primary)` — la sustitución
        // debe resolver hasta el valor base.
        let html = r#"<html><head><style>
            :root {
                --primary: rgb(0, 200, 100);
                --secondary: var(--primary);
            }
            p { color: var(--secondary) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 200, 100));
    }

    #[test]
    fn css_var_en_inline_style() {
        // `style="..."` también debe resolver var().
        let html = r#"<html><head><style>
            :root { --hi: hsl(120, 100%, 50%) }
        </style></head><body>
          <p style="background: var(--hi)">x</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).background, Some(Color::rgb(0, 255, 0)));
    }

    #[test]
    fn css_var_inexistente_sin_fallback_borra_declaracion() {
        // `var(--nope)` sin fallback resuelve a "" — el parser de color
        // rechaza el value y la decl se ignora silenciosamente.
        // El color debe quedar en el default BLACK heredado.
        let html = r#"<html><head><style>
            p { color: var(--nope) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::BLACK);
    }

    #[test]
    fn css_var_multiple_en_un_value() {
        // Shorthand `border: var(--w) solid var(--c)`.
        let html = r#"<html><head><style>
            :root { --w: 3px; --c: orange }
            div { border: var(--w) solid var(--c) }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert!((s.border_widths.top - 3.0).abs() < 1e-6);
        assert_eq!(s.border_colors.top, Some(Color::rgb(255, 165, 0)));
    }

    #[test]
    fn parsea_box_sizing() {
        assert_eq!(parse_box_sizing("content-box"), Some(BoxSizing::ContentBox));
        assert_eq!(parse_box_sizing("border-box"), Some(BoxSizing::BorderBox));
        assert_eq!(parse_box_sizing("WeIrD"), None);
    }

    #[test]
    fn computa_min_max_sizes() {
        let html = r#"<html><head><style>
            div {
                min-width: 100px;
                min-height: 50px;
                max-height: 200px;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert!(matches!(s.min_width, LengthVal::Px(100.0)));
        assert!(matches!(s.min_height, LengthVal::Px(50.0)));
        assert!(matches!(s.max_height, LengthVal::Px(200.0)));
    }

    #[test]
    fn parsea_overflow_alias() {
        assert_eq!(parse_overflow("visible"), Some(Overflow::Visible));
        assert_eq!(parse_overflow("hidden"), Some(Overflow::Hidden));
        assert_eq!(parse_overflow("auto"), Some(Overflow::Hidden));
        assert_eq!(parse_overflow("scroll"), Some(Overflow::Hidden));
        assert_eq!(parse_overflow("clip"), Some(Overflow::Hidden));
    }

    #[test]
    fn parsea_white_space_y_se_hereda() {
        let html = r#"<html><head><style>
            pre { white-space: pre }
        </style></head><body><pre>line1
line2</pre></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let pre = dom.find("pre").unwrap();
        let s = eng.compute(&pre);
        assert_eq!(s.white_space, WhiteSpace::Pre);
    }

    #[test]
    fn parsea_text_transform_y_se_hereda() {
        let html = r#"<html><head><style>
            p { text-transform: uppercase }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.text_transform, TextTransform::Uppercase);
    }

    #[test]
    fn parsea_opacity_clampa() {
        assert_eq!(parse_opacity("0.5"), Some(0.5));
        assert_eq!(parse_opacity("100%"), Some(1.0));
        assert_eq!(parse_opacity("0"), Some(0.0));
        assert_eq!(parse_opacity("2"), Some(1.0)); // clamp arriba
        assert_eq!(parse_opacity("-0.5"), Some(0.0)); // clamp abajo
    }

    #[test]
    fn parsea_align_self() {
        assert_eq!(parse_align_self("auto"), Some(AlignSelf::Auto));
        assert_eq!(parse_align_self("flex-end"), Some(AlignSelf::End));
        assert_eq!(parse_align_self("stretch"), Some(AlignSelf::Stretch));
    }

    #[test]
    fn parsea_flex_shorthand_presets() {
        let decls = parse_flex_shorthand("none", false);
        assert_eq!(decls.len(), 3);
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 0.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 0.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Auto)));

        let decls = parse_flex_shorthand("auto", false);
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 1.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 1.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Auto)));

        let decls = parse_flex_shorthand("1", false);
        // `flex: 1` ⇒ `1 1 0%`
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 1.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 1.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Pct(0.0))));
    }

    #[test]
    fn parsea_flex_shorthand_3_valores() {
        let decls = parse_flex_shorthand("2 0 200px", false);
        assert_eq!(decls.len(), 3);
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 2.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 0.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Px(200.0))));
    }

    #[test]
    fn parsea_outline_shorthand() {
        let decls = parse_outline_shorthand("2px solid orange", false);
        let mut has_w = false; let mut has_s = false; let mut has_c = false;
        for d in &decls {
            match &d.kind {
                DeclKind::OutlineWidth(w) => { has_w = (*w - 2.0).abs() < 1e-6; }
                DeclKind::OutlineStyle(active) => { has_s = *active; }
                DeclKind::OutlineColor(c) => { has_c = *c == Color::rgb(255, 165, 0); }
                _ => {}
            }
        }
        assert!(has_w && has_s && has_c);

        let decls = parse_outline_shorthand("none", false);
        assert_eq!(decls.len(), 1);
        assert!(matches!(decls[0].kind, DeclKind::OutlineStyle(false)));
    }

    #[test]
    fn parsea_linear_gradient_basico() {
        let g = parse_linear_gradient("to right, #f00, #00f").unwrap();
        assert!((g.angle_deg() - 90.0).abs() < 1e-6);
        assert_eq!(g.stops.len(), 2);
        assert_eq!(g.stops[0].color, Color::rgb(255, 0, 0));
        assert_eq!(g.stops[1].color, Color::rgb(0, 0, 255));

        let g = parse_linear_gradient("45deg, red 0%, blue 100%").unwrap();
        assert!((g.angle_deg() - 45.0).abs() < 1e-6);
        assert_eq!(g.stops[0].pos, Some(LengthVal::Pct(0.0)));
        assert_eq!(g.stops[1].pos, Some(LengthVal::Pct(100.0)));

        // Default 180 (top→bottom) cuando no se da dirección.
        let g = parse_linear_gradient("red, blue").unwrap();
        assert!((g.angle_deg() - 180.0).abs() < 1e-6);
    }

    #[test]
    fn parsea_background_image_gradient_y_none() {
        // `background-image: linear-gradient(...)` produce un Gradient.
        let html = r#"<html><head><style>
            div { background-image: linear-gradient(to right, red, blue) }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert!(s.background_gradient.is_some());

        // `background-image: none` deshabilita.
        let html2 = r#"<html><head><style>
            div { background-image: linear-gradient(red, blue); background-image: none }
        </style></head><body><div></div></body></html>"#;
        let dom2 = DomTree::parse(html2);
        let eng2 = StyleEngine::from_dom(&dom2);
        let d2 = dom2.find("div").unwrap();
        assert!(eng2.compute(&d2).background_gradient.is_none());
    }

    #[test]
    fn parsea_background_size_position_repeat() {
        // Fase 7.204 — keywords y valores de las tres props de background.
        let compute = |css: &str| {
            let html = format!(
                "<html><head><style>div {{ {css} }}</style></head><body><div></div></body></html>"
            );
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            eng.compute(&dom.find("div").unwrap())
        };

        // background-size
        assert_eq!(compute("background-size: cover").background_size, BackgroundSize::Cover);
        assert_eq!(compute("background-size: contain").background_size, BackgroundSize::Contain);
        assert_eq!(
            compute("background-size: 50% auto").background_size,
            BackgroundSize::Explicit { x: LengthVal::Pct(50.0), y: LengthVal::Auto }
        );
        assert_eq!(
            compute("background-size: 100px 40px").background_size,
            BackgroundSize::Explicit { x: LengthVal::Px(100.0), y: LengthVal::Px(40.0) }
        );

        // background-repeat (incluye sintaxis de dos valores)
        assert_eq!(
            compute("background-repeat: no-repeat").background_repeat,
            BackgroundRepeat::NoRepeat
        );
        assert_eq!(
            compute("background-repeat: repeat-x").background_repeat,
            BackgroundRepeat::RepeatX
        );
        assert_eq!(
            compute("background-repeat: repeat no-repeat").background_repeat,
            BackgroundRepeat::RepeatX
        );
        assert_eq!(
            compute("background-repeat: no-repeat repeat").background_repeat,
            BackgroundRepeat::RepeatY
        );

        // background-position: keyword posicional, orden invertido y %.
        let p = compute("background-position: right bottom").background_position;
        assert_eq!((p.x, p.y), (LengthVal::Pct(100.0), LengthVal::Pct(100.0)));
        let p = compute("background-position: top left").background_position; // invertido
        assert_eq!((p.x, p.y), (LengthVal::Pct(0.0), LengthVal::Pct(0.0)));
        let p = compute("background-position: 10px 20px").background_position;
        assert_eq!((p.x, p.y), (LengthVal::Px(10.0), LengthVal::Px(20.0)));
        let p = compute("background-position: center").background_position; // un solo valor
        assert_eq!((p.x, p.y), (LengthVal::Pct(50.0), LengthVal::Pct(50.0)));
    }

    #[test]
    fn shorthand_background_expande_color_imagen_posicion_size_repeat() {
        // Fase 7.205 — el shorthand `background:` reparte sus piezas en los
        // longhands. Reusa los value-parsers de cada sub-propiedad.
        let compute = |css: &str| {
            let html = format!(
                "<html><head><style>div {{ {css} }}</style></head><body><div></div></body></html>"
            );
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            eng.compute(&dom.find("div").unwrap())
        };

        // Color suelto.
        let s = compute("background: #ff0000");
        assert_eq!(s.background, Some(Color::rgb(255, 0, 0)));

        // Imagen + repeat + position / size (con `/` pegado o suelto).
        let s = compute("background: url(bg.png) no-repeat center / cover");
        assert_eq!(s.background_image_url.as_deref(), Some("bg.png"));
        assert_eq!(s.background_repeat, BackgroundRepeat::NoRepeat);
        assert_eq!(
            (s.background_position.x, s.background_position.y),
            (LengthVal::Pct(50.0), LengthVal::Pct(50.0))
        );
        assert_eq!(s.background_size, BackgroundSize::Cover);

        // `/` pegado a los tokens (`center/contain`) y orden invertido de
        // keywords de position, color al final.
        let s = compute("background: url(p.png) repeat-x top left, url(otra.png)");
        assert_eq!(s.background_image_url.as_deref(), Some("p.png")); // sólo la 1ª capa
        assert_eq!(s.background_repeat, BackgroundRepeat::RepeatX);
        assert_eq!(
            (s.background_position.x, s.background_position.y),
            (LengthVal::Pct(0.0), LengthVal::Pct(0.0)) // top left → x=0%, y=0%
        );

        // attachment/box se aceptan y descartan; el color sigue tomándose.
        let s = compute("background: green url(g.png) fixed border-box no-repeat 10px 20px / 50px");
        assert_eq!(s.background, Some(Color::rgb(0, 128, 0)));
        assert_eq!(s.background_image_url.as_deref(), Some("g.png"));
        assert_eq!(s.background_repeat, BackgroundRepeat::NoRepeat);
        assert_eq!(
            (s.background_position.x, s.background_position.y),
            (LengthVal::Px(10.0), LengthVal::Px(20.0))
        );
        assert_eq!(
            s.background_size,
            BackgroundSize::Explicit { x: LengthVal::Px(50.0), y: LengthVal::Auto }
        );
    }

    #[test]
    fn background_props_default_y_se_propagan_al_box() {
        // Defaults CSS: auto / 0% 0% / repeat. Y un override viaja al BoxNode.
        let eng = crate::Engine::new();
        let html = r#"<html><body>
            <div id="plain" style="background-image: url(x.png)"></div>
            <div id="cov" style="background-image: url(x.png); background-size: cover;
                 background-position: 50% 50%; background-repeat: no-repeat"></div>
        </body></html>"#;
        let doc = eng.load_html("about:test", html);
        let mut plain = None;
        let mut cov = None;
        doc.box_tree.walk(|b| match b.element_id.as_deref() {
            Some("plain") => plain = Some((b.background_size, b.background_repeat, b.background_position)),
            Some("cov") => cov = Some((b.background_size, b.background_repeat, b.background_position)),
            _ => {}
        });
        let (psize, prep, ppos) = plain.expect("plain box");
        assert_eq!(psize, BackgroundSize::Auto);
        assert_eq!(prep, BackgroundRepeat::Repeat);
        assert_eq!((ppos.x, ppos.y), (LengthVal::Pct(0.0), LengthVal::Pct(0.0)));
        let (csize, crep, cpos) = cov.expect("cov box");
        assert_eq!(csize, BackgroundSize::Cover);
        assert_eq!(crep, BackgroundRepeat::NoRepeat);
        assert_eq!((cpos.x, cpos.y), (LengthVal::Pct(50.0), LengthVal::Pct(50.0)));
    }

    #[test]
    fn background_origin_clip_longhand_shorthand_y_box() {
        // Fase 7.207 — `background-origin` / `background-clip`.
        let compute = |css: &str| {
            let html = format!(
                "<html><head><style>div {{ {css} }}</style></head><body><div></div></body></html>"
            );
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            eng.compute(&dom.find("div").unwrap())
        };

        // Defaults CSS: origin = padding-box, clip = border-box.
        let s = compute("color: red");
        assert_eq!(s.background_origin, BackgroundOrigin::PaddingBox);
        assert_eq!(s.background_clip, BackgroundClip::BorderBox);

        // Longhands.
        let s = compute("background-origin: content-box; background-clip: padding-box");
        assert_eq!(s.background_origin, BackgroundOrigin::ContentBox);
        assert_eq!(s.background_clip, BackgroundClip::PaddingBox);

        // `text` ahora es un valor real (Fase 7.208).
        let s = compute("background-clip: text");
        assert_eq!(s.background_clip, BackgroundClip::Text);

        // Shorthand con UNA caja → fija origin Y clip.
        let s = compute("background: url(b.png) content-box");
        assert_eq!(s.background_origin, BackgroundOrigin::ContentBox);
        assert_eq!(s.background_clip, BackgroundClip::ContentBox);

        // Shorthand con DOS cajas → 1ª = origin, 2ª = clip.
        let s = compute("background: url(b.png) padding-box content-box");
        assert_eq!(s.background_origin, BackgroundOrigin::PaddingBox);
        assert_eq!(s.background_clip, BackgroundClip::ContentBox);

        // Propagación al BoxNode (vía build).
        let eng = crate::Engine::new();
        let doc = eng.load_html(
            "about:test",
            r#"<html><body><div id="d" style="background-image: url(x.png);
               background-origin: content-box; background-clip: padding-box"></div></body></html>"#,
        );
        let mut got = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("d") {
                got = Some((b.background_origin, b.background_clip));
            }
        });
        let (o, c) = got.expect("box d");
        assert_eq!(o, BackgroundOrigin::ContentBox);
        assert_eq!(c, BackgroundClip::PaddingBox);
    }

    #[test]
    fn background_clip_text_parsea_y_propaga_a_la_hoja() {
        // Fase 7.208 — `background-clip: text` (+ `-webkit-` prefix) y la
        // propagación del gradiente del elemento estilado a su hoja de texto.
        let compute = |css: &str| {
            let html = format!(
                "<html><head><style>div {{ {css} }}</style></head><body><div></div></body></html>"
            );
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            eng.compute(&dom.find("div").unwrap())
        };
        assert_eq!(compute("background-clip: text").background_clip, BackgroundClip::Text);
        assert_eq!(
            compute("-webkit-background-clip: text").background_clip,
            BackgroundClip::Text
        );

        // El gradiente vive en el <h1>; su hoja de texto hija lo hereda junto
        // con el clip:text para rellenar los glifos.
        let eng = crate::Engine::new();
        let doc = eng.load_html(
            "about:test",
            r#"<html><body><h1 style="background-image: linear-gradient(90deg, red, blue);
               -webkit-background-clip: text; color: transparent">Hola</h1></body></html>"#,
        );
        let mut leaf = None;
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("Hola") {
                leaf = Some((b.background_clip, b.background_gradient.is_some()));
            }
        });
        let (clip, has_grad) = leaf.expect("hoja de texto Hola");
        assert_eq!(clip, BackgroundClip::Text);
        assert!(has_grad, "la hoja debería heredar el gradiente del <h1>");
    }

