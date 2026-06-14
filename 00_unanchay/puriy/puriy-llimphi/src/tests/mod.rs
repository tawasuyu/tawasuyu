// Tests partidos en grupos por tamaño (Regla 1). Sin reordenar lógica.
mod grupo_01;
mod grupo_02;
mod grupo_03;
mod grupo_04;
mod grupo_05;
mod grupo_06;
mod grupo_07;

use super::*;
    // Helpers del canvas extraídos a `canvas.rs` (pub(crate) para estos tests).
    use super::canvas::{
        canvas_brush, canvas_color, canvas_composite, canvas_font_px, canvas_shadow,
        canvas_stroke, collect_dom_image_pixels, decode_canvas_images, paint_canvas_cmds,
    };
    // Tipos peniko/kurbo que sólo los tests del canvas usan (el código no-test
    // de lib.rs ya no, tras mover el painter a canvas.rs).
    use llimphi_raster::kurbo::{Cap, Join};
    use llimphi_raster::peniko::{Brush, Extend};

    /// Helper: parsea un snippet HTML offline y devuelve el BoxTree.
    fn parse(html: &str) -> BoxTree {
        let engine = Engine::new();
        engine.load_html("about:test", html).box_tree
    }

    /// Matcher case-insensitive por substring (el default de la find bar).
    fn ci(q: &str) -> Matcher {
        Matcher::new(q, MatchOpts::default())
    }

    // ── Fase 7.31 — flujo de Msg de la find bar (sin Handle) ─────────
    // `update` necesita un `Handle` (no construible en test), pero los
    // handlers de find delegan en métodos puros de `Model`. Testeamos
    // esos métodos para cubrir el flujo open → query → next → prev.

    /// Model mínimo con una sola pestaña cuyo box tree es `parse(html)`.
    fn model_con_doc(html: &str) -> Model {
        let mut t = TabState::new("about:test".into());
        t.box_tree = Some(parse(html));
        Model {
            tabs: vec![t],
            active: 0,
            spaces: vec![Space::new("Principal", "◆")],
            active_space: 0,
            orientation: TabOrientation::Horizontal,
            theme: Theme::dark(),
            settings_open: false,
            settings: AllichayState::new(),
            addr_suggest: Vec::new(),
            zoom: 1.0,
            find_active: false,
            find_input: TextInputState::new(),
            find_current: 0,
            find_case_sensitive: false,
            find_whole_word: false,
            panel: None,
            panel_filter: TextInputState::new(),
            hover_link: None,
            start: std::time::Instant::now(),
            menu_open: None,
            edit_menu: None,
            clipboard: SystemClipboard::new(),
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
        }
    }

    fn model_con_script(inline: &str) -> Model {
        let mut t = TabState::new("about:test".into());
        t.title = "T".into();
        t.url = "about:test".into();
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some(inline.into()),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        Model {
            tabs: vec![t],
            active: 0,
            spaces: vec![Space::new("Principal", "◆")],
            active_space: 0,
            orientation: TabOrientation::Horizontal,
            theme: Theme::dark(),
            settings_open: false,
            settings: AllichayState::new(),
            addr_suggest: Vec::new(),
            zoom: 1.0,
            find_active: false,
            find_input: TextInputState::new(),
            find_current: 0,
            find_case_sensitive: false,
            find_whole_word: false,
            panel: None,
            panel_filter: TextInputState::new(),
            hover_link: None,
            start: std::time::Instant::now(),
            menu_open: None,
            edit_menu: None,
            clipboard: SystemClipboard::new(),
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
        }
    }