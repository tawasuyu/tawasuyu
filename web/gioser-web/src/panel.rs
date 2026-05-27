//! Panel de control del escritorio gioser-web.
//!
//! Se monta dentro de una ventana del sistema (igual layout que las
//! demás: titlebar + body con contenedor). El contenedor recibe el
//! chrome del panel: nav de categorías a la izquierda + secciones a
//! la derecha. Cada categoría es simple en presentación:
//!
//! * **Apariencia** — variante de theme (dark/light/aurora/sunset),
//!   acento por cuadrante, densidad.
//! * **Idioma** — es/en/qu + formato de hora.
//! * **Demos & Apps** — catálogo de las apps del SO. Cada card es un
//!   `[data-doc]` que el host engancha al sistema de ventanas
//!   (markdown reader), así no duplicamos el plumbing.
//! * **Monitor** — clock, ventanas abiertas, plataforma, viewport,
//!   js-heap, dpr. Refresca cada 2s.
//! * **Módulos** — toggles de visibilidad de las secciones del
//!   sidebar; el cambio aplica clase `.mod-hidden` y persiste.
//! * **Acerca de** — meta del escritorio.
//!
//! Persistencia: `localStorage` con prefijo `gioser.`. Si no hay
//! storage (modo privado, etc.) el panel sigue funcionando, sólo no
//! recuerda preferencias entre cargas.
//!
//! Para que los `[data-doc]` que inyectamos queden enganchados al
//! sistema de ventanas, al terminar de montar disparamos un
//! `CustomEvent("gioser:rebind")` sobre `document`, con
//! `detail = container`. El JS host escucha y llama a su
//! `bindDocTriggers(container)`.

use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    CustomEvent, CustomEventInit, Document, Element, HtmlElement, HtmlInputElement, Storage,
    Window,
};

const STORE_VARIANT: &str = "gioser.theme.variant";
const STORE_ACCENT: &str = "gioser.theme.accent";
const STORE_DENSITY: &str = "gioser.theme.density";
const STORE_LANG: &str = "gioser.lang";
const STORE_TIMEFMT: &str = "gioser.timefmt";
const STORE_MODS_HIDDEN: &str = "gioser.mods.hidden";

pub struct Panel {
    container: HtmlElement,
}

impl Panel {
    pub fn new(container: HtmlElement) -> Self {
        Self { container }
    }

    pub fn mount(&self) -> Result<(), JsValue> {
        let win = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        let doc = win.document().ok_or_else(|| JsValue::from_str("no doc"))?;
        let storage = win.local_storage().ok().flatten();

        let state = State::load(storage.as_ref());

        self.container.set_inner_html(&render_html(&state));

        // Aplicar al documento las preferencias persistidas.
        apply_theme(&doc, &state)?;
        apply_modules(&doc, &state.mods_hidden)?;

        // Wire-up de la nav (cambia categoría visible).
        self.bind_nav()?;

        // Wire-up de controles segmentados (variant/accent/density/lang/timefmt).
        self.bind_segmented(STORE_VARIANT, &state.variant)?;
        self.bind_segmented(STORE_ACCENT, &state.accent)?;
        self.bind_segmented(STORE_DENSITY, &state.density)?;
        self.bind_segmented(STORE_LANG, &state.lang)?;
        self.bind_segmented(STORE_TIMEFMT, &state.timefmt)?;

        // Toggles de módulos.
        self.bind_module_toggles()?;

        // Monitor: refresca y arranca timer.
        self.refresh_monitor(&win)?;
        self.start_monitor_ticker()?;

        // Dispara rebind para los [data-doc] de la sección demos.
        let init = CustomEventInit::new();
        init.set_detail(&self.container);
        let ev = CustomEvent::new_with_event_init_dict("gioser:rebind", &init)?;
        doc.dispatch_event(&ev)?;

        Ok(())
    }

    fn bind_nav(&self) -> Result<(), JsValue> {
        let nav = self.container.query_selector(".panel-nav")?.ok_or_else(|| JsValue::from_str("no nav"))?;
        let content = self.container.query_selector(".panel-content")?.ok_or_else(|| JsValue::from_str("no content"))?;
        let nav_el: HtmlElement = nav.dyn_into()?;
        let content_el: HtmlElement = content.dyn_into()?;

        let nav_clone = nav_el.clone();
        let cb = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |e: web_sys::MouseEvent| {
            let target = match e.target() {
                Some(t) => t,
                None => return,
            };
            let el: Element = match target.dyn_into() {
                Ok(el) => el,
                Err(_) => return,
            };
            let item = el.closest(".panel-nav-item").ok().flatten();
            let item = match item {
                Some(i) => i,
                None => return,
            };
            let cat = match item.get_attribute("data-cat") {
                Some(c) => c,
                None => return,
            };
            // marca activo
            if let Ok(items) = nav_clone.query_selector_all(".panel-nav-item") {
                for i in 0..items.length() {
                    if let Some(n) = items.item(i) {
                        if let Ok(el) = n.dyn_into::<Element>() {
                            let _ = el.class_list().remove_1("active");
                        }
                    }
                }
            }
            let _ = item.class_list().add_1("active");
            // muestra sección
            if let Ok(secs) = content_el.query_selector_all(".panel-section") {
                for i in 0..secs.length() {
                    if let Some(n) = secs.item(i) {
                        if let Ok(el) = n.dyn_into::<Element>() {
                            let _ = el.class_list().remove_1("active");
                        }
                    }
                }
            }
            if let Ok(Some(sec)) = content_el.query_selector(&format!(".panel-section[data-cat=\"{}\"]", cat)) {
                let _ = sec.class_list().add_1("active");
            }
        });
        nav_el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
        cb.forget();
        Ok(())
    }

    /// Bind genérico de un grupo segmented. Identifica el grupo por
    /// `data-group=<store_key>`; al click guarda en localStorage y
    /// aplica el efecto correspondiente al documento.
    fn bind_segmented(&self, store_key: &str, current: &str) -> Result<(), JsValue> {
        let selector = format!(".panel-segmented[data-group=\"{}\"]", store_key);
        let group = match self.container.query_selector(&selector)? {
            Some(g) => g,
            None => return Ok(()),
        };
        // marca el activo inicial
        mark_segmented_active(&group, current)?;

        let store_key_s = store_key.to_string();
        let group_clone: HtmlElement = group.clone().dyn_into()?;
        let cb = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |e: web_sys::MouseEvent| {
            let target = match e.target() {
                Some(t) => t,
                None => return,
            };
            let el: Element = match target.dyn_into() {
                Ok(el) => el,
                Err(_) => return,
            };
            let btn = el.closest(".panel-seg").ok().flatten();
            let btn = match btn {
                Some(b) => b,
                None => return,
            };
            let value = match btn.get_attribute("data-value") {
                Some(v) => v,
                None => return,
            };
            // persistir
            if let Some(win) = web_sys::window() {
                if let Ok(Some(storage)) = win.local_storage() {
                    let _ = storage.set_item(&store_key_s, &value);
                }
                if let Some(doc) = win.document() {
                    apply_after_change(&doc, &store_key_s, &value);
                }
            }
            let _ = mark_segmented_active(&group_clone, &value);
        });
        let g_el: HtmlElement = group.dyn_into()?;
        g_el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
        cb.forget();
        Ok(())
    }

    fn bind_module_toggles(&self) -> Result<(), JsValue> {
        let list = match self.container.query_selector(".panel-mod-list")? {
            Some(l) => l,
            None => return Ok(()),
        };
        let list_el: HtmlElement = list.dyn_into()?;
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            let target = match e.target() {
                Some(t) => t,
                None => return,
            };
            let input: HtmlInputElement = match target.dyn_into() {
                Ok(i) => i,
                Err(_) => return,
            };
            if input.type_() != "checkbox" {
                return;
            }
            let key = match input.get_attribute("data-mod") {
                Some(k) => k,
                None => return,
            };
            let win = match web_sys::window() {
                Some(w) => w,
                None => return,
            };
            let doc = match win.document() {
                Some(d) => d,
                None => return,
            };
            // input.checked = visible? con switch on = visible
            let visible = input.checked();
            // localStorage: lista CSV de oculto
            let mut hidden = read_mods_hidden(win.local_storage().ok().flatten().as_ref());
            hidden.retain(|h| h != &key);
            if !visible {
                hidden.push(key.clone());
            }
            if let Ok(Some(storage)) = win.local_storage() {
                let _ = storage.set_item(STORE_MODS_HIDDEN, &hidden.join(","));
            }
            let _ = apply_modules(&doc, &hidden);
        });
        list_el.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())?;
        cb.forget();
        Ok(())
    }

    fn refresh_monitor(&self, win: &Window) -> Result<(), JsValue> {
        let doc = win.document().ok_or_else(|| JsValue::from_str("no doc"))?;
        // hora
        let timefmt = win.local_storage().ok().flatten()
            .and_then(|s| s.get_item(STORE_TIMEFMT).ok().flatten())
            .unwrap_or_else(|| "24h".into());
        let date = js_sys::Date::new_0();
        let h = date.get_hours() as u32;
        let m = date.get_minutes() as u32;
        let s = date.get_seconds() as u32;
        let clock = if timefmt == "12h" {
            let ampm = if h >= 12 { "pm" } else { "am" };
            let h12 = match h % 12 { 0 => 12, x => x };
            format!("{:02}:{:02}:{:02} {}", h12, m, s, ampm)
        } else {
            format!("{:02}:{:02}:{:02}", h, m, s)
        };
        set_stat(&doc, "clock", &clock);

        // ventanas abiertas (cuenta .window en #windows-layer)
        if let Some(layer) = doc.get_element_by_id("windows-layer") {
            let total = layer.query_selector_all(".window")?.length();
            let mini = layer.query_selector_all(".window[style*=\"display: none\"]")?.length();
            set_stat(&doc, "windows", &format!("{} abiertas", total));
            set_stat_sub(&doc, "windows", &format!("{} minimizadas", mini));
        }

        // viewport
        let vw = win.inner_width()?.as_f64().unwrap_or(0.0) as u32;
        let vh = win.inner_height()?.as_f64().unwrap_or(0.0) as u32;
        set_stat(&doc, "viewport", &format!("{}×{}", vw, vh));
        let dpr = win.device_pixel_ratio();
        set_stat_sub(&doc, "viewport", &format!("dpr {:.2}", dpr));

        // plataforma
        let nav = win.navigator();
        let platform = nav.platform().unwrap_or_else(|_| "—".into());
        set_stat(&doc, "platform", &platform);
        let ua = nav.user_agent().unwrap_or_else(|_| "—".into());
        let ua_short = ua.chars().take(40).collect::<String>();
        set_stat_sub(&doc, "platform", &ua_short);

        // heap (no expuesto por web-sys; lo leemos de performance.memory si existe)
        if let Some(perf) = win.performance() {
            let mem = js_sys::Reflect::get(&perf, &JsValue::from_str("memory")).ok();
            if let Some(mem) = mem {
                if !mem.is_undefined() {
                    let used = js_sys::Reflect::get(&mem, &JsValue::from_str("usedJSHeapSize"))
                        .ok()
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    let total = js_sys::Reflect::get(&mem, &JsValue::from_str("totalJSHeapSize"))
                        .ok()
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    set_stat(&doc, "heap", &format!("{:.1} MB", used / (1024.0 * 1024.0)));
                    set_stat_sub(&doc, "heap", &format!("de {:.0} MB", total / (1024.0 * 1024.0)));
                }
            }
        }

        // pantalla
        if let Ok(screen) = win.screen() {
            let sw = screen.width().unwrap_or(0);
            let sh = screen.height().unwrap_or(0);
            set_stat(&doc, "screen", &format!("{}×{}", sw, sh));
            set_stat_sub(&doc, "screen", &format!("{} bpp", screen.color_depth().unwrap_or(0)));
        }

        // idiomas del navegador
        let arr = nav.languages();
        let mut langs: Vec<String> = Vec::new();
        for i in 0..arr.length() {
            if let Some(s) = arr.get(i).as_string() {
                langs.push(s);
            }
        }
        if let Some(first) = langs.first() {
            set_stat(&doc, "browser-lang", first);
            set_stat_sub(&doc, "browser-lang", &langs.join(" · "));
        } else {
            let l = nav.language().unwrap_or_default();
            set_stat(&doc, "browser-lang", &l);
        }

        Ok(())
    }

    fn start_monitor_ticker(&self) -> Result<(), JsValue> {
        let win = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        let container = self.container.clone();
        let cb = Closure::<dyn FnMut()>::new(move || {
            // Si el panel ya no está montado en el DOM, salir silenciosamente.
            if !container.is_connected() {
                return;
            }
            let panel = Panel::new(container.clone());
            if let Some(w) = web_sys::window() {
                let _ = panel.refresh_monitor(&w);
            }
        });
        win.set_interval_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            2000,
        )?;
        cb.forget();
        Ok(())
    }
}

// ============================================================
// Estado persistido
// ============================================================

struct State {
    variant: String,
    accent: String,
    density: String,
    lang: String,
    timefmt: String,
    mods_hidden: Vec<String>,
}

impl State {
    fn load(storage: Option<&Storage>) -> Self {
        let get = |k: &str, default: &str| -> String {
            storage
                .and_then(|s| s.get_item(k).ok().flatten())
                .unwrap_or_else(|| default.to_string())
        };
        Self {
            variant: get(STORE_VARIANT, "dark"),
            accent: get(STORE_ACCENT, "gioser"),
            density: get(STORE_DENSITY, "comfortable"),
            lang: get(STORE_LANG, "es"),
            timefmt: get(STORE_TIMEFMT, "24h"),
            mods_hidden: read_mods_hidden(storage),
        }
    }
}

fn read_mods_hidden(storage: Option<&Storage>) -> Vec<String> {
    storage
        .and_then(|s| s.get_item(STORE_MODS_HIDDEN).ok().flatten())
        .map(|csv| {
            csv.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

// ============================================================
// Aplicación de preferencias al documento
// ============================================================

fn apply_theme(doc: &Document, state: &State) -> Result<(), JsValue> {
    let root: Element = doc
        .document_element()
        .ok_or_else(|| JsValue::from_str("no documentElement"))?;
    let variant = if state.variant == "dark" { "" } else { &state.variant };
    if variant.is_empty() {
        root.remove_attribute("data-theme-variant")?;
    } else {
        root.set_attribute("data-theme-variant", variant)?;
    }
    let accent = if state.accent == "gioser" { "" } else { &state.accent };
    if accent.is_empty() {
        root.remove_attribute("data-accent")?;
    } else {
        root.set_attribute("data-accent", accent)?;
    }
    // densidad: clase en body
    if let Some(body) = doc.body() {
        let cl = body.class_list();
        let _ = cl.remove_1("density-compact");
        if state.density == "compact" {
            let _ = cl.add_1("density-compact");
        }
    }
    Ok(())
}

fn apply_modules(doc: &Document, hidden: &[String]) -> Result<(), JsValue> {
    let secs = doc.query_selector_all(".sidebar-section")?;
    for i in 0..secs.length() {
        let n = match secs.item(i) {
            Some(n) => n,
            None => continue,
        };
        let el: Element = match n.dyn_into() {
            Ok(e) => e,
            Err(_) => continue,
        };
        let key = el.get_attribute("data-mod").unwrap_or_default();
        let cl = el.class_list();
        let _ = cl.remove_1("mod-hidden");
        if !key.is_empty() && hidden.iter().any(|h| h == &key) {
            let _ = cl.add_1("mod-hidden");
        }
    }
    Ok(())
}

fn apply_after_change(doc: &Document, key: &str, value: &str) {
    if key == STORE_VARIANT || key == STORE_ACCENT || key == STORE_DENSITY {
        // re-leer state completo y aplicar
        let state = State::load(
            web_sys::window()
                .and_then(|w| w.local_storage().ok().flatten())
                .as_ref(),
        );
        let _ = apply_theme(doc, &state);
    }
    if key == STORE_LANG {
        // re-render panel content text con nuevo idioma:
        // por simplicidad, re-renderizamos toda la sección de textos
        // del panel sin perder bindings (solo cambian labels).
        relabel_panel(doc, value);
    }
    // STORE_TIMEFMT: nada que aplicar fuera del próximo tick del monitor.
}

fn relabel_panel(doc: &Document, lang: &str) {
    let nodes = match doc.query_selector_all("[data-i18n]") {
        Ok(n) => n,
        Err(_) => return,
    };
    for i in 0..nodes.length() {
        let n = match nodes.item(i) {
            Some(n) => n,
            None => continue,
        };
        let el: Element = match n.dyn_into() {
            Ok(e) => e,
            Err(_) => continue,
        };
        let key = el.get_attribute("data-i18n").unwrap_or_default();
        let text = t(&key, lang);
        el.set_text_content(Some(text));
    }
}

// ============================================================
// Helpers DOM
// ============================================================

fn mark_segmented_active(group: &Element, value: &str) -> Result<(), JsValue> {
    let segs = group.query_selector_all(".panel-seg")?;
    for i in 0..segs.length() {
        if let Some(n) = segs.item(i) {
            if let Ok(el) = n.dyn_into::<Element>() {
                let v = el.get_attribute("data-value").unwrap_or_default();
                let cl = el.class_list();
                if v == value {
                    let _ = cl.add_1("active");
                } else {
                    let _ = cl.remove_1("active");
                }
            }
        }
    }
    Ok(())
}

fn set_stat(doc: &Document, id: &str, value: &str) {
    if let Some(el) = doc.query_selector(&format!(".panel-stat[data-stat=\"{}\"] .panel-stat-value", id)).ok().flatten() {
        el.set_text_content(Some(value));
    }
}

fn set_stat_sub(doc: &Document, id: &str, value: &str) {
    if let Some(el) = doc.query_selector(&format!(".panel-stat[data-stat=\"{}\"] .panel-stat-sub", id)).ok().flatten() {
        el.set_text_content(Some(value));
    }
}

// ============================================================
// i18n del panel — es / en / qu
// ============================================================

fn t(key: &str, lang: &str) -> &'static str {
    // No traducimos nombres propios (gioser, llimphi, supay, …).
    // Cada (key, lang) → string. Si falta, fallback a es.
    macro_rules! triple {
        ($es:expr, $en:expr, $qu:expr) => {
            match lang {
                "en" => $en,
                "qu" => $qu,
                _    => $es,
            }
        };
    }
    match key {
        // nav
        "nav.appearance" => triple!("Apariencia", "Appearance", "Rikch'aynin"),
        "nav.language"   => triple!("Idioma",     "Language",   "Simi"),
        "nav.apps"       => triple!("Demos & Apps","Demos & Apps","Llamk'anakuna"),
        "nav.monitor"    => triple!("Monitor",    "Monitor",    "Qhaway"),
        "nav.modules"    => triple!("Módulos",    "Modules",    "T'aqakuna"),
        "nav.about"      => triple!("Acerca de",  "About",      "Imaynan"),

        // títulos sección
        "sec.appearance.title" => triple!("Apariencia",    "Appearance",   "Rikch'aynin"),
        "sec.appearance.hint"  => triple!("Theme, acento del cuadrante y densidad de la interfaz.",
                                          "Theme, quadrant accent and interface density.",
                                          "Llinphi, tawa kuti tinki, panchata kawapuni."),
        "sec.language.title"   => triple!("Idioma",        "Language",     "Simi"),
        "sec.language.hint"    => triple!("Idioma del panel y formato del reloj.",
                                          "Panel language and clock format.",
                                          "Panil simi, pacha rikch'ay."),
        "sec.apps.title"       => triple!("Demos & Apps",  "Demos & Apps", "Llamk'anakuna"),
        "sec.apps.hint"        => triple!("Las apps del SO. Cada card abre su documentación.",
                                          "OS apps. Each card opens its docs.",
                                          "Apukunaq llamk'ana. Sapa carda kichaq qhawayninta."),
        "sec.monitor.title"    => triple!("Monitor",       "Monitor",      "Qhaway"),
        "sec.monitor.hint"     => triple!("Estado del escritorio en vivo. Refresca cada 2 s.",
                                          "Live desktop state. Refreshes every 2 s.",
                                          "Kunan munay kawsay. 2 s sapa kuti musuq."),
        "sec.modules.title"    => triple!("Módulos",       "Modules",      "T'aqakuna"),
        "sec.modules.hint"     => triple!("Mostrar u ocultar secciones del sidebar.",
                                          "Show or hide sidebar sections.",
                                          "Rikuy mana rikuychu sidebar t'aqakunata."),
        "sec.about.title"      => triple!("Acerca de",     "About",        "Imaynan"),
        "sec.about.hint"       => triple!("Información del escritorio gioser.",
                                          "About the gioser desktop.",
                                          "gioser munay kawsayniq willay."),

        // appearance rows
        "ap.variant"  => triple!("Variante", "Variant", "Rikch'ay"),
        "ap.accent"   => triple!("Acento",   "Accent",  "Tinki"),
        "ap.density"  => triple!("Densidad", "Density", "Panchata"),
        "ap.dark"     => triple!("Dark",      "Dark",   "Llanthu"),
        "ap.light"    => triple!("Light",     "Light",  "K'anchay"),
        "ap.aurora"   => triple!("Aurora",    "Aurora", "Aurora"),
        "ap.sunset"   => triple!("Sunset",    "Sunset", "Inti haykuy"),
        "ap.gioser"   => triple!("gioser",    "gioser", "gioser"),
        "ap.compact"  => triple!("Compacto",  "Compact","Hap'isqa"),
        "ap.comfort"  => triple!("Confortable","Comfortable","Sumaq"),

        // language rows
        "lg.lang"     => triple!("Idioma",     "Language", "Simi"),
        "lg.timefmt"  => triple!("Reloj",      "Clock",    "Pacha"),
        "lg.es"       => "Español",
        "lg.en"       => "English",
        "lg.qu"       => "Runasimi",
        "lg.24h"      => "24 h",
        "lg.12h"      => "12 h",

        // monitor stats
        "mn.clock"        => triple!("Hora",       "Time",       "Pacha"),
        "mn.windows"      => triple!("Ventanas",   "Windows",    "Wisana"),
        "mn.viewport"     => triple!("Viewport",   "Viewport",   "Qhaway uku"),
        "mn.platform"     => triple!("Plataforma", "Platform",   "Pataya"),
        "mn.heap"         => triple!("Heap JS",    "JS Heap",    "JS Tantana"),
        "mn.screen"       => triple!("Pantalla",   "Screen",     "Killka"),
        "mn.browser-lang" => triple!("Idioma del navegador", "Browser language", "Maskaq simi"),

        // about
        "ab.blurb" => triple!(
            "Escritorio web de gioser — monorepo de cuatro cuadrantes: unanchay (percibir), yachay (conocer), ruway (hacer) y ukupacha (raíz).",
            "gioser web desktop — four-quadrant monorepo: unanchay (perceive), yachay (know), ruway (do), and ukupacha (root).",
            "gioser munay kawsay — tawa kuti hatun llamk'ay: unanchay (riqsiy), yachay (yachay), ruway (ruway), ukupacha (saphi)."
        ),
        "ab.version" => triple!("Versión",   "Version",  "Mit'a"),
        "ab.build"   => triple!("Build",     "Build",    "Hump'i"),
        "ab.repo"    => triple!("Repo",      "Repo",     "Q'utu"),
        "ab.license" => triple!("Licencia",  "License",  "Saqiy"),
        _ => "",
    }
}

// ============================================================
// Render del HTML
// ============================================================

fn render_html(state: &State) -> String {
    let nav = render_nav(&state.lang);
    let secs = format!(
        "{}{}{}{}{}{}",
        render_appearance(&state.lang),
        render_language(&state.lang),
        render_apps(),
        render_monitor(&state.lang),
        render_modules(&state.lang, &state.mods_hidden),
        render_about(&state.lang),
    );
    format!(
        r#"<div class="panel">
  {nav}
  <div class="panel-content">
    {secs}
  </div>
</div>"#
    )
}

fn render_nav(lang: &str) -> String {
    format!(
        r#"<nav class="panel-nav">
  <button class="panel-nav-item active" data-cat="appearance"><span class="panel-nav-glyph">◐</span><span data-i18n="nav.appearance">{ap}</span></button>
  <button class="panel-nav-item"        data-cat="language"  ><span class="panel-nav-glyph">✦</span><span data-i18n="nav.language">{lg}</span></button>
  <button class="panel-nav-item"        data-cat="apps"      ><span class="panel-nav-glyph">▣</span><span data-i18n="nav.apps">{ap2}</span></button>
  <button class="panel-nav-item"        data-cat="monitor"   ><span class="panel-nav-glyph">◉</span><span data-i18n="nav.monitor">{mn}</span></button>
  <button class="panel-nav-item"        data-cat="modules"   ><span class="panel-nav-glyph">≡</span><span data-i18n="nav.modules">{md}</span></button>
  <button class="panel-nav-item"        data-cat="about"     ><span class="panel-nav-glyph">?</span><span data-i18n="nav.about">{ab}</span></button>
</nav>"#,
        ap  = t("nav.appearance", lang),
        lg  = t("nav.language", lang),
        ap2 = t("nav.apps", lang),
        mn  = t("nav.monitor", lang),
        md  = t("nav.modules", lang),
        ab  = t("nav.about", lang),
    )
}

fn render_section_head(lang: &str, title_key: &str, hint_key: &str) -> String {
    format!(
        r#"<header class="panel-section-head">
  <h2 class="panel-section-title" data-i18n="{tk}">{title}</h2>
  <p class="panel-section-hint"   data-i18n="{hk}">{hint}</p>
</header>"#,
        tk = title_key,
        hk = hint_key,
        title = t(title_key, lang),
        hint = t(hint_key, lang),
    )
}

fn render_appearance(lang: &str) -> String {
    let head = render_section_head(lang, "sec.appearance.title", "sec.appearance.hint");
    format!(
        r#"<section class="panel-section active" data-cat="appearance">
  {head}
  <div class="panel-group">
    <div class="panel-row">
      <div class="panel-row-label" data-i18n="ap.variant">{l_var}</div>
      <div class="panel-segmented" data-group="{store_variant}">
        <button class="panel-seg" data-value="dark"   data-i18n="ap.dark">{l_dark}</button>
        <button class="panel-seg" data-value="light"  data-i18n="ap.light">{l_light}</button>
        <button class="panel-seg" data-value="aurora" data-i18n="ap.aurora">{l_aurora}</button>
        <button class="panel-seg" data-value="sunset" data-i18n="ap.sunset">{l_sunset}</button>
      </div>
    </div>
    <div class="panel-row">
      <div class="panel-row-label" data-i18n="ap.accent">{l_acc}</div>
      <div class="panel-segmented" data-group="{store_accent}">
        <button class="panel-seg" data-value="gioser"  ><span class="seg-swatch" style="background:#6E8CDC"></span>gioser</button>
        <button class="panel-seg" data-value="unanchay"><span class="seg-swatch" style="background:#B9C9E8"></span>unanchay</button>
        <button class="panel-seg" data-value="yachay"  ><span class="seg-swatch" style="background:#E8C97A"></span>yachay</button>
        <button class="panel-seg" data-value="ruway"   ><span class="seg-swatch" style="background:#E89B6E"></span>ruway</button>
        <button class="panel-seg" data-value="ukupacha"><span class="seg-swatch" style="background:#8FB58C"></span>ukupacha</button>
      </div>
    </div>
    <div class="panel-row">
      <div class="panel-row-label" data-i18n="ap.density">{l_den}</div>
      <div class="panel-segmented" data-group="{store_density}">
        <button class="panel-seg" data-value="comfortable" data-i18n="ap.comfort">{l_comf}</button>
        <button class="panel-seg" data-value="compact"     data-i18n="ap.compact">{l_comp}</button>
      </div>
    </div>
  </div>
</section>"#,
        head = head,
        store_variant = STORE_VARIANT,
        store_accent = STORE_ACCENT,
        store_density = STORE_DENSITY,
        l_var = t("ap.variant", lang),
        l_dark = t("ap.dark", lang),
        l_light = t("ap.light", lang),
        l_aurora = t("ap.aurora", lang),
        l_sunset = t("ap.sunset", lang),
        l_acc = t("ap.accent", lang),
        l_den = t("ap.density", lang),
        l_comp = t("ap.compact", lang),
        l_comf = t("ap.comfort", lang),
    )
}

fn render_language(lang: &str) -> String {
    let head = render_section_head(lang, "sec.language.title", "sec.language.hint");
    format!(
        r#"<section class="panel-section" data-cat="language">
  {head}
  <div class="panel-group">
    <div class="panel-row">
      <div class="panel-row-label" data-i18n="lg.lang">{l_lang}</div>
      <div class="panel-segmented" data-group="{store_lang}">
        <button class="panel-seg" data-value="es" data-i18n="lg.es">Español</button>
        <button class="panel-seg" data-value="en" data-i18n="lg.en">English</button>
        <button class="panel-seg" data-value="qu" data-i18n="lg.qu">Runasimi</button>
      </div>
    </div>
    <div class="panel-row">
      <div class="panel-row-label" data-i18n="lg.timefmt">{l_tf}</div>
      <div class="panel-segmented" data-group="{store_tf}">
        <button class="panel-seg" data-value="24h" data-i18n="lg.24h">24 h</button>
        <button class="panel-seg" data-value="12h" data-i18n="lg.12h">12 h</button>
      </div>
    </div>
  </div>
</section>"#,
        head = head,
        store_lang = STORE_LANG,
        store_tf = STORE_TIMEFMT,
        l_lang = t("lg.lang", lang),
        l_tf = t("lg.timefmt", lang),
    )
}

fn render_apps() -> String {
    // Catálogo de apps. (name, quad-label, theme, desc, doc-url, doc-title)
    // Los textos son cortos a propósito; cada card abre el README/SDD en una
    // ventana flotante.
    let cards: &[(&str, &str, &str, &str, &str, &str)] = &[
        // 00 unanchay
        ("pluma · editor",  "00 unanchay", "unanchay",
         "Editor de markdown y notebooks.",
         "/web/gioser-web/md/aire.md", "pluma"),
        ("khipu",           "00 unanchay", "unanchay",
         "Telar de hebras y nodos — gravedad simbólica.",
         "/web/gioser-web/md/sombra.md", "khipu"),
        ("chaka · shadow",  "00 unanchay", "unanchay",
         "Lenguaje de sombra: lexer, parser, codegen.",
         "/00_unanchay/puriy/SDD.md", "chaka"),
        ("puriy",           "00 unanchay", "unanchay",
         "Caminar entre sensores y eventos.",
         "/00_unanchay/puriy/SDD.md", "puriy"),

        // 01 yachay
        ("cosmos · app",    "01 yachay",  "yachay",
         "Observatorio astronómico: ephemeris, sky, pointing.",
         "/01_yachay/cosmos/cosmos-core/README.md", "cosmos"),
        ("nakui · explorer","01 yachay",  "yachay",
         "Explorador de cuerpos y catálogos celestes.",
         "/web/gioser-web/md/cosmos.md", "nakui"),
        ("dominium",        "01 yachay",  "yachay",
         "Modelo del dominio compartido y reglas.",
         "/01_yachay/dominium/SDD.md", "dominium"),

        // 02 ruway
        ("mirada · compositor","02 ruway", "ruway",
         "Compositor wayland — superficies y output.",
         "/02_ruway/mirada/mirada-compositor/README.md", "mirada-compositor"),
        ("mirada · portal", "02 ruway",   "ruway",
         "xdg-desktop-portal — diálogos del sistema.",
         "/02_ruway/mirada/mirada-portal/README.md", "mirada-portal"),
        ("mirada · greeter","02 ruway",   "ruway",
         "Login manager con sesiones llimphi.",
         "/02_ruway/mirada/mirada-greeter/README.md", "mirada-greeter"),
        ("llimphi",         "02 ruway",   "ruway",
         "Toolkit GUI nativo — el lienzo de las apps.",
         "/02_ruway/llimphi/SDD.md", "llimphi"),
        ("supay · doom",    "02 ruway",   "ruway",
         "Shell ritual + port de Doom como protector.",
         "/02_ruway/supay/SDD.md", "supay"),
        ("shuma",           "02 ruway",   "ruway",
         "Empaquetado de releases y canal akasha.",
         "/web/gioser-web/md/practica.md", "shuma"),
        ("nahual · viewer", "02 ruway",   "ruway",
         "Visor de imágenes con texto y shell.",
         "/web/gioser-web/md/fuego.md", "nahual"),
        ("chasqui",         "02 ruway",   "ruway",
         "Correo y mensajería — núcleo y broker.",
         "/web/gioser-web/md/practica.md", "chasqui"),
        ("nada",     "02 ruway",   "ruway",
         "Editor de texto con sesiones y temas.",
         "/02_ruway/llimphi/SDD.md", "nada"),

        // 03 ukupacha
        ("arje · seeds",    "03 ukupacha","ukupacha",
         "Kernel y semillas del sistema base.",
         "/03_ukupacha/arje/seeds/README.md", "arje-seeds"),
        ("arje · installer","03 ukupacha","ukupacha",
         "Instalador UEFI con OVMF.",
         "/03_ukupacha/arje/init/arje-installer/README.md", "arje-installer"),
        ("wawa-explorer",   "03 ukupacha","ukupacha",
         "Explorador de paquetes y release channels.",
         "/web/gioser-web/md/tierra.md", "wawa"),
        ("agora",           "03 ukupacha","ukupacha",
         "Mercado y plaza pública del sistema.",
         "/web/gioser-web/md/agua.md", "agora"),
        ("minga · p2p",     "03 ukupacha","ukupacha",
         "Red p2p, dht y vfs distribuida.",
         "/03_ukupacha/arje/seeds/README.md", "minga"),
    ];

    let mut body = String::new();
    for (name, quad, theme, desc, url, title) in cards {
        body.push_str(&format!(
            r#"<div class="panel-card" data-theme="{theme}" data-doc="{url}" data-title="{title}">
  <div class="panel-card-head">
    <span class="panel-card-name">{name}</span>
    <span class="panel-card-quad">{quad}</span>
  </div>
  <div class="panel-card-desc">{desc}</div>
</div>"#
        ));
    }

    let head_es = "Demos & Apps";
    let hint_es = "Las apps del SO. Cada card abre su documentación.";
    format!(
        r#"<section class="panel-section" data-cat="apps">
  <header class="panel-section-head">
    <h2 class="panel-section-title" data-i18n="sec.apps.title">{head_es}</h2>
    <p  class="panel-section-hint"  data-i18n="sec.apps.hint">{hint_es}</p>
  </header>
  <div class="panel-card-grid">
    {body}
  </div>
</section>"#
    )
}

fn render_monitor(lang: &str) -> String {
    let head = render_section_head(lang, "sec.monitor.title", "sec.monitor.hint");
    let stat = |id: &str, key: &str| -> String {
        format!(
            r#"<div class="panel-stat" data-stat="{id}">
  <div class="panel-stat-label" data-i18n="{key}">{label}</div>
  <div class="panel-stat-value">—</div>
  <div class="panel-stat-sub"></div>
</div>"#,
            id = id,
            key = key,
            label = t(key, lang),
        )
    };
    format!(
        r#"<section class="panel-section" data-cat="monitor">
  {head}
  <div class="panel-stats">
    {clock}{windows}{viewport}{platform}{heap}{screen}{lang}
  </div>
</section>"#,
        head = head,
        clock = stat("clock", "mn.clock"),
        windows = stat("windows", "mn.windows"),
        viewport = stat("viewport", "mn.viewport"),
        platform = stat("platform", "mn.platform"),
        heap = stat("heap", "mn.heap"),
        screen = stat("screen", "mn.screen"),
        lang = stat("browser-lang", "mn.browser-lang"),
    )
}

fn render_modules(lang: &str, hidden: &[String]) -> String {
    let head = render_section_head(lang, "sec.modules.title", "sec.modules.hint");
    // Las claves vienen del data-mod que vamos a estampar en cada
    // .sidebar-section del index.html.
    let mods: &[(&str, &str, &str)] = &[
        ("sistema",  "▤", "sistema"),
        ("unanchay", "◑", "00 unanchay · percibir"),
        ("yachay",   "★", "01 yachay · conocer"),
        ("ruway",    "✦", "02 ruway · hacer"),
        ("ukupacha", "◉", "03 ukupacha · raíz"),
        ("shared",   "◈", "shared"),
    ];
    let mut rows = String::new();
    for (k, g, name) in mods {
        let checked = if hidden.iter().any(|h| h == k) { "" } else { " checked" };
        rows.push_str(&format!(
            r#"<div class="panel-mod-row">
  <span class="panel-mod-name"><span class="mod-glyph">{g}</span>{name}</span>
  <input type="checkbox" class="panel-toggle" data-mod="{k}"{checked}>
</div>"#
        ));
    }
    format!(
        r#"<section class="panel-section" data-cat="modules">
  {head}
  <div class="panel-mod-list">
    {rows}
  </div>
</section>"#
    )
}

fn render_about(lang: &str) -> String {
    let head = render_section_head(lang, "sec.about.title", "sec.about.hint");
    let blurb = t("ab.blurb", lang);
    let v = env!("CARGO_PKG_VERSION");
    format!(
        r#"<section class="panel-section" data-cat="about">
  {head}
  <div class="panel-about">
    <div class="panel-about-blurb" data-i18n="ab.blurb">{blurb}</div>
    <dl class="panel-about-kv">
      <dt data-i18n="ab.version">{lab_v}</dt><dd>{v}</dd>
      <dt data-i18n="ab.build">{lab_b}</dt><dd>{prof}</dd>
      <dt data-i18n="ab.repo">{lab_r}</dt><dd><a href="https://gitea.gioser.net/sergio/gioser" target="_blank" rel="noopener">gitea.gioser.net/sergio/gioser</a></dd>
      <dt data-i18n="ab.license">{lab_l}</dt><dd>0BSD · MIT · Apache-2.0</dd>
    </dl>
  </div>
</section>"#,
        head = head,
        blurb = blurb,
        v = v,
        prof = if cfg!(debug_assertions) { "dev" } else { "release" },
        lab_v = t("ab.version", lang),
        lab_b = t("ab.build", lang),
        lab_r = t("ab.repo", lang),
        lab_l = t("ab.license", lang),
    )
}

/// Aplica preferencias persistidas al documento sin montar la UI.
/// Pensado para llamarse al cargar la página, así el escritorio
/// arranca ya con la variante/acento/densidad/módulos del usuario.
pub fn aplicar_preferencias_iniciales() -> Result<(), JsValue> {
    let win = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let doc = win.document().ok_or_else(|| JsValue::from_str("no doc"))?;
    let storage = win.local_storage().ok().flatten();
    let state = State::load(storage.as_ref());
    apply_theme(&doc, &state)?;
    apply_modules(&doc, &state.mods_hidden)?;
    Ok(())
}
